mod conversation;
mod navigation;
mod transport;

pub(crate) use conversation::load as conversation;

use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use crate::store::opencode::{Activity, Client, Pane, Session, Snapshot, conversation::LatestTurn};
use serde::Deserialize;
use transport::{get, local_server, zellij};

const QUESTION_REFRESH_LIMIT: usize = 16;
const SESSION_DIRECTORY_WINDOW: usize = 21;

#[derive(Clone)]
pub(crate) struct Observer {
    pub presence: PathBuf,
    pub daemons: PathBuf,
    pub zellij: PathBuf,
}

#[derive(Deserialize)]
struct Presence {
    pid: u32,
    observed_at: u64,
    id: String,
    title: String,
    directory: String,
    server: String,
    activity: Activity,
    context_tokens: Option<u64>,
    context_limit: Option<u64>,
    zellij_session: String,
    pane_id: Option<u32>,
}

#[derive(Deserialize)]
struct RemoteSession {
    id: String,
    title: String,
    directory: String,
    #[serde(rename = "parentID")]
    parent_id: Option<String>,
    time: RemoteTime,
}

#[derive(Deserialize)]
struct RemoteTime {
    updated: u64,
}

#[derive(Deserialize)]
struct RemoteStatus {
    #[serde(rename = "type")]
    kind: String,
}

#[derive(Deserialize)]
struct RemotePane {
    id: u32,
    is_plugin: bool,
    exited: bool,
    tab_id: u32,
    tab_name: String,
    #[serde(default)]
    pane_cwd: Option<String>,
    #[serde(default)]
    pane_command: Option<String>,
}

impl Observer {
    pub fn from_env() -> Self {
        let state = dirs::state_dir().unwrap_or_else(|| std::env::temp_dir().join("tandem-state"));
        Self {
            presence: state.join("tandem/opencode"),
            daemons: std::env::var_os("OC_DAEMON_STATE")
                .map(PathBuf::from)
                .unwrap_or_else(|| state.join("opencode-daemon")),
            zellij: "zellij".into(),
        }
    }

    fn inventory(&self) -> (Vec<Presence>, BTreeMap<String, BTreeSet<String>>) {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;
        let presences = entries(&self.presence)
            .filter(|path| {
                path.extension()
                    .is_some_and(|extension| extension == "json")
            })
            .filter_map(|path| {
                let presence: Presence = serde_json::from_str(&read_small(&path)?).ok()?;
                if now.abs_diff(presence.observed_at) > 10_000
                    || presence.pid == 0
                    || !Path::new(&presence.directory).is_absolute()
                    || (!presence.id.is_empty() && !valid_id(&presence.id))
                    || !Path::new("/proc").join(presence.pid.to_string()).exists()
                {
                    return None;
                }
                Some(presence)
            })
            .collect::<Vec<_>>();
        let mut servers = BTreeMap::<String, BTreeSet<String>>::new();
        for presence in &presences {
            if let Some(server) = local_server(&presence.server) {
                servers
                    .entry(server)
                    .or_default()
                    .insert(presence.directory.clone());
            }
        }
        for directory in entries(&self.daemons) {
            if let Some(port) = read_small(&directory.join("port"))
                .and_then(|text| text.trim().parse::<u16>().ok())
                .filter(|port| *port != 0)
            {
                let directories = servers
                    .entry(format!("http://127.0.0.1:{port}"))
                    .or_default();
                directories.extend(
                    entries(&directory.join("dirs"))
                        .filter(|path| path.extension().is_some_and(|extension| extension == "dir"))
                        .filter_map(|path| read_small(&path))
                        .map(|path| path.trim_end_matches('\n').to_owned())
                        .filter(|path| Path::new(path).is_absolute()),
                );
            }
        }
        (presences, servers)
    }

