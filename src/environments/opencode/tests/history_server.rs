use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    io::{BufRead, BufReader, Read, Write},
    net::TcpListener,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::Duration,
};

#[derive(Default)]
pub struct Data {
    pub v2: bool,
    pub approval: bool,
    pub queued: bool,
    pub sessions: BTreeMap<String, Value>,
    pub deleted: Vec<String>,
    pub busy: bool,
    pub awaiting_answer: bool,
    pub status_failure: bool,
    pub delete_failure: bool,
    pub delete_failure_for: Option<String>,
    pub delete_lies: bool,
    pub unfiltered_list: bool,
    pub rename_lies: bool,
    pub renamed: Vec<Value>,
    pub interrupted: Vec<String>,
    pub interrupt_failure: bool,
    pub interrupt_lies: bool,
}

pub struct Server {
    pub url: String,
    pub data: Arc<Mutex<Data>>,
    stop: Arc<AtomicBool>,
    thread: Option<thread::JoinHandle<()>>,
}

impl Server {
    pub fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let data = Arc::new(Mutex::new(Data::default()));
        let shared = Arc::clone(&data);
        let stop = Arc::new(AtomicBool::new(false));
        let stopping = Arc::clone(&stop);
        let thread = thread::spawn(move || {
            while !stopping.load(Ordering::Relaxed) {
                let Ok((mut stream, _)) = listener.accept() else {
                    thread::sleep(Duration::from_millis(2));
                    continue;
                };
                stream
                    .set_read_timeout(Some(Duration::from_secs(2)))
                    .unwrap();
                let mut request = String::new();
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                if reader.read_line(&mut request).is_err() {
                    continue;
                }
                let mut length = 0;
                loop {
                    let mut header = String::new();
                    if reader.read_line(&mut header).unwrap_or(0) == 0 || header == "\r\n" {
                        break;
                    }
                    if let Some((name, value)) = header.split_once(':')
                        && name.eq_ignore_ascii_case("content-length")
                    {
                        length = value.trim().parse::<usize>().unwrap();
                    }
                }
                let mut body = vec![0; length];
                if reader.read_exact(&mut body).is_err() {
                    continue;
                }
                let payload: Value = serde_json::from_slice(&body).unwrap_or(Value::Null);
                let mut parts = request.split_whitespace();
                let method = parts.next().unwrap_or_default();
                let Some(path) = parts.next() else {
                    continue;
                };
                let url = reqwest::Url::parse(&format!("http://localhost{path}")).unwrap();
                let directory = url
                    .query_pairs()
                    .find(|(key, _)| key == "directory")
                    .map(|(_, value)| value.into_owned())
                    .unwrap_or_default();
                let mut data = shared.lock().unwrap();
                if method == "PATCH" {
                    let id = url
                        .path()
                        .strip_prefix("/api/session/")
                        .or_else(|| url.path().strip_prefix("/session/"));
                    if let Some(id) = id {
                        data.renamed.push(payload.clone());
                        if !data.rename_lies
                            && let Some(session) = data.sessions.get_mut(id)
                        {
                            session["title"] = payload["title"].clone();
                        }
                    }
                }
                let (status, body) = if data.v2 {
                    respond_v2(&mut data, method, &url)
                } else {
                    respond(&mut data, method, url.path(), &directory)
                };
                drop(data);
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
            data,
            stop,
            thread: Some(thread),
        }
    }

    pub fn session(&self, id: &str, directory: &str, parent: Option<&str>) {
        self.data.lock().unwrap().sessions.insert(
            id.into(),
            json!({
                "id": id, "directory": directory, "parentID": parent, "title": "Fixture", "time": {"updated": 1}
            }),
        );
    }
}

fn respond_v2(data: &mut Data, method: &str, url: &reqwest::Url) -> (&'static str, Value) {
    let query = |name| {
        url.query_pairs()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.into_owned())
    };
    let native = |mut session: Value| {
        session["location"] = json!({"directory": session["directory"]});
        if session["title"].is_null() {
            session["title"] = json!("Fixture");
        }
        session["time"] = json!({"updated": 1});
        session.as_object_mut().unwrap().remove("directory");
        session
    };
    match url.path() {
        "/api/info" => ("200 OK", json!({"version":"2.0.22"})),
        "/api/session/active" if data.status_failure => ("503 Service Unavailable", json!({})),
        "/api/session/active" => (
            "200 OK",
            json!({"data": if data.busy { json!({"ses_old":{"type":"running"}}) } else { json!({}) }}),
        ),
        "/api/form" => (
            "200 OK",
            json!({"data": if data.awaiting_answer { json!([{"sessionID":"ses_old"}]) } else { json!([]) }}),
        ),
        "/api/permission/request" => ("200 OK", json!({"data":[]})),
        "/api/session" => {
            let limit = query("limit")
                .and_then(|value| value.parse::<usize>().ok())
                .unwrap_or(50);
            let offset = query("cursor")
                .and_then(|value| {
                    value
                        .strip_prefix("opaque_")
                        .and_then(|value| value.parse::<usize>().ok())
                })
                .unwrap_or(0);
            let directory = query("directory");
            let parent = query("parentID");
            let found = data
                .sessions
                .values()
                .filter(|session| {
                    directory.as_ref().is_none_or(|directory| {
                        data.unfiltered_list || session["directory"] == *directory
                    }) && parent
                        .as_ref()
                        .is_none_or(|parent| session["parentID"] == *parent)
                })
                .cloned()
                .map(native)
                .collect::<Vec<_>>();
            let next = (offset + limit < found.len()).then(|| format!("opaque_{}", offset + limit));
            (
                "200 OK",
                json!({"data": found.iter().skip(offset).take(limit).collect::<Vec<_>>(), "cursor": {"next":next}}),
            )
        }
        path if path.ends_with("/permission") => (
            "200 OK",
            json!({"data":if data.approval { json!([{"sessionID":"ses_old"}]) } else { json!([]) }}),
        ),
        path if method == "POST" && path.ends_with("/interrupt") => {
            let id = path
                .strip_prefix("/api/session/")
                .unwrap()
                .strip_suffix("/interrupt")
                .unwrap();
            assert_eq!(query("resume").as_deref(), Some("false"));
            interrupt(data, id)
        }
        path if path.ends_with("/form") => (
            "200 OK",
            json!({"data":if data.awaiting_answer { json!([{"sessionID":"ses_old"}]) } else { json!([]) }}),
        ),
        path if path.ends_with("/inbox") => (
            "200 OK",
            json!({"data":if data.queued { json!([{"id":"msg_parked"}]) } else { json!([]) }}),
        ),
        path if path.ends_with("/export") => (
            "200 OK",
            json!({"data":{"messages":[
                {"id":"msg_1","type":"user","text":"Native question","time":{"created":1}},
                {"id":"msg_2","type":"assistant","agent":"tracer","model":{"providerID":"openai","id":"test","variant":"high"},"content":[
                    {"type":"reasoning","text":"Native reasoning","state":{"itemId":"reasoning_1","reasoningEncryptedContent":"opaque"}},
                    {"type":"tool","id":"call_1","name":"shell","state":{"status":"completed","input":{},"content":[{"type":"text","text":"Done"}]},"time":{"created":2}},
                    {"type":"text","text":"Native answer","state":{"itemId":"text_1","phase":"final_answer"}}
                ],"time":{"created":2}}
            ]}}),
        ),
        path => {
            let Some(id) = path.strip_prefix("/api/session/") else {
                return ("404 Not Found", json!({}));
            };
            if method == "DELETE" {
                if data.delete_failure || data.delete_failure_for.as_deref() == Some(id) {
                    return ("500 Internal Server Error", json!({}));
                }
                data.deleted.push(id.into());
                if !data.delete_lies {
                    remove_tree(data, id);
                }
                return ("204 No Content", Value::Null);
            }
            if method == "PATCH" && data.sessions.contains_key(id) {
                return ("204 No Content", Value::Null);
            }
            match data.sessions.get(id) {
                Some(session) => ("200 OK", json!({"data":native(session.clone())})),
                None => ("404 Not Found", json!({})),
            }
        }
    }
}

