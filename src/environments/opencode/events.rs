use std::{
    collections::BTreeMap,
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU8, Ordering},
    },
    time::Duration,
};

use notify::{RecursiveMode, Watcher};
use tokio::sync::Notify;

use super::{Observer, transport};
use crate::store::opencode::Snapshot;

pub(crate) const LOCAL: u8 = 1;
pub(crate) const REMOTE: u8 = 2;

#[derive(Default)]
pub(crate) struct Signal {
    flags: AtomicU8,
    ready: Notify,
    errors: Mutex<BTreeMap<String, String>>,
}

impl Signal {
    pub(crate) fn send(&self, flags: u8) {
        self.flags.fetch_or(flags, Ordering::Release);
        self.ready.notify_one();
    }

    pub(crate) async fn wait(&self) -> u8 {
        loop {
            self.ready.notified().await;
            let flags = self.flags.swap(0, Ordering::AcqRel);
            if flags != 0 {
                return flags;
            }
        }
    }

    pub(crate) fn errors(&self) -> BTreeMap<String, String> {
        self.errors
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .clone()
    }

    fn connection(&self, server: &str, error: Option<String>) {
        let mut errors = self
            .errors
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        match error {
            Some(error) => {
                errors.insert(server.to_owned(), error);
            }
            None => {
                errors.remove(server);
            }
        }
        self.send(REMOTE);
    }
}

pub(crate) struct Changes {
    _watcher: notify::RecommendedWatcher,
    streams: BTreeMap<String, tokio::task::JoinHandle<()>>,
    signal: Arc<Signal>,
    signature: Vec<String>,
}

impl Changes {
    pub(crate) fn new(observer: &Observer, signal: Arc<Signal>) -> Result<Self, String> {
        let presence = observer.presence.clone();
        let daemons = observer.daemons.clone();
        let changed = Arc::clone(&signal);
        let mut watcher = notify::recommended_watcher(
            move |result: notify::Result<notify::Event>| match result {
                Ok(event) if !event.kind.is_access() => {
                    if event.need_rescan() {
                        changed.send(REMOTE | LOCAL);
                    } else if event
                        .paths
                        .iter()
                        .any(|path| path.starts_with(&daemons) || daemons.starts_with(path))
                    {
                        changed.send(REMOTE);
                    } else if event.paths.iter().any(|path| {
                        presence.starts_with(path)
                            || path.starts_with(&presence)
                                && path.extension().is_some_and(|ext| ext == "json")
                    }) {
                        changed.send(LOCAL);
                    }
                }
                Err(_) => changed.send(REMOTE | LOCAL),
                _ => {}
            },
        )
        .map_err(|error| format!("Cannot watch OpenCode receipts: {error}"))?;
        let mut parents = Vec::<PathBuf>::new();
        for path in [&observer.presence, &observer.daemons] {
            let mut parent = path
                .parent()
                .ok_or("OpenCode receipt directory has no parent")?;
            while !parent.is_dir() {
                parent = parent
                    .parent()
                    .ok_or("OpenCode receipt parent is unavailable")?;
            }
            if !parents.iter().any(|existing| parent.starts_with(existing)) {
                parents.retain(|existing| !existing.starts_with(parent));
                parents.push(parent.to_owned());
            }
        }
        for parent in parents {
            watcher
                .watch(&parent, RecursiveMode::Recursive)
                .map_err(|error| format!("Cannot watch OpenCode receipts: {error}"))?;
        }
        Ok(Self {
            _watcher: watcher,
            streams: BTreeMap::new(),
            signal,
            signature: Vec::new(),
        })
    }

    pub(crate) async fn sync(
        &mut self,
        observer: &Observer,
        snapshot: &Snapshot,
    ) -> Result<bool, String> {
        let observer = observer.clone();
        let (presences, mut servers) = tokio::task::spawn_blocking(move || observer.inventory())
            .await
            .map_err(|error| error.to_string())?;
        for session in &snapshot.sessions {
            if let Some(server) = transport::local_server(&session.server) {
                servers.entry(server).or_default();
            }
        }
        self.streams.retain(|server, task| {
            if servers.contains_key(server) {
                true
            } else {
                task.abort();
                self.signal
                    .errors
                    .lock()
                    .unwrap_or_else(|error| error.into_inner())
                    .remove(server);
                false
            }
        });
        for server in servers.keys() {
            if !self.streams.contains_key(server) {
                let signal = Arc::clone(&self.signal);
                let address = server.clone();
                self.streams.insert(
                    server.clone(),
                    tokio::spawn(async move {
                        let mut delay = 1;
                        loop {
                            let result = subscribe(&address, &signal).await;
                            // A disconnected source is uncertain, not proof of idle or closure.
                            signal.connection(
                                &address,
                                Some("OpenCode live updates disconnected; reconnecting".into()),
                            );
                            tokio::time::sleep(Duration::from_secs(delay)).await;
                            delay = if result.is_ok() {
                                1
                            } else {
                                (delay * 2).min(30)
                            };
                        }
                    }),
                );
            }
        }
        let mut signature = presences
            .into_iter()
            .map(|presence| {
                let mut value =
                    serde_json::to_value(presence).expect("presence contains JSON values");
                value
                    .as_object_mut()
                    .expect("presence is an object")
                    .remove("observed_at");
                value.to_string()
            })
            .collect::<Vec<_>>();
        signature.sort();
        let changed = signature != self.signature;
        self.signature = signature;
        Ok(changed)
    }
}

