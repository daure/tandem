use super::*;
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    io::{BufRead, BufReader, Read, Write},
    net::TcpListener,
    sync::{
        Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

#[path = "rule_settings.rs"]
mod settings;

#[derive(Default)]
struct Data {
    sessions: BTreeMap<String, Value>,
    calls: Vec<(String, Value)>,
    disconnect: bool,
    hidden: bool,
    lookup_error: bool,
}

struct Server {
    data: Arc<Mutex<Data>>,
    stop: Arc<AtomicBool>,
    thread: Option<thread::JoinHandle<()>>,
}

impl Server {
    fn start(service: &AppService) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let config = service.config_for_tests();
        let directory = config
            .workspaces
            .join("shared")
            .to_str()
            .unwrap()
            .to_owned();
        let data = Arc::new(Mutex::new(Data::default()));
        let inputs = data.clone();
        let stop = Arc::new(AtomicBool::new(false));
        let stopped = stop.clone();
        let server = url.clone();
        let path = directory.clone();
        let thread = thread::spawn(move || {
            while !stopped.load(Ordering::Acquire) {
                let Ok((mut stream, _)) = listener.accept() else {
                    thread::sleep(Duration::from_millis(2));
                    continue;
                };
                stream
                    .set_read_timeout(Some(Duration::from_secs(2)))
                    .unwrap();
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                let mut line = String::new();
                if reader.read_line(&mut line).unwrap_or_default() == 0 {
                    continue;
                }
                let route = line.split_whitespace().nth(1).unwrap().to_owned();
                let mut length = 0;
                let mut authorized = false;
                loop {
                    line.clear();
                    reader.read_line(&mut line).unwrap();
                    if line == "\r\n" {
                        break;
                    }
                    if let Some(value) = line.strip_prefix("content-length: ") {
                        length = value.trim().parse().unwrap();
                    }
                    authorized |= line == "authorization: Bearer client-secret\r\n";
                }
                let mut bytes = vec![0; length];
                reader.read_exact(&mut bytes).unwrap();
                let mut data = inputs.lock().unwrap();
                let mut status = "200 OK";
                let response = if route == "/api/info" {
                    json!({"version":"2.0.22"})
                } else if let Some(id) = route.strip_prefix("/api/session/") {
                    if data.lookup_error {
                        status = "503 Service Unavailable";
                        json!({})
                    } else if !data.hidden && let Some(session) = data.sessions.get(id) {
                        json!({"data":session})
                    } else {
                        status = "404 Not Found";
                        json!({})
                    }
                } else {
                    assert!(authorized);
                    assert!(matches!(route.as_str(), "/sessions" | "/sessions/prompt"));
                    let input: Value = serde_json::from_slice(&bytes).unwrap();
                    let lock = fs::OpenOptions::new().read(true).write(true).open(config.home.join("locks/instance-shared")).unwrap();
                    assert!(lock.try_lock().is_err(), "the instance lock covers prompt delivery");
                    data.calls.push((route.clone(), input.clone()));
                    let id = if route == "/sessions" {
                        let id = input["create_id"].as_str().unwrap();
                        data.sessions.insert(id.into(), json!({"id":id, "title":input["title"], "location":{"directory":path}}));
                        id
                    } else { input["sessionID"].as_str().unwrap() };
                    if data.disconnect { continue; }
                    json!({"session_id":id, "server":server, "prompt_outcome":if input["when_busy"] == "queue" {"queued"} else {"submitted"}, "error":null})
                }.to_string();
                stream.write_all(format!("HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{response}", response.len()).as_bytes()).unwrap();
            }
        });
        let presence = &service.opencode.observer.presence;
        fs::create_dir_all(presence).unwrap();
        fs::write(presence.join("named.json"), json!({
            "pid":std::process::id(), "observed_at":SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_millis(),
            "id":"", "title":"OpenCode", "directory":directory, "server":url, "activity":"idle",
            "zellij_session":"main", "pane_id":100,
            "tab_control":{"server":url,"token":"client-secret","prompted_sessions":true,"session_prompts":true,"session_tabs":true,"named_rule_sessions":true}
        }).to_string()).unwrap();
        Self {
            data,
            stop,
            thread: Some(thread),
        }
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        self.thread.take().unwrap().join().unwrap();
    }
}

