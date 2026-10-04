use super::*;
use crate::{
    environments::{events::EventStore, rules::RuleStore},
    store::events::Batch,
};

fn definition() -> Definition {
    Definition {
        name: "inspect".into(),
        description: "Inspect messages".into(),
        script: "fn matches(event) { true }".into(),
        template: "blank".into(),
        model: "openai/test".into(),
        variant: None,
        initial_prompt: "Inspect {{event.data.text}}".into(),
        enabled: true,
        start_instance: false,
    }
}

fn put(config: &Config, definition: &Definition) -> PathBuf {
    let directory = config.home.join("templates/rules").join(&definition.name);
    fs::create_dir_all(&directory).unwrap();
    let path = directory.join("rule.json");
    let mut value = serde_json::to_value(definition).unwrap();
    value.as_object_mut().unwrap().remove("enabled");
    fs::write(&path, serde_json::to_string_pretty(&value).unwrap()).unwrap();
    path
}

fn ingest(events: &EventStore, token: &str, id: &str) -> i64 {
    events
        .ingest(
            token,
            Batch {
                events: vec![crate::environments::events::tests::event(id)],
            },
        )
        .unwrap()
        .receipts[0]
        .sequence
}

#[test]
fn saved_recipes_are_portable_and_activation_stays_local() {
    let home = tempfile::tempdir().unwrap();
    let first = Config::at(home.path().into(), "first".into(), 9876).unwrap();
    let store = RuleStore::open(&first).unwrap();
    let saved = store.save(definition(), None, "main".into()).unwrap();
    let path = first.home.join("templates/rules/inspect/rule.json");
    let value: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
    assert_eq!(value["name"], "inspect");
    assert_eq!(value["start_instance"], false);
    assert!(value.get("enabled").is_none());
    assert!(value.get("zellij_session").is_none());
    assert!(value.get("revision").is_none());
    assert_eq!(
        RuleStore::open(&first).unwrap().snapshot().unwrap().rules,
        vec![saved.clone()]
    );

    let second = Config::at(home.path().into(), "second".into(), 9876).unwrap();
    let sibling = RuleStore::open(&second).unwrap();
    let discovered = sibling.snapshot().unwrap().rules.remove(0);
    assert_eq!(discovered.definition, recipe(&saved.definition));
    assert!(discovered.zellij_session.is_empty());
    assert_eq!(store.snapshot().unwrap().rules, vec![saved.clone()]);

    let copied_home = tempfile::tempdir().unwrap();
    let copied = Config::at(copied_home.path().into(), "first".into(), 9876).unwrap();
    put(&copied, &saved.definition);
    fs::write(
        copied.home.join("templates/rules/README.md"),
        "Shared recipes",
    )
    .unwrap();
    let discovered = RuleStore::open(&copied)
        .unwrap()
        .snapshot()
        .unwrap()
        .rules
        .remove(0);
    assert!(!discovered.definition.enabled);
    assert_eq!(discovered.revision, 1);
    assert!(discovered.zellij_session.is_empty());
}

