use super::*;
use crate::environments::Startup;
use std::sync::atomic::{AtomicUsize, Ordering};

fn start(config: &Config, calls: &Arc<AtomicUsize>, fail: bool) -> Result<Instance, String> {
    let calls = Arc::clone(calls);
    let expected = config.workspaces.join("review");
    lifecycle::start(
        config,
        "local",
        "review",
        Startup {
            before_creation: Some(Box::new(move |workspace, _| {
                assert_eq!(std::path::Path::new(workspace), expected);
                assert!(expected.is_dir());
                calls.fetch_add(1, Ordering::Relaxed);
                if fail {
                    Err("cleanup unavailable".into())
                } else {
                    Ok(())
                }
            })),
            ..Default::default()
        },
        30,
        Arc::new(|_| {}),
        |_| {},
    )
}

#[test]
fn creation_cleanup_runs_once_per_instance_identity_and_before_readiness() {
    let (_root, config) = fixture();
    templates::create(&config, "local").unwrap();
    let calls = Arc::new(AtomicUsize::new(0));
    let created = start(&config, &calls, false).unwrap();
    assert!(created.runtime.workspace_ready);
    let existing = start(&config, &calls, false).unwrap();
    assert!(existing.runtime.workspace_ready);
    assert_eq!(calls.load(Ordering::Relaxed), 1);
    lifecycle::delete(&config, "review", Arc::new(|_| {}), &|_, _| Ok(())).unwrap();
    let recreated = start(&config, &calls, false).unwrap();
    assert!(recreated.runtime.workspace_ready);
    assert_eq!(calls.load(Ordering::Relaxed), 2);
}

#[test]
fn failed_cleanup_blocks_creation_and_must_succeed_before_retry_proceeds() {
    let (_root, config) = fixture();
    templates::create(&config, "local").unwrap();
    let calls = Arc::new(AtomicUsize::new(0));
    assert_eq!(
        start(&config, &calls, true).unwrap_err(),
        "cleanup unavailable"
    );
    assert!(journal::recorded(&config, "review").unwrap().is_none());
    assert!(!config.workspaces.join("review/AGENTS.md").exists());
    let retried = start(&config, &calls, false).unwrap();
    assert!(retried.runtime.workspace_ready);
    assert_eq!(calls.load(Ordering::Relaxed), 2);
}

#[test]
fn provisioning_retries_preserve_conversations_after_cleanup() {
    let (_root, config) = fixture();
    templates::create(&config, "local").unwrap();
    templates::update_manifest(
        &config,
        "local",
        serde_json::from_value(json!({
            "repositories": [{"source": config.home.join("missing-repository"), "target": "app"}]
        }))
        .unwrap(),
    )
    .unwrap();
    let calls = Arc::new(AtomicUsize::new(0));
    assert!(start(&config, &calls, false).is_err());
    assert!(journal::recorded(&config, "review").unwrap().is_some());
    templates::update_manifest(&config, "local", Default::default()).unwrap();
    let retried = start(&config, &calls, true).unwrap();
    assert!(retried.runtime.workspace_ready);
    assert_eq!(calls.load(Ordering::Relaxed), 1);
}

#[test]
fn symlinked_workspaces_never_reach_creation_cleanup() {
    let (_root, config) = fixture();
    templates::create(&config, "local").unwrap();
    let outside = tempfile::tempdir().unwrap();
    std::os::unix::fs::symlink(outside.path(), config.workspaces.join("review")).unwrap();
    let calls = Arc::new(AtomicUsize::new(0));
    assert!(
        start(&config, &calls, false)
            .unwrap_err()
            .contains("real directory")
    );
    assert_eq!(calls.load(Ordering::Relaxed), 0);
}