fn restarted(service: &AppService) -> AppService {
    let mut restarted = AppService::from_config(service.config_for_tests()).unwrap();
    let observer = &mut Arc::get_mut(&mut restarted.opencode).unwrap().observer;
    observer.zellij = service.opencode.observer.zellij.clone();
    observer.presence = service.opencode.observer.presence.clone();
    restarted
}

#[test]
fn named_sessions_reuse_ids_after_restart_title_edits_and_event_history_deletion() {
    let service = fixture_with_session(true, Some("triage"));
    let server = Server::start(&service);
    service.rule_cycle().unwrap();
    let rows = service.rules.store.snapshot().unwrap().acceptances;
    assert_eq!(rows.len(), 2);
    assert!(rows.iter().all(|row| row.status == DispatchStatus::Launched
        && row.prompt_outcome == Some(crate::store::opencode::PromptOutcome::Queued)));
    assert_eq!(rows[0].session_id, rows[1].session_id);
    let id = rows[0].session_id.clone().unwrap();
    server.data.lock().unwrap().sessions.get_mut(&id).unwrap()["title"] =
        json!("User-edited title");
    let history = service.rules.store.snapshot().unwrap().workspaces;
    assert!(
        rows.iter().all(
            |row| history[&row.id].sessions.len() == 1 && history[&row.id].sessions[0].id == id
        )
    );
    service
        .delete_events(crate::store::events::Deletion::All)
        .blocking_recv()
        .unwrap()
        .unwrap();
    let restarted = restarted(&service);
    let events =
        crate::environments::events::EventStore::open(&service.config_for_tests()).unwrap();
    let token = events.register_provider("sample").unwrap();
    events
        .ingest(
            &token,
            Batch {
                events: vec![crate::environments::events::tests::event("third")],
            },
        )
        .unwrap();
    restarted.rule_cycle().unwrap();
    let rows = restarted.rules.store.snapshot().unwrap().acceptances;
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].session_id.as_deref(), Some(id.as_str()));
    let data = server.data.lock().unwrap();
    assert_eq!(
        data.calls
            .iter()
            .map(|(path, _)| path.as_str())
            .collect::<Vec<_>>(),
        ["/sessions", "/sessions/prompt", "/sessions/prompt"]
    );
    assert_eq!(data.sessions[&id]["title"], "User-edited title");
    assert_eq!(launches(&service), 0);
}

#[test]
fn named_session_delivery_modes_and_background_focus_reach_the_exact_conversation() {
    use crate::store::rules::DeliveryMode;
    for mode in [
        DeliveryMode::Queue,
        DeliveryMode::Steer,
        DeliveryMode::Interrupt,
    ] {
        let service = fixture_with_session(true, Some("triage"));
        let server = Server::start(&service);
        for mut acceptance in service.rules.store.snapshot().unwrap().acceptances {
            acceptance.rule.definition.delivery_mode = mode;
            acceptance.rule.definition.focus_pane = false;
            service.rules.store.update(&acceptance, false).unwrap();
        }
        service.rule_cycle().unwrap();
        let data = server.data.lock().unwrap();
        assert_eq!(data.calls.len(), 2);
        assert!(data.calls.iter().all(|(_, input)| input["when_busy"]
            == serde_json::to_value(mode).unwrap()
            && input["focus"] == false));
        assert_eq!(data.calls[0].1["create_id"], data.calls[1].1["sessionID"]);
        assert_eq!(data.calls[0].1["title"], "triage");
        let calls =
            fs::read_to_string(service.config_for_tests().home.join("target-calls")).unwrap();
        assert!(!calls.contains("focus-pane-id"));
        assert!(!calls.contains("go-to-tab-by-id"));
    }
}

