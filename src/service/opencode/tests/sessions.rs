use crate::{
    environments::opencode::tests::history_server::Server,
    service::AppService,
    store::opencode::{Activity, Session, Snapshot},
};

fn folder_service() -> AppService {
    let mut service = AppService::for_tests();
    let config = service.config_for_tests();
    crate::environments::runtime_db::prepare(&config).unwrap();
    let observer = &mut std::sync::Arc::get_mut(&mut service.opencode)
        .unwrap()
        .observer;
    observer.daemons = config.home.join("daemons");
    observer.presence = config.home.join("presence");
    service
}

#[test]
fn session_mutations_publish_verified_titles_and_hide_deleted_history() {
    let service = AppService::for_tests();
    let server = Server::start();
    server.data.lock().unwrap().v2 = true;
    server.session("ses_old", "/work/review", None);
    let observed = Snapshot {
        sessions: vec![Session {
            id: "ses_old".into(),
            title: "Fixture".into(),
            directory: "/work/review".into(),
            server: server.url.clone(),
            activity: Activity::Busy,
            ..Default::default()
        }],
        ..Default::default()
    };
    service.set_opencode_snapshot_for_tests(observed.clone());
    service
        .runtime
        .block_on(
            service
                .rename_opencode_session("ses_old", " Updated title ".into())
                .unwrap(),
        )
        .unwrap()
        .unwrap();
    assert_eq!(
        service.opencode_snapshot().sessions[0].title,
        "Updated title"
    );
    server.data.lock().unwrap().busy = true;
    service
        .runtime
        .block_on(service.delete_opencode_session("ses_old").unwrap())
        .unwrap()
        .unwrap();
    assert!(service.opencode_snapshot().sessions.is_empty());
    service.set_opencode_snapshot_for_tests(observed);
    assert!(service.known_opencode_session("ses_old").is_none());
}

#[test]
fn external_folder_cleanup_filters_late_observations_across_observer_resets_and_allows_reopening() {
    let root = tempfile::tempdir().unwrap();
    let mut service = folder_service();
    let observer = &mut std::sync::Arc::get_mut(&mut service.opencode)
        .unwrap()
        .observer;
    observer.daemons = root.path().join("daemons");
    observer.presence = root.path().join("presence");
    let server = Server::start();
    server.data.lock().unwrap().v2 = true;
    let directory = root.path().join("external").display().to_string();
    std::fs::create_dir(&directory).unwrap();
    server.session("ses_old", &directory, None);
    let observed = Snapshot {
        directories: vec![directory.clone()],
        sessions: vec![Session {
            id: "ses_old".into(),
            directory: directory.clone(),
            server: server.url.clone(),
            activity: Activity::Idle,
            ..Default::default()
        }],
        ..Default::default()
    };
    service.set_opencode_snapshot_for_tests(observed.clone());
    let data = server.data.lock().unwrap();
    let reply = service.clear_opencode_folder(&directory).unwrap();
    assert!(
        service
            .opencode_snapshot()
            .workspace_directories()
            .next()
            .is_none()
    );
    super::super::observation::publish(
        &std::sync::Arc::downgrade(&service.opencode),
        0,
        Ok(observed.clone()),
    );
    assert!(
        service
            .opencode_snapshot()
            .workspace_directories()
            .next()
            .is_none()
    );
    assert!(service.opencode.state.lock().unwrap().navigation.is_none());
    service.opencode.reset();
    service.set_opencode_snapshot_for_tests(observed.clone());
    assert!(service.opencode_snapshot().directories.is_empty());
    let generation = service.opencode.state.lock().unwrap().generation;
    drop(data);
    service.runtime.block_on(reply).unwrap().unwrap();
    assert!(
        service
            .opencode_snapshot()
            .workspace_directories()
            .next()
            .is_none()
    );
    super::super::observation::publish(
        &std::sync::Arc::downgrade(&service.opencode),
        generation,
        Ok(observed.clone()),
    );
    assert!(
        service
            .opencode_snapshot()
            .workspace_directories()
            .next()
            .is_none()
    );
    let mut reopened = observed;
    reopened.sessions[0].id = "ses_new".into();
    reopened.sessions[0]
        .panes
        .push(crate::store::opencode::Pane {
            session: "main".into(),
            id: 7,
            tab_id: 1,
            tab_name: "Review".into(),
        });
    super::super::observation::publish(
        &std::sync::Arc::downgrade(&service.opencode),
        generation,
        Ok(reopened),
    );
    assert_eq!(service.opencode_snapshot().directories, [directory]);
}

