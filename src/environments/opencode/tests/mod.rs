use super::*;
use serde_json::json;
use std::{
    io::{BufRead, BufReader, Write},
    net::TcpListener,
    os::unix::fs::PermissionsExt,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread,
};

struct Server {
    url: String,
    stop: Arc<AtomicBool>,
    thread: Option<thread::JoinHandle<()>>,
    deleted: Arc<std::sync::Mutex<BTreeSet<String>>>,
    requests: Arc<std::sync::Mutex<Vec<String>>>,
    full_history: Arc<AtomicBool>,
    busy: Arc<AtomicBool>,
    failed_status_directories: Arc<std::sync::Mutex<BTreeSet<String>>>,
}

impl Server {
    fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let stop = Arc::new(AtomicBool::new(false));
        let stopping = stop.clone();
        let deleted = Arc::new(std::sync::Mutex::new(BTreeSet::<String>::new()));
        let removed = deleted.clone();
        let requests = Arc::new(std::sync::Mutex::new(Vec::new()));
        let recorded_requests = requests.clone();
        let full_history = Arc::new(AtomicBool::new(false));
        let full = full_history.clone();
        let busy = Arc::new(AtomicBool::new(true));
        let active = busy.clone();
        let failed_status_directories = Arc::new(std::sync::Mutex::new(BTreeSet::new()));
        let failed = failed_status_directories.clone();
        let thread = thread::spawn(move || {
            while !stopping.load(Ordering::Relaxed) {
                let Ok((mut stream, _)) = listener.accept() else {
                    thread::sleep(std::time::Duration::from_millis(5));
                    continue;
                };
                stream
                    .set_read_timeout(Some(std::time::Duration::from_secs(2)))
                    .unwrap();
                let mut request = String::new();
                BufReader::new(stream.try_clone().unwrap())
                    .read_line(&mut request)
                    .unwrap();
                recorded_requests.lock().unwrap().push(request.clone());
                let mut status = "200 OK";
                let body = if request.contains("/session/status?directory=") {
                    let target = request.split_whitespace().nth(1).unwrap();
                    let url = reqwest::Url::parse(&format!("http://localhost{target}")).unwrap();
                    let directory = url
                        .query_pairs()
                        .find(|(key, _)| key == "directory")
                        .unwrap()
                        .1;
                    if failed.lock().unwrap().contains(directory.as_ref()) {
                        status = "503 Service Unavailable";
                        json!({"error":"Directory unavailable"})
                    } else if active.load(Ordering::Relaxed) {
                        json!({"ses_busy":{"type":"busy"}, "ses_background":{"type":"retry"}})
                    } else {
                        json!({})
                    }
                } else if request.contains("/session/status") {
                    json!({})
                } else if request.contains("/experimental/session") {
                    let mut history = json!([
                        {"id":"ses_busy","title":"Same title","directory":"/work/review/repo","time":{"updated":4}},
                        {"id":"ses_idle","title":"Same title","directory":"/work/review/repo","time":{"updated":3}},
                        {"id":"ses_background","title":"Tests","directory":"/work/review","time":{"updated":2}},
                        {"id":"ses_saved","title":"Old conversation","directory":"/work/review","time":{"updated":1}},
                        {"id":"ses_elsewhere","title":"Other workspace","directory":"/work/review-other","time":{"updated":5}},
                        {"id":"ses_child","parentID":"ses_busy","title":"Subagent","directory":"/work/review","time":{"updated":6}}
                    ]);
                    let history = history.as_array_mut().unwrap();
                    history.retain(|session| {
                        !removed
                            .lock()
                            .unwrap()
                            .contains(session["id"].as_str().unwrap())
                    });
                    if full.load(Ordering::Relaxed) {
                        while history.len() < 1000 {
                            history.push(json!({"id":format!("ses_filler_{}", history.len()),"title":"Other","directory":"/unrelated","time":{"updated":100}}));
                        }
                    }
                    json!(history)
                } else if request.contains("/message?") {
                    json!([
                        {"info":{"id":"msg_1","role":"user","time":{"created":1}},"parts":[{"type":"text","text":"Earlier question"}]},
                        {"info":{"id":"msg_2","role":"assistant","time":{"created":2}},"parts":[{"type":"text","text":"Earlier answer"}]},
                        {"info":{"id":"msg_3","role":"user","time":{"created":3}},"parts":[{"type":"text","text":"Latest question"}]},
                        {"info":{"id":"msg_4","role":"assistant","time":{"created":4}},"parts":[{"type":"text","text":"Latest answer"}]}
                    ])
                } else if request.contains("GET /session/") {
                    status = "404 Not Found";
                    json!({"name":"NotFoundError"})
                } else {
                    json!({"healthy":true})
                };
                let body = body.to_string();
                let _ = write!(
                    stream,
                    "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
            }
        });
        Self {
            url,
            stop,
            thread: Some(thread),
            deleted,
            requests,
            full_history,
            busy,
            failed_status_directories,
        }
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        self.thread.take().unwrap().join().unwrap();
    }
}