#[test]
fn named_sessions_create_missing_targets_and_replace_confirmed_deleted_conversations() {
    let service = fixture_with_session(false, Some("triage"));
    let server = Server::start(&service);
    service.rule_cycle().unwrap();
    let operation = startup::read(&service.config_for_tests(), "shared")
        .unwrap()
        .unwrap()
        .operation
        .id;
    assert_eq!(
        service
            .runtime
            .block_on(service.wait_operation(&operation))
            .unwrap()
            .state,
        OperationState::Succeeded
    );
    service.rule_cycle().unwrap();
    let mut rows = service.rules.store.snapshot().unwrap().acceptances;
    assert!(
        rows.iter()
            .all(|row| row.status == DispatchStatus::Launched),
        "{rows:?}"
    );
    let first_id = rows[0].session_id.clone().unwrap();
    server.data.lock().unwrap().sessions.remove(&first_id);
    let events =
        crate::environments::events::EventStore::open(&service.config_for_tests()).unwrap();
    let token = events.register_provider("sample").unwrap();
    events
        .ingest(
            &token,
            Batch {
                events: vec![crate::environments::events::tests::event("third")],
            },
        )
        .unwrap();
    service.rule_cycle().unwrap();
    rows = service.rules.store.snapshot().unwrap().acceptances;
    assert_eq!(rows[0].status, DispatchStatus::Launched);
    assert_ne!(rows[0].session_id.as_ref(), Some(&first_id));
    assert_eq!(
        server
            .data
            .lock()
            .unwrap()
            .calls
            .iter()
            .filter(|(path, _)| path == "/sessions")
            .count(),
        2
    );
}

#[test]
fn uncertain_named_creation_never_duplicates_or_resends_a_previous_events_prompt() {
    for hidden in [false, true] {
        let service = fixture_with_session(true, Some("triage"));
        let server = Server::start(&service);
        server.data.lock().unwrap().disconnect = true;
        let mut rows = service.rules.store.snapshot().unwrap().acceptances;
        rows.sort_by_key(|row| row.id);
        let mut available = 4;
        service
            .advance_retained_acceptance(&rows[0], &mut available)
            .unwrap();
        assert_eq!(
            service.rules.store.acceptance(rows[0].id).unwrap().status,
            DispatchStatus::Uncertain
        );
        {
            let mut data = server.data.lock().unwrap();
            data.disconnect = false;
            data.hidden = hidden;
        }
        let restarted = restarted(&service);
        restarted.rule_cycle().unwrap();
        let second = restarted.rules.store.acceptance(rows[1].id).unwrap();
        assert_eq!(
            second.status,
            if hidden {
                DispatchStatus::Failed
            } else {
                DispatchStatus::Launched
            }
        );
        let data = server.data.lock().unwrap();
        assert_eq!(
            data.calls
                .iter()
                .filter(|(path, _)| path == "/sessions")
                .count(),
            1
        );
        assert_eq!(data.calls.len(), if hidden { 1 } else { 2 });
    }
}

#[test]
fn named_sessions_reject_unavailable_servers_and_mismatched_workspace_identity() {
    for unavailable in [true, false] {
        let service = fixture_with_session(true, Some("triage"));
        let server = Server::start(&service);
        let mut rows = service.rules.store.snapshot().unwrap().acceptances;
        rows.sort_by_key(|row| row.id);
        let mut available = 4;
        service
            .advance_retained_acceptance(&rows[0], &mut available)
            .unwrap();
        {
            let mut data = server.data.lock().unwrap();
            data.lookup_error = unavailable;
            if !unavailable {
                for session in data.sessions.values_mut() {
                    session["location"]["directory"] = json!("/other/workspace");
                }
            }
        }
        service.rule_cycle().unwrap();
        assert_eq!(
            service.rules.store.acceptance(rows[1].id).unwrap().status,
            DispatchStatus::Failed
        );
        assert_eq!(server.data.lock().unwrap().calls.len(), 1);
    }
}
