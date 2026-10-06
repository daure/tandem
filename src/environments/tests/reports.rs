use super::*;
use crate::{
    environments::{config::Config, events::EventStore},
    store::{
        events::{Batch, Deletion},
        rules::Definition,
    },
};

fn fixture() -> (
    tempfile::TempDir,
    Config,
    RuleStore,
    EventStore,
    Vec<Acceptance>,
) {
    let home = tempfile::tempdir().unwrap();
    let config = Config::at(home.path().into(), "reports".into(), 9876).unwrap();
    let rules = RuleStore::open(&config).unwrap();
    let events = EventStore::open(&config).unwrap();
    for name in ["inspect", "triage"] {
        rules
            .save(
                Definition {
                    name: name.into(),
                    description: String::new(),
                    script: "fn matches(event) { true }".into(),
                    template: "blank".into(),
                    model: "openai/test".into(),
                    variant: None,
                    initial_prompt: "Inspect {{event.summary}}".into(),
                    enabled: true,
                    start_instance: false,
                    focus_pane: true,
                },
                None,
                "main".into(),
            )
            .unwrap();
    }
    let token = events.register_provider("sample").unwrap();
    events
        .ingest(
            &token,
            Batch {
                events: vec![crate::environments::events::tests::event("one")],
            },
        )
        .unwrap();
    rules.evaluate().unwrap();
    let acceptances = rules.snapshot().unwrap().acceptances;
    (home, config, rules, events, acceptances)
}

fn input(title: &str) -> ReportInput {
    ReportInput {
        title: title.into(),
        summary: "Fixed deployment".into(),
        markdown: "# Evidence\nÜberprüfung found 100%_literal and a retry issue.\n".into(),
    }
}

#[test]
fn acceptance_lookup_allows_unlinked_instances_and_rejects_mismatched_or_ambiguous_links() {
    let (_home, _config, rules, _events, acceptances) = fixture();
    let mut instance = Instance {
        name: "manual".into(),
        template: "blank".into(),
        ..Default::default()
    };
    assert_eq!(rules.instance_acceptance(&instance).unwrap(), None);
    instance.name = acceptances[0].instance.clone();
    instance.template = "other".into();
    assert!(matches!(
        rules.instance_acceptance(&instance),
        Err(Error::Conflict(_))
    ));
    instance.template = "blank".into();
    rules
        .events
        .connection()
        .unwrap()
        .execute(
            "UPDATE rule_acceptances SET payload = json_set(payload, '$.instance', ?2) WHERE id = ?1",
            rusqlite::params![acceptances[1].id, instance.name],
        )
        .unwrap();
    assert!(matches!(
        rules.instance_acceptance(&instance),
        Err(Error::Conflict(_))
    ));
}