fn observer(root: &Path) -> Observer {
    let program = root.join("zellij");
    fs::write(
        &program,
        r#"#!/bin/sh
root=$(dirname "$0")
printf '%s\n' "$*" >> "$root/calls"
case "$*" in
  list-sessions*) printf 'main\n' ;;
  *list-panes*) cat "$root/panes.json" ;;
  *) printf 'terminal_99\n' ;;
esac
"#,
    )
    .unwrap();
    fs::set_permissions(&program, fs::Permissions::from_mode(0o700)).unwrap();
    fs::write(
        root.join("panes.json"),
        json!([
            {"id":7,"is_plugin":false,"exited":false,"tab_id":4,"tab_name":"Review"},
            {"id":8,"is_plugin":false,"exited":false,"tab_id":4,"tab_name":"Review"},
            {"id":9,"is_plugin":false,"exited":false,"tab_id":4,"tab_name":"Review"}
        ])
        .to_string(),
    )
    .unwrap();
    let observer = Observer {
        presence: root.join("presence"),
        daemons: root.join("daemons"),
        zellij: program,
    };
    fs::create_dir_all(&observer.presence).unwrap();
    observer
}

fn presence(observer: &Observer, file: &str, id: &str, pane: u32, server: &str) {
    presence_in(observer, file, id, pane, server, "/work/review/repo");
}

fn presence_in(
    observer: &Observer,
    file: &str,
    id: &str,
    pane: u32,
    server: &str,
    directory: &str,
) {
    fs::write(observer.presence.join(file), json!({
        "pid":std::process::id(), "observed_at":SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_millis() as u64,
        "id":id, "title":"Same title", "directory":directory, "server":server,
        "activity":"idle", "context_tokens":83_600, "context_limit":272_000,
        "zellij_session":"main", "pane_id":pane
    }).to_string()).unwrap();
}

#[test]
fn companion_receipts_expose_sessions_outside_tandem_roots() {
    let root = tempfile::tempdir().unwrap();
    let server = Server::start();
    let observer = observer(root.path());
    presence_in(
        &observer,
        "outside.json",
        "ses_elsewhere",
        7,
        &server.url,
        "/work/review-other",
    );

    let snapshot = tokio::runtime::Runtime::new()
        .unwrap()
        .block_on(observer.observe(&["/work/review".into()], Snapshot::default()))
        .unwrap();

    let session = snapshot
        .sessions
        .iter()
        .find(|session| session.id == "ses_elsewhere")
        .unwrap();
    assert_eq!(session.directory, "/work/review-other");
    assert!(session.attached());
    let history_requests = server
        .requests
        .lock()
        .unwrap()
        .iter()
        .filter(|request| request.contains("/experimental/session"))
        .cloned()
        .collect::<Vec<_>>();
    assert_eq!(history_requests.len(), 1);
    assert!(history_requests.iter().any(|request| {
        request.contains("roots=true")
            && request.contains("limit=21")
            && request.contains("directory=%2Fwork%2Freview-other")
    }));
}