#[test]
fn session_mutations_validate_titles_and_verify_uncertain_sessions_on_the_server() {
    let service = AppService::for_tests();
    for title in ["", "  ", "Title\nline", "\u{1b}[1m", &"x".repeat(1025)] {
        assert!(
            service
                .rename_opencode_session("ses_old", title.into())
                .unwrap_err()
                .contains("nonempty title")
        );
    }
    for activity in [Activity::Busy, Activity::AwaitingAnswer, Activity::Unknown] {
        service.set_opencode_snapshot_for_tests(Snapshot {
            sessions: vec![Session {
                id: "ses_old".into(),
                activity,
                ..Default::default()
            }],
            ..Default::default()
        });
        let reply = service.delete_opencode_session("ses_old").unwrap();
        assert!(service.runtime.block_on(reply).unwrap().is_err());
    }
}

#[test]
fn failed_folder_cleanup_restores_observed_history_and_allows_retry() {
    let service = folder_service();
    let server = Server::start();
    server.session("ses_old", "/work/review", None);
    server.data.lock().unwrap().delete_failure = true;
    let observed = Snapshot {
        directories: vec!["/work/review".into()],
        sessions: vec![Session {
            id: "ses_old".into(),
            directory: "/work/review".into(),
            server: server.url.clone(),
            activity: Activity::Idle,
            ..Default::default()
        }],
        ..Default::default()
    };
    service.set_opencode_snapshot_for_tests(observed);
    let reply = service.clear_opencode_folder("/work/review").unwrap();
    assert!(
        service
            .runtime
            .block_on(reply)
            .unwrap()
            .unwrap_err()
            .contains("500")
    );
    assert_eq!(service.opencode_snapshot().sessions[0].id, "ses_old");
    server.data.lock().unwrap().delete_failure = false;
    let reply = service.clear_opencode_folder("/work/review").unwrap();
    service.runtime.block_on(reply).unwrap().unwrap();
    assert!(
        service
            .opencode_snapshot()
            .workspace_directories()
            .next()
            .is_none()
    );
}

#[test]
fn folder_cleanup_completes_after_the_initiating_process_exits() {
    use std::{
        process::{Command, Stdio},
        time::{Duration, Instant},
    };

    let service = folder_service();
    let server = Server::start();
    let config = service.config_for_tests();
    let directory = config.home.join("external").display().to_string();
    std::fs::create_dir(&directory).unwrap();
    let station = config.home.join("daemons/station");
    std::fs::create_dir_all(station.join("dirs")).unwrap();
    std::fs::write(station.join("port"), server.url.rsplit(':').next().unwrap()).unwrap();
    let receipt = station.join("dirs/external.dir");
    std::fs::write(&receipt, &directory).unwrap();
    server.session("ses_old", &directory, None);
    let data = server.data.lock().unwrap();
    let status = Command::new("/proc/self/exe")
        .args([
            "--exact",
            "service::opencode::sessions::tests::launcher_entry",
            "--ignored",
        ])
        .env("TANDEM_HOME", &config.home)
        .env("TANDEM_NAMESPACE", &config.namespace)
        .env("TANDEM_GATEWAY_PORT", config.port.to_string())
        .env("TANDEM_INSTRUCTIONS_FILE", &config.instructions)
        .env("TANDEM_TEST_CLEANUP_DIRECTORY", &directory)
        .env("TANDEM_TEST_CLEANUP_SERVER", &server.url)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .unwrap();
    assert!(status.success());
    assert!(receipt.exists());
    drop(data);
    let deadline = Instant::now() + Duration::from_secs(5);
    while receipt.exists() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(!receipt.exists());
    assert_eq!(server.data.lock().unwrap().deleted, ["ses_old"]);
}