    pub async fn observe(&self, roots: &[String], previous: Snapshot) -> Result<Snapshot, String> {
        let observed_at = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;
        let observer = self.clone();
        let (presences, mut servers) = tokio::task::spawn_blocking(move || observer.inventory())
            .await
            .map_err(|error| error.to_string())?;
        for session in &previous.sessions {
            if let Some(server) = local_server(&session.server) {
                servers
                    .entry(server)
                    .or_default()
                    .insert(session.directory.clone());
            }
        }
        let client = transport::client()?;
        let mut sessions: BTreeMap<_, _> = previous
            .sessions
            .into_iter()
            .map(|mut session| {
                session.panes.clear();
                session.stale = true;
                session.activity = Activity::Unknown;
                (session.id.clone(), session)
            })
            .collect();
        let known_directories = servers
            .values()
            .flat_map(|directories| directories.iter().cloned())
            .collect::<Vec<_>>();
        let mut errors = BTreeSet::new();
        let mut failed_status_directories = BTreeMap::<String, BTreeSet<String>>::new();
        let mut observed_sessions = BTreeSet::new();
        let mut deleted = BTreeSet::new();
        let mut question_refresh = BTreeSet::new();
        for (server, directories) in servers {
            let mut statuses = BTreeMap::new();
            let mut status_tasks = tokio::task::JoinSet::new();
            for directory in &directories {
                let mut path = reqwest::Url::parse("http://localhost/session/status")
                    .map_err(|error| error.to_string())?;
                path.query_pairs_mut().append_pair("directory", directory);
                let target = format!("{}?{}", path.path(), path.query().unwrap_or_default());
                let client = client.clone();
                let server = server.clone();
                let directory = directory.clone();
                status_tasks.spawn(async move {
                    let result =
                        get::<BTreeMap<String, RemoteStatus>>(&client, &server, &target).await;
                    (directory, result)
                });
            }
            let failed_status = failed_status_directories.entry(server.clone()).or_default();
            while let Some(result) = status_tasks.join_next().await {
                let (directory, result) = result.map_err(|error| error.to_string())?;
                match result {
                    Ok(found) => statuses.extend(found),
                    Err(error) => {
                        // Directory initialization can fail independently on a shared server.
                        failed_status.insert(directory.clone());
                        // Port receipts outlive servers; stopped daemons need no notification.
                        if sessions.values().any(|session| session.server == server) {
                            errors.insert(format!(
                                "{directory}: OpenCode observation unavailable ({error})"
                            ));
                        }
                    }
                }
            }
            if failed_status.len() == directories.len() {
                continue;
            }
            let mut history_tasks = tokio::task::JoinSet::new();
            for directory in &directories {
                let mut path = reqwest::Url::parse("http://localhost/experimental/session")
                    .map_err(|error| error.to_string())?;
                path.query_pairs_mut()
                    .append_pair("roots", "true")
                    .append_pair("limit", &SESSION_DIRECTORY_WINDOW.to_string())
                    .append_pair("directory", directory);
                let target = format!("{}?{}", path.path(), path.query().unwrap_or_default());
                let client = client.clone();
                let server = server.clone();
                let directory = directory.clone();
                history_tasks.spawn(async move {
                    let result = get::<Vec<RemoteSession>>(&client, &server, &target).await;
                    (directory, result)
                });
            }
            let mut remote = BTreeMap::new();
            let mut failed_directories = BTreeSet::new();
            let mut history_task_failed = false;
            while let Some(result) = history_tasks.join_next().await {
                match result {
                    Ok((_directory, Ok(found))) => {
                        for session in found {
                            remote.insert(session.id.clone(), session);
                        }
                    }
                    Ok((directory, Err(error))) => {
                        failed_directories.insert(directory);
                        errors.insert(error);
                    }
                    Err(error) => {
                        history_task_failed = true;
                        errors.insert(error.to_string());
                    }
                }
            }
            let present = remote.keys().cloned().collect::<BTreeSet<_>>();
            sessions.retain(|id, session| {
                session.server != server
                    || history_task_failed
                    || failed_directories.contains(&session.directory)
                    || present.contains(id)
            });
            // Page absence is ambiguous; direct 404s establish deletion even with old receipts.
            let candidates: BTreeSet<_> = statuses
                .keys()
                .cloned()
                .chain(
                    presences
                        .iter()
                        .filter(|presence| local_server(&presence.server).as_ref() == Some(&server))
                        .map(|presence| presence.id.clone()),
                )
                .filter(|id| valid_id(id))
                .collect();
            for id in &candidates {
                if !remote.contains_key(id) {
                    match transport::get_optional::<RemoteSession>(
                        &client,
                        &server,
                        &format!("/session/{id}"),
                    )
                    .await
                    {
                        Ok(Some(session)) => {
                            remote.insert(session.id.clone(), session);
                        }
                        Ok(None) => {
                            if !observed_sessions.contains(id) {
                                sessions.remove(id);
                            }
                            deleted.insert(id.clone());
                        }
                        Err(error) => {
                            errors.insert(error);
                        }
                    }
                }
            }
            for (_, item) in remote {
                if item.parent_id.is_some()
                    || !valid_id(&item.id)
                    || !directories.iter().any(|directory| {
                        Path::new(&item.directory).starts_with(directory)
                            || Path::new(directory).starts_with(&item.directory)
                    })
                {
                    continue;
                }
                let status_failed = failed_status.contains(&item.directory);
                let activity = if status_failed {
                    Activity::Unknown
                } else {
                    match statuses.get(&item.id).map(|s| s.kind.as_str()) {
                        Some("busy" | "retry") => Activity::Busy,
                        None | Some("idle") => Activity::Idle,
                        _ => Activity::Unknown,
                    }
                };
                observed_sessions.insert(item.id.clone());
                let mut candidate = Session {
                    id: item.id.clone(),
                    title: clean(&item.title),
                    directory: item.directory,
                    server: server.clone(),
                    activity,
                    stale: status_failed,
                    updated: item.time.updated,
                    ..Default::default()
                };
                let id = item.id;
                update_activity_timing(&mut candidate, sessions.get(&id), observed_at);
                let current = sessions
                    .entry(id.clone())
                    .or_insert_with(|| candidate.clone());
                let refresh_question =
                    !current.question_observed || candidate.updated > current.updated;
                if current.stale
                    || candidate.activity == Activity::Busy
                    || candidate.activity != current.activity
                    || candidate.updated > current.updated
                {
                    let last_question = current.last_question.take();
                    let question_observed = current.question_observed;
                    *current = candidate;
                    current.last_question = last_question;
                    current.question_observed = question_observed;
                }
                if refresh_question {
                    question_refresh.insert(id);
                }
            }
        }
        let mut panes = BTreeMap::new();
        let names = zellij(
            &self.zellij,
            &[
                "list-sessions".into(),
                "--short".into(),
                "--no-formatting".into(),
            ],
        )
        .await;
        if let Ok(names) = names {
            for name in names.lines().filter(|name| !name.is_empty()).take(64) {
                match self.list_panes(name).await {
                    Ok(found) => {
                        panes.insert(name.to_owned(), found);
                    }
                    Err(error) => {
                        errors.insert(error);
                    }
                }
            }
        } else if !sessions.is_empty() || !presences.is_empty() {
            errors.insert("Zellij unavailable; attachment observation incomplete".into());
            for session in sessions.values_mut() {
                session.stale = true;
            }
        }
        let mut tracked = BTreeSet::new();
        let mut clients = BTreeMap::new();
        for presence in presences {
            if presence.id.is_empty()
                || deleted.contains(&presence.id) && !observed_sessions.contains(&presence.id)
            {
                if let Some(pane) = panes.get(&presence.zellij_session).and_then(|panes| {
                    panes.iter().find(|pane| {
                        Some(pane.id) == presence.pane_id && !pane.is_plugin && !pane.exited
                    })
                }) {
                    tracked.insert((presence.zellij_session.clone(), pane.id));
                    if presence.id.is_empty() {
                        let pane = Pane {
                            session: presence.zellij_session,
                            id: pane.id,
                            tab_id: pane.tab_id,
                            tab_name: clean(&pane.tab_name),
                        };
                        clients.insert(
                            (pane.session.clone(), pane.id),
                            Client {
                                title: clean(&presence.title),
                                directory: presence.directory,
                                server: local_server(&presence.server).unwrap_or_default(),
                                pane,
                                stale: false,
                            },
                        );
                    }
                }
                continue;
            }
            let status_failed = local_server(&presence.server)
                .and_then(|server| failed_status_directories.get(&server))
                .is_some_and(|directories| directories.contains(&presence.directory));
            let session = sessions
                .entry(presence.id.clone())
                .or_insert_with(|| Session {
                    id: presence.id,
                    title: clean(&presence.title),
                    directory: presence.directory,
                    server: local_server(&presence.server).unwrap_or_default(),
                    activity: presence.activity,
                    ..Default::default()
                });
            session.title = clean(&presence.title);
            session.context_tokens = presence.context_tokens;
            session.context_limit = presence.context_limit;
            session.stale = (!presence.zellij_session.is_empty()
                && !panes.contains_key(&presence.zellij_session))
                || status_failed;
            if !observed_sessions.contains(&session.id) {
                let previous = session.clone();
                session.activity = if status_failed {
                    Activity::Unknown
                } else {
                    presence.activity
                };
                update_activity_timing(session, Some(&previous), observed_at);
            }
            if let Some(server) = local_server(&presence.server)
                && (session.server.is_empty() || session.activity != Activity::Busy)
            {
                session.server = server;
            }
            if let Some(pane) = panes.get(&presence.zellij_session).and_then(|panes| {
                panes.iter().find(|pane| {
                    Some(pane.id) == presence.pane_id && !pane.is_plugin && !pane.exited
                })
            }) {
                tracked.insert((presence.zellij_session.clone(), pane.id));
                let pane = Pane {
                    session: presence.zellij_session,
                    id: pane.id,
                    tab_id: pane.tab_id,
                    tab_name: clean(&pane.tab_name),
                };
                if !session.panes.contains(&pane) {
                    session.panes.push(pane);
                }
            }
        }
        for (name, panes) in panes {
            for pane in panes.iter().filter(|pane| {
                !pane.is_plugin && !pane.exited && !tracked.contains(&(name.clone(), pane.id))
            }) {
                if pane.pane_command.as_ref().is_some_and(|command| {
                    command.contains("opencode") || command.contains("oc-pane")
                }) && let Some(directory) = &pane.pane_cwd
                    && belongs(directory, &known_directories)
                {
                    errors.insert("OpenCode panes need the Tandem TUI companion; run tandem opencode-setup and reopen those clients".into());
                    for session in sessions
                        .values_mut()
                        .filter(|session| session.directory == *directory)
                    {
                        session.stale = true;
                    }
                }
            }
        }
        let mut questions: Vec<_> = sessions
            .values()
            .filter(|session| question_refresh.contains(&session.id) || !session.question_observed)
            .map(|session| {
                (
                    session.id.clone(),
                    belongs(&session.directory, roots),
                    session.live(),
                    session.updated,
                )
            })
            .collect();
        questions.sort_by(|a, b| {
            b.1.cmp(&a.1)
                .then_with(|| b.2.cmp(&a.2))
                .then_with(|| b.3.cmp(&a.3))
        });
        let mut question_tasks = tokio::task::JoinSet::new();
        for (id, _, _, _) in questions.into_iter().take(QUESTION_REFRESH_LIMIT) {
            let session = sessions.get(&id).expect("question session exists").clone();
            let client = client.clone();
            question_tasks.spawn(async move {
                let turn = conversation::latest_turn(&client, &session).await;
                (id, turn)
            });
        }
        while let Some(Ok((id, Ok(turn)))) = question_tasks.join_next().await {
            if let Some(session) = sessions.get_mut(&id) {
                session.question_observed = true;
                if let Some(turn) = turn {
                    session.last_question = turn.question.clone();
                    apply_turn_timing(session, &turn, observed_at);
                }
            }
        }
        Ok(Snapshot {
            sessions: sessions.into_values().collect(),
            clients: clients.into_values().collect(),
            error: (!errors.is_empty()).then(|| errors.into_iter().collect::<Vec<_>>().join("\n")),
        })
    }
}