#[test]
fn shared_server_observations_join_exact_ids_and_track_conversation_switches() {
    let root = tempfile::tempdir().unwrap();
    let server = Server::start();
    let observer = observer(root.path());
    presence(&observer, "one.json", "ses_busy", 7, &server.url);
    presence(&observer, "two.json", "ses_idle", 8, &server.url);
    presence(&observer, "three.json", "ses_busy", 9, &server.url);
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let roots = ["/work/review".into()];
    let snapshot = runtime
        .block_on(observer.observe(&roots, Snapshot::default()))
        .unwrap();
    assert_eq!(snapshot.error, None);
    assert_eq!(snapshot.sessions.len(), 4);
    assert_eq!(
        crate::store::opencode::Counts::of(snapshot.sessions.iter()),
        crate::store::opencode::Counts {
            live: 3,
            attached: 2,
            busy: 2
        }
    );
    let busy = snapshot
        .sessions
        .iter()
        .find(|session| session.id == "ses_busy")
        .unwrap();
    assert_eq!(busy.panes.len(), 2);
    assert_eq!(busy.context_tokens, Some(83_600));
    assert_eq!(busy.context_limit, Some(272_000));
    assert_eq!(busy.last_question.as_deref(), Some("Latest question"));
    assert!(busy.question_observed);
    runtime
        .block_on(observer.jump("ses_busy", &busy.panes[0], "main"))
        .unwrap();
    let calls = fs::read_to_string(root.path().join("calls")).unwrap();
    assert!(calls.contains("go-to-tab-by-id 4"));
    assert!(calls.contains("focus-pane-id terminal_7"));

    presence(&observer, "one.json", "ses_idle", 7, &server.url);
    assert!(
        runtime
            .block_on(observer.jump("ses_busy", &busy.panes[0], "main"))
            .unwrap_err()
            .contains("changed conversation")
    );
    server.busy.store(false, Ordering::Relaxed);
    let snapshot = runtime
        .block_on(observer.observe(&roots, snapshot))
        .unwrap();
    assert_eq!(
        snapshot
            .sessions
            .iter()
            .find(|s| s.id == "ses_busy")
            .unwrap()
            .panes
            .len(),
        1
    );
    let finished = snapshot
        .sessions
        .iter()
        .find(|session| session.id == "ses_busy")
        .unwrap();
    assert_eq!(finished.activity, Activity::Idle);
    assert!(finished.activity_elapsed_milliseconds.is_some());
    assert_eq!(
        snapshot
            .sessions
            .iter()
            .find(|s| s.id == "ses_idle")
            .unwrap()
            .panes
            .len(),
        2
    );
    let idle = snapshot
        .sessions
        .iter()
        .find(|s| s.id == "ses_idle")
        .unwrap();
    runtime
        .block_on(observer.jump("ses_idle", &idle.panes[0], "other"))
        .unwrap();
    let calls = fs::read_to_string(root.path().join("calls")).unwrap();
    assert!(calls.contains("switch-session main --pane-id terminal_"));
}

#[test]
fn orphan_client_navigation_targets_its_exact_observed_pane() {
    let root = tempfile::tempdir().unwrap();
    let observer = observer(root.path());
    presence(&observer, "orphan.json", "", 7, "http://127.0.0.1:1234");
    let pane = Pane {
        session: "main".into(),
        id: 7,
        tab_id: 4,
        tab_name: "Review".into(),
    };

    tokio::runtime::Runtime::new()
        .unwrap()
        .block_on(observer.jump_pane(&pane, "main"))
        .unwrap();

    let calls = fs::read_to_string(root.path().join("calls")).unwrap();
    assert!(calls.contains("go-to-tab-by-id 4"));
    assert!(calls.contains("focus-pane-id terminal_7"));
}

#[test]
fn close_session_closes_the_exact_attached_zellij_pane() {
    let root = tempfile::tempdir().unwrap();
    let observer = observer(root.path());
    presence(
        &observer,
        "one.json",
        "ses_review",
        7,
        "http://127.0.0.1:1234",
    );
    let pane = Pane {
        session: "main".into(),
        id: 7,
        tab_id: 4,
        tab_name: "Review".into(),
    };

    tokio::runtime::Runtime::new()
        .unwrap()
        .block_on(observer.close("ses_review", &pane))
        .unwrap();

    let calls = fs::read_to_string(root.path().join("calls")).unwrap();
    assert!(
        calls.contains("--session main action close-pane --pane-id terminal_7"),
        "{calls}"
    );
}

#[test]
fn activity_timing_counts_up_and_freezes_when_the_session_becomes_idle() {
    let mut busy = Session {
        activity: Activity::Busy,
        updated: 1_000,
        ..Default::default()
    };
    update_activity_timing(&mut busy, None, 30_000);
    assert_eq!(busy.activity_started_at_milliseconds, Some(1_000));
    assert_eq!(busy.activity_elapsed_milliseconds, Some(29_000));

    let previous = busy.clone();
    update_activity_timing(&mut busy, Some(&previous), 115_000);
    assert_eq!(busy.activity_elapsed_milliseconds, Some(114_000));

    let mut idle = Session {
        activity: Activity::Idle,
        updated: 120_000,
        ..Default::default()
    };
    update_activity_timing(&mut idle, Some(&busy), 120_000);
    assert_eq!(idle.activity_started_at_milliseconds, None);
    assert_eq!(idle.activity_elapsed_milliseconds, Some(119_000));

    let previous = idle.clone();
    update_activity_timing(&mut idle, Some(&previous), 180_000);
    assert_eq!(idle.activity_elapsed_milliseconds, Some(119_000));
}