#[test]
fn external_edits_pause_future_attempts_and_keep_pinned_work() {
    let home = tempfile::tempdir().unwrap();
    let config = Config::at(home.path().into(), "edited".into(), 9876).unwrap();
    let store = RuleStore::open(&config).unwrap();
    let saved = store.save(definition(), None, "main".into()).unwrap();
    let events = EventStore::open(&config).unwrap();
    let token = events.register_provider("sample").unwrap();
    let sequence = ingest(&events, &token, "pinned");
    let mut edited = saved.definition.clone();
    edited.initial_prompt = "An edited prompt".into();
    put(&config, &edited);
    assert!(matches!(
        store.save(
            saved.definition.clone(),
            Some(saved.revision),
            "main".into()
        ),
        Err(Error::Conflict(_))
    ));
    ingest(&events, &token, "paused");
    store.evaluate().unwrap();
    let snapshot = store.snapshot().unwrap();
    assert_eq!(snapshot.acceptances.len(), 1);
    assert_eq!(snapshot.acceptances[0].rule, saved);
    let current = snapshot.rules[0].clone();
    assert_eq!(current.revision, 2);
    assert!(!current.definition.enabled);
    assert!(current.zellij_session.is_empty());
    assert_eq!(current.definition.initial_prompt, "An edited prompt");

    edited.enabled = true;
    let activated = store
        .save(edited, Some(current.revision), "today".into())
        .unwrap();
    events.replay(sequence, "edited-replay").unwrap();
    store.evaluate().unwrap();
    let snapshot = store.snapshot().unwrap();
    assert_eq!(snapshot.acceptances.len(), 2);
    assert_eq!(snapshot.acceptances[0].rule, activated);
    assert_eq!(
        snapshot.acceptances[0].resolved_prompt.as_deref(),
        Some("An edited prompt")
    );

    let path = config.home.join("templates/rules/inspect/rule.json");
    fs::remove_file(&path).unwrap();
    ingest(&events, &token, "removed");
    assert!(store.snapshot().unwrap().rules.is_empty());
    assert_eq!(store.snapshot().unwrap().acceptances.len(), 2);
    put(&config, &activated.definition);
    let restored = store.snapshot().unwrap().rules.remove(0);
    assert!(!restored.definition.enabled);
    assert!(restored.revision > activated.revision);
    assert!(restored.zellij_session.is_empty());
    fs::remove_file(&path).unwrap();
    assert!(store.snapshot().unwrap().rules.is_empty());
    let recreated = store
        .save(recipe(&restored.definition), None, String::new())
        .unwrap();
    assert!(recreated.revision > restored.revision);
    assert!(!recreated.definition.enabled);
    assert_eq!(store.snapshot().unwrap().acceptances.len(), 2);
}

#[test]
fn invalid_catalog_files_block_attempts_without_overwriting_files_or_history() {
    let home = tempfile::tempdir().unwrap();
    let config = Config::at(home.path().into(), "invalid".into(), 9876).unwrap();
    let store = RuleStore::open(&config).unwrap();
    let saved = store.save(definition(), None, "main".into()).unwrap();
    let events = EventStore::open(&config).unwrap();
    let token = events.register_provider("sample").unwrap();
    let sequence = ingest(&events, &token, "retained");
    let path = config.home.join("templates/rules/inspect/rule.json");
    for contents in [
        "{",
        &serde_json::to_string(&saved.definition).unwrap(),
        &serde_json::to_string(&serde_json::json!({
            "name": "wrong-name", "script": "fn matches(event) { true }",
            "template": "blank", "model": "openai/test", "initial_prompt": "Inspect"
        }))
        .unwrap(),
    ] {
        fs::write(&path, contents).unwrap();
        assert!(store.snapshot().is_err());
        assert!(events.replay(sequence, "invalid").is_err());
        assert!(
            events
                .ingest(
                    &token,
                    Batch {
                        events: vec![crate::environments::events::tests::event("invalid")],
                    }
                )
                .is_err()
        );
        assert!(
            store
                .save(
                    saved.definition.clone(),
                    Some(saved.revision),
                    "main".into()
                )
                .is_err()
        );
        assert_eq!(fs::read_to_string(&path).unwrap(), contents);
        assert_eq!(events.snapshot().unwrap().total, 1);
    }
    put(&config, &saved.definition);
    store.evaluate().unwrap();
    assert_eq!(store.snapshot().unwrap().acceptances.len(), 1);
}

#[cfg(unix)]
#[test]
fn rule_files_and_directories_must_be_real_catalog_paths() {
    use std::os::unix::fs::symlink;

    let home = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    let config = Config::at(home.path().into(), "paths".into(), 9876).unwrap();
    let store = RuleStore::open(&config).unwrap();
    let path = put(&config, &definition());
    let original = fs::read_to_string(&path).unwrap();
    let external = outside.path().join("rule.json");
    fs::write(&external, &original).unwrap();
    fs::remove_file(&path).unwrap();
    symlink(&external, &path).unwrap();
    assert!(store.snapshot().is_err());
    assert!(store.save(definition(), None, "main".into()).is_err());
    assert_eq!(fs::read_to_string(&external).unwrap(), original);
    fs::remove_file(&path).unwrap();
    fs::remove_dir(path.parent().unwrap()).unwrap();
    symlink(outside.path(), path.parent().unwrap()).unwrap();
    assert!(store.snapshot().is_err());
    assert!(store.save(definition(), None, "main".into()).is_err());
    assert_eq!(fs::read_to_string(&external).unwrap(), original);
}