impl Drop for Changes {
    fn drop(&mut self) {
        for task in self.streams.values() {
            task.abort();
        }
    }
}

async fn subscribe(server: &str, signal: &Signal) -> Result<(), String> {
    let probe = transport::client()?;
    let v2 = transport::is_v2(&probe, server).await?;
    let path = if v2 { "/api/event" } else { "/global/event" };
    let client = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(2))
        .redirect(reqwest::redirect::Policy::none())
        .no_proxy()
        .build()
        .map_err(|error| error.to_string())?;
    let mut request = client.get(format!("{server}{path}"));
    if let Some(password) = transport::password(server) {
        request = request.basic_auth(
            std::env::var("OPENCODE_SERVER_USERNAME").unwrap_or_else(|_| "opencode".into()),
            Some(password),
        );
    }
    let mut response = tokio::time::timeout(Duration::from_secs(3), request.send())
        .await
        .map_err(|_| "OpenCode event connection timed out")?
        .map_err(|_| "OpenCode event connection unavailable")?
        .error_for_status()
        .map_err(|_| "OpenCode event subscription refused")?;
    // Subscribe before requesting a baseline so events racing that read stay queued.
    signal.connection(server, None);
    let mut decoder = Decoder::default();
    loop {
        let chunk = tokio::time::timeout(Duration::from_secs(45), response.chunk())
            .await
            .map_err(|_| "OpenCode event stream timed out")?
            .map_err(|_| "OpenCode event stream failed")?;
        let Some(chunk) = chunk else {
            return Ok(());
        };
        if decoder.push(&chunk)? {
            signal.send(REMOTE);
        }
    }
}

#[derive(Default)]
pub(super) struct Decoder {
    pending: Vec<u8>,
    data: Vec<u8>,
}

impl Decoder {
    pub(super) fn push(&mut self, chunk: &[u8]) -> Result<bool, String> {
        if self.pending.len() + self.data.len() + chunk.len() > 1_048_576 {
            return Err("OpenCode event exceeds 1 MiB".into());
        }
        self.pending.extend_from_slice(chunk);
        let mut changed = false;
        let mut consumed = 0;
        while let Some(end) = self.pending[consumed..]
            .iter()
            .position(|byte| *byte == b'\n')
        {
            let end = consumed + end;
            let line = self.pending[consumed..end]
                .strip_suffix(b"\r")
                .unwrap_or(&self.pending[consumed..end]);
            if line.is_empty() {
                if !self.data.is_empty() {
                    let mut value: serde_json::Value =
                        serde_json::from_slice(&self.data).map_err(|_| "Invalid OpenCode event")?;
                    if let Some(text) = value.as_str() {
                        value = serde_json::from_str(text).map_err(|_| "Invalid OpenCode event")?;
                    }
                    let event = value.get("payload").unwrap_or(&value);
                    if let Some(kind) = event["type"].as_str() {
                        changed |= relevant(kind);
                    }
                    self.data.clear();
                }
            } else if let Some(data) = line.strip_prefix(b"data:") {
                if !self.data.is_empty() {
                    self.data.push(b'\n');
                }
                self.data
                    .extend_from_slice(data.strip_prefix(b" ").unwrap_or(data));
            } else if line == b"event: effect/httpapi/stream/failure" {
                return Err("OpenCode event stream overflowed".into());
            }
            consumed = end + 1;
        }
        self.pending.drain(..consumed);
        Ok(changed)
    }
}

fn relevant(kind: &str) -> bool {
    if kind.ends_with(".delta") {
        return false;
    }
    kind == "server.connected"
        || kind.starts_with("session.")
        || kind.starts_with("permission.")
        || kind.starts_with("form.")
        || kind.starts_with("question.")
        || kind == "message.updated"
        || kind == "message.removed"
}
