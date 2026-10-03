use super::*;
use serde_json::json;
use std::{
    fs,
    os::unix::fs::PermissionsExt,
    sync::atomic::{AtomicBool, AtomicUsize, Ordering},
    time::{Instant, SystemTime, UNIX_EPOCH},
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

#[test]
fn unverified_clients_retry_quietly_stop_tracking_and_recover_on_receipt_changes() {
    let root = tempfile::tempdir().unwrap();
    let runtime = tokio::runtime::Runtime::new().unwrap();
    runtime.block_on(async {
        let observer = Observer {
            presence: root.path().join("presence"),
            daemons: root.path().join("daemons"),
            zellij: root.path().join("zellij"),
            excluded: Default::default(),
        };
        fs::create_dir(&observer.presence).unwrap();
        fs::write(&observer.zellij, r#"#!/bin/sh
root=$(dirname "$0")
case "$*" in
list-sessions*) echo main ;;
*list-panes*)
  echo check >> "$root/checks"
  if [ -f "$root/fail" ]; then echo unavailable >&2; exit 1; fi
  echo '[{"id":7,"is_plugin":false,"exited":false,"tab_id":4,"tab_name":"test"}]' ;;
*list-tabs*) echo '[{"tab_id":4,"position":0}]' ;;
esac
"#).unwrap();
        fs::set_permissions(&observer.zellij, fs::Permissions::from_mode(0o700)).unwrap();
        let receipt = observer.presence.join("one.json");
        let mut value = json!({"pid":std::process::id(),"observed_at":SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_millis() as u64,
            "id":"ses_one","title":"Client","directory":root.path(),"server":"","activity":"idle","zellij_session":"main","pane_id":7});
        fs::write(&receipt, value.to_string()).unwrap();
        let settings = Arc::new(crate::service::settings::Settings::open(root.path().join("settings.sqlite3")).unwrap());
        let owner = Arc::new(Integration::new());
        let signal = Arc::new(Signal::default());
        let worker = tokio::spawn(run(observer, Arc::downgrade(&owner), settings, 0, Arc::clone(&signal)));
        tokio::time::timeout(Duration::from_secs(3), async {
            loop {
                if !owner.state.lock().unwrap().snapshot.sessions.is_empty() { break; }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        }).await.unwrap();
        fs::write(root.path().join("fail"), "").unwrap();
        signal.send(REMOTE);
        tokio::time::timeout(Duration::from_secs(7), async {
            loop {
                let done = {
                    let state = owner.state.lock().unwrap();
                    let visible = state.retention.visible_snapshot(&state.snapshot);
                    if state.snapshot.sessions.iter().any(|session| session.stale) {
                        assert!(visible.sessions.is_empty());
                        assert_eq!(visible.error, None);
                    }
                    state.retention.exclusions().sessions.contains("ses_one") && state.retention.next_retry().is_none()
                };
                if done { break; }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        }).await.unwrap();
        let checks = fs::read_to_string(root.path().join("checks")).unwrap().lines().count();
        value["observed_at"] = json!(SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_millis() as u64);
        fs::write(&receipt, value.to_string()).unwrap();
        tokio::time::sleep(Duration::from_millis(300)).await;
        assert_eq!(fs::read_to_string(root.path().join("checks")).unwrap().lines().count(), checks);
        assert!(owner.state.lock().unwrap().snapshot.sessions.is_empty());

        fs::remove_file(root.path().join("fail")).unwrap();
        value["title"] = json!("Recovered client");
        fs::write(&receipt, value.to_string()).unwrap();
        tokio::time::timeout(Duration::from_secs(3), async {
            loop {
                if owner.state.lock().unwrap().snapshot.sessions.iter().any(|session| session.title == "Recovered client" && !session.stale) { break; }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        }).await.unwrap();
        assert_eq!(json!(serde_json::from_str::<serde_json::Value>(&fs::read_to_string(&receipt).unwrap()).unwrap()), value);
        worker.abort();
    });
}

#[test]
fn live_events_refresh_state_and_idle_sampling_does_not_query_the_server() {
    let root = tempfile::tempdir().unwrap();
    let runtime = tokio::runtime::Runtime::new().unwrap();
    runtime.block_on(async {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let server = format!("http://{}", listener.local_addr().unwrap());
        let busy = Arc::new(AtomicBool::new(false));
        let requests = Arc::new(AtomicUsize::new(0));
        let connections = Arc::new(AtomicUsize::new(0));
        let (events, _) = tokio::sync::broadcast::channel::<Option<String>>(16);
        let address = root.path().to_string_lossy().into_owned();
        let active = Arc::clone(&busy);
        let calls = Arc::clone(&requests);
        let streams = Arc::clone(&connections);
        let sent = events.clone();
        let directory = address.clone();
        let http = tokio::spawn(async move {
            loop {
                let (mut socket, _) = listener.accept().await.unwrap();
                let active = Arc::clone(&active);
                let calls = Arc::clone(&calls);
                let streams = Arc::clone(&streams);
                let mut events = sent.subscribe();
                let directory = directory.clone();
                tokio::spawn(async move {
                    let mut buffer = [0; 8192];
                    let size = socket.read(&mut buffer).await.unwrap();
                    let request = String::from_utf8_lossy(&buffer[..size]);
                    if request.starts_with("GET /api/event ") {
                        socket.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nConnection: close\r\n\r\n").await.unwrap();
                        streams.fetch_add(1, Ordering::SeqCst);
                        socket.write_all(b"data: {\"type\":\"server.connected\"}\n\n").await.unwrap();
                        while let Ok(Some(event)) = events.recv().await {
                            if socket.write_all(format!("data: {event}\n\n").as_bytes()).await.is_err() { break; }
                        }
                        return;
                    }
                    calls.fetch_add(1, Ordering::SeqCst);
                    let body = if request.starts_with("GET /api/info ") {
                        json!({"version":"2.0.22"})
                    } else if request.starts_with("GET /api/session/active ") {
                        json!({"data":if active.load(Ordering::SeqCst) { json!({"ses_one":{"type":"running"}}) } else { json!({}) }})
                    } else if request.starts_with("GET /api/session?") {
                        json!({"data":[{"id":"ses_one","title":"Live session","location":{"directory":directory},"time":{"updated":1}}]})
                    } else {
                        json!({"data":[]})
                    }.to_string();
                    let _ = socket.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).as_bytes()).await;
                });
            }
        });
        let observer = Observer { presence: root.path().join("presence"), daemons: root.path().join("daemons"), zellij: root.path().join("zellij"), excluded: Default::default() };
        fs::create_dir(&observer.presence).unwrap();
        fs::create_dir(&observer.daemons).unwrap();
        let dormant = tokio::net::TcpSocket::new_v4().unwrap();
        dormant.bind("127.0.0.1:0".parse().unwrap()).unwrap();
        let dormant_record = observer.daemons.join("dormant");
        fs::create_dir_all(dormant_record.join("dirs")).unwrap();
        fs::write(dormant_record.join("port"), dormant.local_addr().unwrap().port().to_string()).unwrap();
        fs::write(dormant_record.join("dirs/work.dir"), &address).unwrap();
        fs::write(root.path().join("tabs.json"), "[{\"tab_id\":4,\"position\":0}]").unwrap();
        fs::write(&observer.zellij, "#!/bin/sh\ncase \"$*\" in\nlist-sessions*) echo main ;;\n*list-tabs*) cat \"$(dirname \"$0\")/tabs.json\" ;;\n*) echo '[{\"id\":7,\"is_plugin\":false,\"exited\":false,\"tab_id\":4,\"tab_name\":\"test\"}]' ;;\nesac\n").unwrap();
        fs::set_permissions(&observer.zellij, fs::Permissions::from_mode(0o755)).unwrap();
        let receipt = observer.presence.join("one.json");
        let value = json!({"pid":std::process::id(),"observed_at":SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_millis() as u64,
            "id":"ses_one","title":"Live session","directory":address,"server":server,"activity":"idle",
            "last_question":"Test question","zellij_session":"main","pane_id":7,"tab_index":0});
        fs::write(&receipt, value.to_string()).unwrap();
        let settings = Arc::new(crate::service::settings::Settings::open(root.path().join("settings.sqlite3")).unwrap());
        let mut owner = Integration::new();
        owner.observer = observer.clone();
        let owner = Arc::new(owner);
        let signal = Arc::new(Signal::default());
        let worker = tokio::spawn(run(observer, Arc::downgrade(&owner), settings, 0, Arc::clone(&signal)));
        let wait = |activity| {
            let owner = Arc::clone(&owner);
            async move {
                tokio::time::timeout(Duration::from_secs(3), async {
                    loop {
                        if owner.state.lock().unwrap().snapshot.sessions.iter().any(|session| session.id == "ses_one" && session.activity == activity && !session.stale) { break; }
                        tokio::time::sleep(Duration::from_millis(10)).await;
                    }
                }).await.unwrap();
            }
        };
        wait(crate::store::opencode::Activity::Idle).await;
        tokio::time::timeout(Duration::from_secs(3), async {
            let mut count = requests.load(Ordering::SeqCst);
            let mut quiet_since = Instant::now();
            loop {
                tokio::time::sleep(Duration::from_millis(20)).await;
                let latest = requests.load(Ordering::SeqCst);
                if latest != count { count = latest; quiet_since = Instant::now(); }
                if connections.load(Ordering::SeqCst) == 1 && quiet_since.elapsed() >= Duration::from_millis(300) { break; }
            }
        }).await.unwrap();
        let before = requests.load(Ordering::SeqCst);
        assert_eq!(owner.state.lock().unwrap().snapshot.error, None);
        assert!(signal.errors().is_empty());
        assert_eq!(owner.state.lock().unwrap().snapshot.zellij_tabs["main"][&4], 0);
        fs::write(root.path().join("tabs.json"), "[{\"tab_id\":4,\"position\":1}]").unwrap();
        // Cross the sampling deadline while keeping the source quiet.
        tokio::time::sleep(Duration::from_millis(5100)).await;
        assert_eq!(requests.load(Ordering::SeqCst), before, "stream connections: {}; snapshot: {:?}", connections.load(Ordering::SeqCst), owner.state.lock().unwrap().snapshot);
        assert_eq!(connections.load(Ordering::SeqCst), 1);
        assert_eq!(owner.state.lock().unwrap().snapshot.resources.len(), 1);
        assert_eq!(owner.state.lock().unwrap().snapshot.zellij_tabs["main"][&4], 1);
        busy.store(true, Ordering::SeqCst);
        let started = Instant::now();
        events.send(Some(json!({"type":"session.execution.started"}).to_string())).unwrap();
        wait(crate::store::opencode::Activity::Busy).await;
        assert!(started.elapsed() < Duration::from_secs(1));
        busy.store(false, Ordering::SeqCst);
        events.send(None).unwrap();
        wait(crate::store::opencode::Activity::Idle).await;
        assert_eq!(connections.load(Ordering::SeqCst), 2);
        worker.abort();
        worker.await.unwrap_err();
        http.abort();
    });
}