#[test]
fn latest_turn_reconstructs_activity_timing_without_a_previous_snapshot() {
    let turn = crate::store::opencode::conversation::LatestTurn {
        question: Some("Run the checks".into()),
        started_at: 70_000,
        completed_at: None,
    };
    let mut busy = Session {
        activity: Activity::Busy,
        updated: 90_000,
        ..Default::default()
    };
    apply_turn_timing(&mut busy, &turn, 100_000);
    assert_eq!(busy.activity_started_at_milliseconds, Some(70_000));
    assert_eq!(busy.activity_elapsed_milliseconds, Some(30_000));

    let mut idle = Session {
        activity: Activity::Idle,
        updated: 94_000,
        ..Default::default()
    };
    apply_turn_timing(&mut idle, &turn, 100_000);
    assert_eq!(idle.activity_started_at_milliseconds, None);
    assert_eq!(idle.activity_elapsed_milliseconds, Some(24_000));
}

#[test]
fn observer_preserves_the_run_start_and_completed_duration_across_refreshes() {
    let root = tempfile::tempdir().unwrap();
    let server = Server::start();
    let observer = observer(root.path());
    presence(&observer, "one.json", "ses_busy", 7, &server.url);
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let roots = ["/work/review".into()];
    let started = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64
        - 24_000;
    let previous = Snapshot {
        sessions: vec![Session {
            id: "ses_busy".into(),
            directory: "/work/review/repo".into(),
            server: server.url.clone(),
            activity: Activity::Busy,
            activity_started_at_milliseconds: Some(started),
            activity_elapsed_milliseconds: Some(24_000),
            ..Default::default()
        }],
        ..Default::default()
    };
    let busy = runtime
        .block_on(observer.observe(&roots, previous))
        .unwrap();
    let session = busy.sessions.iter().find(|s| s.id == "ses_busy").unwrap();
    assert_eq!(session.activity_started_at_milliseconds, Some(started));
    assert!(session.activity_elapsed_milliseconds.unwrap() >= 24_000);

    server.busy.store(false, Ordering::Relaxed);
    let idle = runtime.block_on(observer.observe(&roots, busy)).unwrap();
    let session = idle.sessions.iter().find(|s| s.id == "ses_busy").unwrap();
    let elapsed = session.activity_elapsed_milliseconds.unwrap();
    assert_eq!(session.activity, Activity::Idle);
    assert_eq!(session.activity_started_at_milliseconds, None);
    assert!(elapsed >= 24_000);
    let idle = runtime.block_on(observer.observe(&roots, idle)).unwrap();
    let session = idle.sessions.iter().find(|s| s.id == "ses_busy").unwrap();
    assert_eq!(session.activity_elapsed_milliseconds, Some(elapsed));

    server.busy.store(true, Ordering::Relaxed);
    let busy = runtime.block_on(observer.observe(&roots, idle)).unwrap();
    let session = busy.sessions.iter().find(|s| s.id == "ses_busy").unwrap();
    assert!(session.activity_started_at_milliseconds.unwrap() > started);
    assert!(session.activity_elapsed_milliseconds.unwrap() < elapsed);
}

#[test]
fn observation_failures_preserve_sessions_as_unknown_and_expired_receipts_do_not_attach() {
    let root = tempfile::tempdir().unwrap();
    let server = Server::start();
    let observer = observer(root.path());
    presence(&observer, "one.json", "ses_busy", 7, &server.url);
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let roots = ["/work/review".into()];
    let snapshot = runtime
        .block_on(observer.observe(&roots, Snapshot::default()))
        .unwrap();
    let path = observer.presence.join("one.json");
    let mut record: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
    record["observed_at"] = json!(0);
    fs::write(path, record.to_string()).unwrap();
    drop(server);
    let snapshot = runtime
        .block_on(observer.observe(&roots, snapshot))
        .unwrap();
    assert_eq!(snapshot.sessions.len(), 4);
    assert!(snapshot.error.is_some());
    assert!(
        snapshot
            .sessions
            .iter()
            .all(|s| s.stale && s.activity == Activity::Unknown && s.panes.is_empty())
    );
}

