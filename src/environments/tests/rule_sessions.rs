use super::*;
use crate::environments::config::Config;

#[test]
fn bindings_scope_exact_names_to_namespace_instance_and_workspace_incarnation() {
    let home = tempfile::tempdir().unwrap();
    let config = Config::at(home.path().into(), "first".into(), 9876).unwrap();
    let store = RuleStore::open(&config).unwrap();
    let binding = store
        .reserve_named_session("Shared", "triage", "workspace-one", "http://127.0.0.1:4199")
        .unwrap();
    assert!(!binding.ready);
    store.finish_named_session(&binding, true).unwrap();
    let saved = store
        .named_session("SHARED", "triage", "workspace-one")
        .unwrap()
        .unwrap();
    assert!(saved.ready);
    assert_eq!(saved.session_id, binding.session_id);
    for (instance, name, identity) in [
        ("other", "triage", "workspace-one"),
        ("shared", "Triage", "workspace-one"),
        ("shared", "triage", "workspace-two"),
    ] {
        assert!(
            store
                .named_session(instance, name, identity)
                .unwrap()
                .is_none()
        );
    }
    let mut other = config;
    other.namespace = "second".into();
    assert!(
        RuleStore::open(&other)
            .unwrap()
            .named_session("shared", "triage", "workspace-one")
            .unwrap()
            .is_none()
    );
    let replacement = store
        .reserve_named_session("shared", "triage", "workspace-two", &binding.server)
        .unwrap();
    assert_ne!(replacement.session_id, binding.session_id);
    store.finish_named_session(&binding, false).unwrap();
    assert_eq!(
        store
            .named_session("shared", "triage", "workspace-two")
            .unwrap()
            .unwrap()
            .session_id,
        replacement.session_id
    );
}