fn respond(data: &mut Data, method: &str, path: &str, directory: &str) -> (&'static str, Value) {
    match path {
        "/global/health" => ("200 OK", json!({"healthy": true})),
        "/experimental/session" => (
            "200 OK",
            json!(
                data.sessions
                    .values()
                    .filter(|session| data.unfiltered_list || session["directory"] == directory)
                    .collect::<Vec<_>>()
            ),
        ),
        "/session/status" if data.status_failure => ("503 Service Unavailable", json!({})),
        "/session/status" => (
            "200 OK",
            if data.busy {
                json!({"ses_old": {"type": "busy"}})
            } else {
                json!({})
            },
        ),
        "/question" => (
            "200 OK",
            if data.awaiting_answer {
                json!([{"sessionID": "ses_old"}])
            } else {
                json!([])
            },
        ),
        "/permission" => ("200 OK", json!([])),
        _ => {
            let Some(id) = path.strip_prefix("/session/") else {
                return ("404 Not Found", json!({}));
            };
            if let Some(parent) = id.strip_suffix("/children") {
                return (
                    "200 OK",
                    json!(
                        data.sessions
                            .values()
                            .filter(|session| session["parentID"] == parent)
                            .collect::<Vec<_>>()
                    ),
                );
            }
            if method == "POST"
                && let Some(id) = id.strip_suffix("/abort")
            {
                return interrupt(data, id);
            }
            if method == "DELETE" {
                if data.delete_failure || data.delete_failure_for.as_deref() == Some(id) {
                    return ("500 Internal Server Error", json!({}));
                }
                data.deleted.push(id.into());
                if !data.delete_lies {
                    remove_tree(data, id);
                }
                return ("200 OK", json!(true));
            }
            match data.sessions.get(id) {
                Some(session) => ("200 OK", session.clone()),
                None => ("404 Not Found", json!({})),
            }
        }
    }
}

fn interrupt(data: &mut Data, id: &str) -> (&'static str, Value) {
    if data.interrupt_failure {
        return ("500 Internal Server Error", json!({}));
    }
    data.interrupted.push(id.to_owned());
    if id == "ses_old" && !data.interrupt_lies {
        data.busy = false;
        data.awaiting_answer = false;
    }
    ("200 OK", json!({"interrupted": true}))
}

fn remove_tree(data: &mut Data, id: &str) {
    let mut queue = vec![id.to_owned()];
    while let Some(id) = queue.pop() {
        queue.extend(
            data.sessions
                .values()
                .filter(|session| session["parentID"] == id)
                .filter_map(|session| session["id"].as_str().map(str::to_owned)),
        );
        data.sessions.remove(&id);
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        self.thread.take().unwrap().join().unwrap();
    }
}