#[test]
fn opening_a_client_in_an_unavailable_directory_keeps_other_sessions_fresh() {
    let root = tempfile::tempdir().unwrap();
    let server = Server::start();
    let observer = observer(root.path());
    presence(&observer, "one.json", "ses_busy", 7, &server.url);
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let roots = ["/work/review".into()];
    let snapshot = runtime
        .block_on(observer.observe(&roots, Snapshot::default()))
        .unwrap();
    assert!(snapshot.sessions.iter().all(|session| !session.stale));

    presence_in(&observer, "new.json", "", 8, &server.url, "/work/a-new");
    server
        .failed_status_directories
        .lock()
        .unwrap()
        .insert("/work/a-new".into());
    let snapshot = runtime
        .block_on(observer.observe(&roots, snapshot))
        .unwrap();

    assert_eq!(snapshot.sessions.len(), 4);
    assert!(snapshot.sessions.iter().all(|session| !session.stale));
    assert_eq!(
        snapshot
            .sessions
            .iter()
            .find(|s| s.id == "ses_busy")
            .unwrap()
            .activity,
        Activity::Busy
    );
    assert_eq!(snapshot.clients.len(), 1);
    assert_eq!(snapshot.clients[0].directory, "/work/a-new");
    assert!(!snapshot.clients[0].stale);
}

#[test]
fn directory_status_failures_are_scoped_and_recover_on_the_next_observation() {
    let root = tempfile::tempdir().unwrap();
    let server = Server::start();
    let observer = observer(root.path());
    presence(&observer, "one.json", "ses_busy", 7, &server.url);
    presence_in(
        &observer,
        "outside.json",
        "ses_elsewhere",
        8,
        &server.url,
        "/work/review-other",
    );
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let roots = ["/work/review".into()];
    let snapshot = runtime
        .block_on(observer.observe(&roots, Snapshot::default()))
        .unwrap();

    server
        .failed_status_directories
        .lock()
        .unwrap()
        .insert("/work/review-other".into());
    let snapshot = runtime
        .block_on(observer.observe(&roots, snapshot))
        .unwrap();
    let healthy = snapshot
        .sessions
        .iter()
        .find(|s| s.id == "ses_busy")
        .unwrap();
    assert!(!healthy.stale);
    assert_eq!(healthy.activity, Activity::Busy);
    let affected = snapshot
        .sessions
        .iter()
        .find(|s| s.id == "ses_elsewhere")
        .unwrap();
    assert!(affected.stale);
    assert_eq!(affected.activity, Activity::Unknown);
    assert!(affected.attached());
    assert!(
        snapshot
            .error
            .as_ref()
            .unwrap()
            .contains("/work/review-other")
    );

    server.failed_status_directories.lock().unwrap().clear();
    let snapshot = runtime
        .block_on(observer.observe(&roots, snapshot))
        .unwrap();
    assert_eq!(snapshot.error, None);
    assert!(snapshot.sessions.iter().all(|session| !session.stale));
    assert_eq!(
        snapshot
            .sessions
            .iter()
            .find(|s| s.id == "ses_elsewhere")
            .unwrap()
            .activity,
        Activity::Idle
    );
}

#[test]
fn server_targets_reject_remote_hosts_credentials_paths_and_redirect_targets() {
    for value in [
        "https://127.0.0.1:4000",
        "http://example.com",
        "http://user:secret@localhost:4000",
        "http://localhost:4000/path",
        "http://localhost:4000?secret=value",
    ] {
        assert_eq!(local_server(value), None, "{value}");
    }
    assert_eq!(
        local_server("http://127.0.0.1:4199"),
        Some("http://127.0.0.1:4199".into())
    );
}

#[test]
fn daemon_directory_receipts_discover_detached_work_without_a_client() {
    let root = tempfile::tempdir().unwrap();
    let server = Server::start();
    let observer = observer(root.path());
    let station = observer.daemons.join("station");
    fs::create_dir_all(station.join("dirs")).unwrap();
    fs::write(station.join("port"), server.url.rsplit(':').next().unwrap()).unwrap();
    fs::write(station.join("dirs/work.dir"), "/work/review\n").unwrap();
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let snapshot = runtime
        .block_on(observer.observe(&["/work/review".into()], Snapshot::default()))
        .unwrap();
    assert_eq!(
        snapshot
            .sessions
            .iter()
            .filter(|session| session.activity == Activity::Busy)
            .count(),
        2
    );
    assert!(
        snapshot
            .sessions
            .iter()
            .all(|session| session.panes.is_empty())
    );
    assert!(
        snapshot
            .sessions
            .iter()
            .all(|session| session.server == server.url)
    );
}