fn entries(path: &Path) -> impl Iterator<Item = PathBuf> {
    fs::read_dir(path)
        .into_iter()
        .flatten()
        .filter_map(Result::ok)
        .take(4096)
        .map(|entry| entry.path())
}

fn read_small(path: &Path) -> Option<String> {
    if !fs::symlink_metadata(path).ok()?.is_file() {
        return None;
    }
    let mut text = String::new();
    fs::File::open(path)
        .ok()?
        .take(16_385)
        .read_to_string(&mut text)
        .ok()?;
    (text.len() <= 16_384).then_some(text)
}

fn belongs(directory: &str, roots: &[String]) -> bool {
    crate::store::opencode::workspace_owner(
        directory,
        roots.iter().map(|root| (root.as_str(), root.as_str())),
    )
    .is_some()
}

fn update_activity_timing(candidate: &mut Session, previous: Option<&Session>, observed_at: u64) {
    match candidate.activity {
        Activity::Busy => {
            // Refresh invalidates activity before merging; the run-start marker survives it.
            let started = previous
                .and_then(|session| session.activity_started_at_milliseconds)
                .unwrap_or_else(|| {
                    let age = observed_at.saturating_sub(candidate.updated);
                    if candidate.updated <= observed_at && age <= 86_400_000 {
                        candidate.updated
                    } else {
                        observed_at
                    }
                });
            candidate.activity_started_at_milliseconds = Some(started);
            candidate.activity_elapsed_milliseconds = Some(observed_at.saturating_sub(started));
        }
        Activity::Idle => {
            candidate.activity_started_at_milliseconds = None;
            candidate.activity_elapsed_milliseconds = previous.and_then(|session| {
                session
                    .activity_started_at_milliseconds
                    .map(|started| observed_at.saturating_sub(started))
                    .or(session.activity_elapsed_milliseconds)
            });
        }
        Activity::Unknown => {
            candidate.activity_started_at_milliseconds =
                previous.and_then(|session| session.activity_started_at_milliseconds);
            candidate.activity_elapsed_milliseconds =
                previous.and_then(|session| session.activity_elapsed_milliseconds);
        }
    }
}

