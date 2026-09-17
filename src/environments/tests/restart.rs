use super::*;
use std::{
    collections::{BTreeMap, BTreeSet},
    time::Instant,
};

#[test]
fn restart_readiness_deadline_reports_failure_and_preserves_data() {
    let (_directory, config) = fixture();
    let workspace = config.workspaces.join("review");
    fs::create_dir(&workspace).unwrap();
    fs::write(workspace.join("data"), "keep").unwrap();
    let result = super::super::restart::wait_ready(
        &config,
        "review",
        &BTreeSet::from(["web-id".into()]),
        &BTreeMap::new(),
        Instant::now(),
        Arc::new(|_| {}),
    );
    assert_eq!(
        result.unwrap_err(),
        "restart readiness timed out: Waiting for restarted containers; resources preserved for inspection"
    );
    assert_eq!(fs::read_to_string(workspace.join("data")).unwrap(), "keep");
}