#[test]
fn attach_uses_the_instance_tab_or_creates_an_instance_named_tab_with_literal_arguments() {
    let root = tempfile::tempdir().unwrap();
    let server = Server::start();
    let observer = observer(root.path());
    let session = Session {
        id: "ses_saved".into(),
        server: server.url.clone(),
        directory: "/work/space ' ; $(touch injected)".into(),
        title: "Old conversation".into(),
        ..Default::default()
    };
    let pane = Pane {
        session: "main".into(),
        id: 7,
        tab_id: 4,
        tab_name: "review".into(),
    };
    let runtime = tokio::runtime::Runtime::new().unwrap();
    runtime
        .block_on(observer.attach(&session, "review", "main", Some(&pane)))
        .unwrap();
    runtime
        .block_on(observer.attach(&session, "review", "main", None))
        .unwrap();
    let calls = fs::read_to_string(root.path().join("calls")).unwrap();
    assert!(
        calls.contains("new-pane --stacked --tab-id 4 --cwd /work/space ' ; $(touch injected)")
    );
    assert!(calls.contains("new-tab --name review"));
    assert!(calls.contains("--session ses_saved"));
    assert!(!root.path().join("injected").exists());
}

#[test]
fn a_client_on_the_home_screen_is_observed_without_counting_a_conversation() {
    let root = tempfile::tempdir().unwrap();
    let server = Server::start();
    let observer = observer(root.path());
    presence(&observer, "one.json", "", 7, &server.url);
    fs::write(root.path().join("panes.json"), json!([
        {"id":7,"is_plugin":false,"exited":false,"tab_id":4,"tab_name":"Review","pane_command":"opencode attach","pane_cwd":"/work/review/repo"}
    ]).to_string()).unwrap();
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let snapshot = runtime
        .block_on(observer.observe(&["/work/review".into()], Snapshot::default()))
        .unwrap();
    assert_eq!(snapshot.error, None);
    assert_eq!(snapshot.sessions.len(), 4);
    assert!(snapshot.sessions.iter().all(|session| !session.attached()));
    assert_eq!(snapshot.clients.len(), 1);
    assert_eq!(snapshot.clients[0].directory, "/work/review/repo");
    assert_eq!(snapshot.clients[0].pane.id, 7);
    assert!(!snapshot.clients[0].stale);
}

#[test]
fn deleted_conversations_disappear_from_full_history_even_with_a_lingering_client_receipt() {
    let root = tempfile::tempdir().unwrap();
    let server = Server::start();
    let observer = observer(root.path());
    presence(&observer, "one.json", "ses_idle", 7, &server.url);
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let roots = ["/work/review".into()];
    let snapshot = runtime
        .block_on(observer.observe(&roots, Snapshot::default()))
        .unwrap();
    assert!(
        snapshot
            .sessions
            .iter()
            .any(|session| session.id == "ses_idle")
    );
    server
        .deleted
        .lock()
        .unwrap()
        .extend(["ses_idle".into(), "ses_saved".into()]);
    server.full_history.store(true, Ordering::Relaxed);
    let snapshot = runtime
        .block_on(observer.observe(&roots, snapshot))
        .unwrap();
    assert_eq!(snapshot.error, None);
    assert_eq!(
        snapshot
            .sessions
            .iter()
            .map(|session| session.id.as_str())
            .collect::<Vec<_>>(),
        ["ses_background", "ses_busy"]
    );
    assert!(snapshot.sessions.iter().all(|session| !session.stale));
}

#[test]
fn conversation_reader_fetches_all_user_and_agent_turns_from_the_server() {
    let server = Server::start();
    let session = Session {
        id: "ses_conversation".into(),
        server: server.url.clone(),
        directory: "/work/review & notes".into(),
        ..Default::default()
    };
    let text = tokio::runtime::Runtime::new()
        .unwrap()
        .block_on(super::conversation(&session))
        .unwrap();
    for part in [
        "Earlier question",
        "Earlier answer",
        "Latest question",
        "Latest answer",
    ] {
        assert!(text.contains(part));
    }
    assert!(text.find("Latest answer").unwrap() < text.find("Latest question").unwrap());
}