fn apply_turn_timing(session: &mut Session, turn: &LatestTurn, observed_at: u64) {
    let started = turn.started_at;
    if started > observed_at || observed_at.saturating_sub(started) > 86_400_000 {
        return;
    }
    match session.activity {
        Activity::Busy => {
            session.activity_started_at_milliseconds = Some(started);
            session.activity_elapsed_milliseconds = Some(observed_at.saturating_sub(started));
        }
        Activity::Idle => {
            let completed = turn.completed_at.unwrap_or(session.updated);
            if completed >= started && completed <= observed_at {
                session.activity_started_at_milliseconds = None;
                session.activity_elapsed_milliseconds = Some(completed - started);
            }
        }
        Activity::Unknown => {}
    }
}

fn valid_id(value: &str) -> bool {
    value.starts_with("ses_")
        && value.len() < 128
        && value
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || c == b'_')
}

fn clean(text: &str) -> String {
    text.chars().filter(|c| !c.is_control()).take(160).collect()
}

pub(crate) fn install(home: &Path) -> Result<String, String> {
    let directory = home.join("opencode-plugin");
    fs::create_dir_all(&directory).map_err(|error| error.to_string())?;
    if !fs::canonicalize(&directory)
        .map_err(|error| error.to_string())?
        .starts_with(home)
    {
        return Err("OpenCode companion directory escapes TANDEM_HOME".into());
    }
    super::config::private_file(&directory.join("bridge.mjs"), false)
        .and_then(|mut file| file.write_all(include_bytes!("bridge.mjs")))
        .map_err(|error| error.to_string())?;
    Ok(format!(
        "Add this entry to the plugin array in your OpenCode tui.json, then reopen OpenCode clients:\n{}",
        serde_json::to_string(&directory.join("bridge.mjs").display().to_string())
            .map_err(|error| error.to_string())?
    ))
}

#[cfg(test)]
#[path = "tests/mod.rs"]
mod tests;