#[test]
#[ignore = "detached OpenCode cleanup launcher entry point"]
fn launcher_entry() {
    let mut service = AppService::initialize().unwrap();
    let config = service.config_for_tests();
    let observer = &mut std::sync::Arc::get_mut(&mut service.opencode)
        .unwrap()
        .observer;
    observer.presence = config.home.join("presence");
    observer.daemons = config.home.join("daemons");
    let directory = std::env::var("TANDEM_TEST_CLEANUP_DIRECTORY").unwrap();
    service.set_opencode_snapshot_for_tests(Snapshot {
        sessions: vec![Session {
            id: "ses_old".into(),
            directory: directory.clone(),
            server: std::env::var("TANDEM_TEST_CLEANUP_SERVER").unwrap(),
            activity: Activity::Idle,
            ..Default::default()
        }],
        ..Default::default()
    });
    drop(service.clear_opencode_folder(&directory).unwrap());
}

#[test]
#[ignore = "detached OpenCode cleanup worker entry point"]
fn worker_entry() {
    let fd = std::env::var("TANDEM_TEST_CLEANUP_FD")
        .unwrap()
        .parse()
        .unwrap();
    AppService::run_opencode_cleanup_worker(fd).unwrap();
}

#[test]
fn partial_folder_cleanup_restores_only_remaining_conversations() {
    let service = folder_service();
    let server = Server::start();
    server.data.lock().unwrap().v2 = true;
    for id in ["ses_a", "ses_b"] {
        server.session(id, "/work/review", None);
    }
    server.data.lock().unwrap().delete_failure_for = Some("ses_b".into());
    let observed = Snapshot {
        directories: vec!["/work/review".into()],
        sessions: ["ses_a", "ses_b"]
            .into_iter()
            .map(|id| Session {
                id: id.into(),
                directory: "/work/review".into(),
                server: server.url.clone(),
                activity: Activity::Idle,
                ..Default::default()
            })
            .collect(),
        ..Default::default()
    };
    service.set_opencode_snapshot_for_tests(observed.clone());
    let reply = service.clear_opencode_folder("/work/review").unwrap();
    assert!(reply.blocking_recv().unwrap().is_err());
    super::super::observation::publish(
        &std::sync::Arc::downgrade(&service.opencode),
        0,
        Ok(observed),
    );
    assert_eq!(service.opencode_snapshot().directories, ["/work/review"]);
    let remaining = service
        .opencode_snapshot()
        .sessions
        .into_iter()
        .map(|session| session.id)
        .collect::<Vec<_>>();
    assert_eq!(remaining, ["ses_b"]);
}

#[test]
fn folder_cleanups_and_session_actions_can_run_independently() {
    let service = folder_service();
    let server = Server::start();
    let other = Server::start();
    server.session("ses_a", "/work/a", None);
    server.session("ses_b", "/work/b", None);
    other.session("ses_other", "/work/other", None);
    service.set_opencode_snapshot_for_tests(Snapshot {
        sessions: [
            ("ses_a", "/work/a", &server),
            ("ses_b", "/work/b", &server),
            ("ses_other", "/work/other", &other),
        ]
        .into_iter()
        .map(|(id, directory, server)| Session {
            id: id.into(),
            directory: directory.into(),
            server: server.url.clone(),
            activity: Activity::Idle,
            ..Default::default()
        })
        .collect(),
        ..Default::default()
    });
    let data = server.data.lock().unwrap();
    let first = service.clear_opencode_folder("/work/a").unwrap();
    let second = service.clear_opencode_folder("/work/b").unwrap();
    let rename = service
        .rename_opencode_session("ses_other", "Renamed".into())
        .unwrap();
    rename.blocking_recv().unwrap().unwrap();
    assert_eq!(service.opencode_snapshot().sessions[0].title, "Renamed");
    drop(data);
    first.blocking_recv().unwrap().unwrap();
    second.blocking_recv().unwrap().unwrap();
    assert_eq!(service.opencode_snapshot().sessions[0].id, "ses_other");
}
