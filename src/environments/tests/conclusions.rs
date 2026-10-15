use super::*;
use crate::environments::config::Config;

#[test]
fn manual_reports_survive_name_reuse_restart_and_interrupted_cleanup_with_namespace_isolation() {
    let home = tempfile::tempdir().unwrap();
    let config = Config::at(home.path().into(), "reports".into(), 9876).unwrap();
    let store = RuleStore::open(&config).unwrap();
    let report = ReportInput {
        title: "Investigated timeout".into(),
        summary: "Verified retry".into(),
        markdown: "# Evidence\nA timeout was reproduced.".into(),
        ..Default::default()
    };
    let first = store
        .save_conclusion("manual", "first-workspace", None, Some(&report))
        .unwrap();
    store
        .set_conclusion_cleanup(first.conclusion_id, CleanupState::Purged, None)
        .unwrap();
    let second = store
        .save_conclusion("manual", "second-workspace", None, Some(&report))
        .unwrap();
    assert_ne!(first.conclusion_id, second.conclusion_id);
    let restarted = RuleStore::open(&config).unwrap();
    assert_eq!(
        restarted.conclusion(first.conclusion_id).unwrap().report,
        Some(report)
    );
    assert_eq!(
        restarted
            .conclusion(first.conclusion_id)
            .unwrap()
            .details
            .cleanup_state,
        CleanupState::Purged
    );
    let interrupted = restarted.conclusion(second.conclusion_id).unwrap();
    assert_eq!(interrupted.details.cleanup_state, CleanupState::Failed);
    assert!(
        interrupted
            .details
            .cleanup_error
            .unwrap()
            .contains("interrupted")
    );
    let matches = restarted
        .search_conclusions(vec!["TIMEOUT".into()])
        .unwrap();
    assert_eq!(
        matches
            .iter()
            .map(|record| record.conclusion_id)
            .collect::<Vec<_>>(),
        [second.conclusion_id, first.conclusion_id]
    );
    let other =
        RuleStore::open(&Config::at(home.path().into(), "other".into(), 9876).unwrap()).unwrap();
    assert!(matches!(
        other.conclusion(first.conclusion_id),
        Err(Error::NotFound)
    ));
    assert!(
        other
            .search_conclusions(vec!["TIMEOUT".into()])
            .unwrap()
            .is_empty()
    );
}