#[test]
fn sibling_acceptance_reports_are_durable_searchable_and_namespace_scoped() {
    let (_home, config, rules, events, acceptances) = fixture();
    let first = rules
        .save_report(acceptances[0].id, &input("Database analysis"))
        .unwrap();
    rules
        .set_report_cleanup(first.details.acceptance_id, CleanupState::Purged, None)
        .unwrap();
    rules
        .save_report(acceptances[1].id, &input("Frontend inspection"))
        .unwrap();
    rules
        .set_report_cleanup(
            acceptances[1].id,
            CleanupState::Failed,
            Some("client closure failed"),
        )
        .unwrap();
    let restarted = RuleStore::open(&config).unwrap();
    for strings in [
        vec!["DEPLOYMENT"],
        vec!["überPRÜFUNG"],
        vec!["100%_literal"],
        vec!["unknown", "evidence"],
    ] {
        let results = restarted
            .search_reports(strings.into_iter().map(str::to_owned).collect())
            .unwrap();
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].acceptance_id, acceptances[0].id);
        assert_eq!(results[0].event_sequence, acceptances[0].event_sequence);
        assert_eq!(results[0].title, "Database analysis");
        assert_eq!(results[0].summary, "Fixed deployment");
        assert_eq!(
            results[1].cleanup_error.as_deref(),
            Some("client closure failed")
        );
    }
    assert_eq!(
        restarted
            .search_reports(vec!["DATABASE".into()])
            .unwrap()
            .len(),
        1
    );
    assert!(
        restarted
            .search_reports(vec!["does not exist".into()])
            .unwrap()
            .is_empty()
    );
    let report = restarted.event_report(acceptances[0].id).unwrap();
    assert_eq!(report.markdown, input("").markdown);
    assert_eq!(report.details.cleanup_state, CleanupState::Purged);
    let projected = restarted.snapshot().unwrap();
    assert_eq!(projected.reports[&acceptances[0].id], report.details);
    assert_eq!(
        projected.reports[&acceptances[1].id]
            .cleanup_error
            .as_deref(),
        Some("client closure failed")
    );
    let other = Config::at(config.home.clone(), "other".into(), 9876).unwrap();
    let foreign = RuleStore::open(&other).unwrap();
    assert!(
        foreign
            .search_reports(vec!["evidence".into()])
            .unwrap()
            .is_empty()
    );
    assert!(matches!(
        foreign.event_report(acceptances[0].id),
        Err(Error::NotFound)
    ));
    assert!(matches!(
        foreign.save_report(acceptances[0].id, &input("Foreign")),
        Err(Error::NotFound)
    ));
    assert!(matches!(
        foreign.set_report_cleanup(acceptances[0].id, CleanupState::Failed, None),
        Err(Error::NotFound)
    ));
    events
        .delete(Deletion::Acceptance(acceptances[1].id))
        .unwrap();
    assert_eq!(
        restarted
            .search_reports(vec!["evidence".into()])
            .unwrap()
            .len(),
        1
    );
    events
        .delete(Deletion::Event(acceptances[0].event_sequence))
        .unwrap();
    assert!(
        restarted
            .search_reports(vec!["evidence".into()])
            .unwrap()
            .is_empty()
    );
}

#[test]
fn report_retries_preserve_content_and_expose_interrupted_cleanup() {
    let (_home, _config, rules, _events, acceptances) = fixture();
    let id = acceptances[0].id;
    let original = rules.save_report(id, &input("Investigation")).unwrap();
    let interrupted = rules.event_report(id).unwrap();
    assert_eq!(interrupted.details.cleanup_state, CleanupState::Failed);
    assert!(
        interrupted
            .details
            .cleanup_error
            .unwrap()
            .contains("interrupted")
    );
    let retry = rules.save_report(id, &input("Investigation")).unwrap();
    assert_eq!(retry.details.reported_at, original.details.reported_at);
    assert!(
        rules
            .save_report(id, &input("Replacement"))
            .unwrap_err()
            .to_string()
            .contains("immutable")
    );
    rules
        .set_report_cleanup(id, CleanupState::Purged, None)
        .unwrap();
    assert!(rules.save_report(id, &input("Investigation")).is_err());
    for strings in [
        vec![],
        vec![" ".into()],
        vec!["x".repeat(501)],
        vec!["x".into(); 21],
        vec!["nul\0".into()],
    ] {
        assert!(rules.search_reports(strings).is_err());
    }
    for invalid in [
        ReportInput {
            title: " ".into(),
            ..input("valid")
        },
        ReportInput {
            summary: " ".into(),
            ..input("valid")
        },
        ReportInput {
            markdown: String::new(),
            ..input("valid")
        },
        ReportInput {
            title: "multiline\ntitle".into(),
            ..input("valid")
        },
        ReportInput {
            markdown: "x".repeat(1_048_577),
            ..input("valid")
        },
    ] {
        assert!(rules.save_report(acceptances[1].id, &invalid).is_err());
    }
    assert!(matches!(
        rules.event_report(acceptances[1].id),
        Err(Error::NotFound)
    ));
}
