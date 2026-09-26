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

use crate::store::opencode::{Activity, Client, Pane, Session, Snapshot};
use serde::Deserialize;
use transport::{get, local_server, zellij};

const QUESTION_REFRESH_LIMIT: usize = 16;

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
            .filter(|session| belongs(&session.directory, roots))
            .map(|mut session| {
                session.panes.clear();
                session.stale = true;
                session.activity = Activity::Unknown;
                (session.id.clone(), session)
            })
            .collect();
        let mut errors = BTreeSet::new();
        let mut failed_servers = BTreeSet::new();
        let mut observed_sessions = BTreeSet::new();
        let mut deleted = BTreeSet::new();
        let mut question_refresh = BTreeSet::new();
        for (server, directories) in servers {
            if !directories
                .iter()
                .any(|directory| belongs(directory, roots))
            {
                continue;
            }
            let mut statuses = BTreeMap::new();
            let mut status_error = None;
            for directory in directories
                .iter()
                .filter(|directory| belongs(directory, roots))
            {
                let mut path = reqwest::Url::parse("http://localhost/session/status")
                    .map_err(|error| error.to_string())?;
                path.query_pairs_mut().append_pair("directory", directory);
                let target = format!("{}?{}", path.path(), path.query().unwrap_or_default());
                match get::<BTreeMap<String, RemoteStatus>>(&client, &server, &target).await {
                    Ok(found) => statuses.extend(found),
                    Err(error) => {
                        status_error = Some(error);
                        break;
                    }
                }
            }
            if status_error.is_some() {
                // Port receipts outlive servers. A stopped daemon is normal, not a notification.
                if sessions.values().any(|session| session.server == server) {
                    errors.insert(format!("{server}: OpenCode observation unavailable"));
                }
                failed_servers.insert(server);
                continue;
            }
            match get::<Vec<RemoteSession>>(
                &client,
                &server,
                "/experimental/session?roots=true&limit=1000",
            )
            .await
            {
                Ok(mut remote) => {
                    if remote.len() < 1000 {
                        let present: BTreeSet<_> =
                            remote.iter().map(|session| session.id.as_str()).collect();
                        sessions.retain(|id, session| {
                            session.server != server
                                || !session.stale
                                || present.contains(id.as_str())
                        });
                    }
                    if remote.len() == 1000 {
                        errors.insert(
                            "OpenCode history limited to 1000 recent conversations per server"
                                .into(),
                        );
                    }
                    // Page absence is ambiguous; direct 404s establish deletion even with old receipts.
                    let candidates: BTreeSet<_> = statuses
                        .keys()
                        .cloned()
                        .chain(
                            sessions
                                .values()
                                .filter(|session| session.server == server)
                                .map(|session| session.id.clone()),
                        )
                        .chain(
                            presences
                                .iter()
                                .filter(|presence| {
                                    local_server(&presence.server).as_ref() == Some(&server)
                                })
                                .map(|presence| presence.id.clone()),
                        )
                        .filter(|id| valid_id(id))
                        .collect();
                    for id in &candidates {
                        if !remote.iter().any(|session| session.id == *id) {
                            match transport::get_optional::<RemoteSession>(
                                &client,
                                &server,
                                &format!("/session/{id}"),
                            )
                            .await
                            {
                                Ok(Some(session)) => remote.push(session),
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
                    for item in remote {
                        if item.parent_id.is_some()
                            || !valid_id(&item.id)
                            || !belongs(&item.directory, roots)
                            || !directories.iter().any(|directory| {
                                Path::new(&item.directory).starts_with(directory)
                                    || Path::new(directory).starts_with(&item.directory)
                            })
                        {
                            continue;
                        }
                        let activity = match statuses.get(&item.id).map(|s| s.kind.as_str()) {
                            Some("busy" | "retry") => Activity::Busy,
                            None | Some("idle") => Activity::Idle,
                            _ => Activity::Unknown,
                        };
                        observed_sessions.insert(item.id.clone());
                        let mut candidate = Session {
                            id: item.id.clone(),
                            title: clean(&item.title),
                            directory: item.directory,
                            server: server.clone(),
                            activity,
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
                Err(error) => {
                    errors.insert(error);
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
            if !belongs(&presence.directory, roots) {
                continue;
            }
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
            session.stale = (!presence.zellij_session.is_empty()
                && !panes.contains_key(&presence.zellij_session))
                || failed_servers.contains(&presence.server);
            if !observed_sessions.contains(&session.id) {
                let previous = session.clone();
                session.activity = presence.activity;
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
                    && belongs(directory, roots)
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
            .map(|session| (session.id.clone(), session.live(), session.updated))
            .collect();
        questions.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| b.2.cmp(&a.2)));
        let mut question_tasks = tokio::task::JoinSet::new();
        for (id, _, _) in questions.into_iter().take(QUESTION_REFRESH_LIMIT) {
            let session = sessions.get(&id).expect("question session exists").clone();
            let client = client.clone();
            question_tasks.spawn(async move {
                let question = conversation::latest_question(&client, &session).await;
                (id, question)
            });
        }
        while let Some(Ok((id, Ok(question)))) = question_tasks.join_next().await {
            if let Some(session) = sessions.get_mut(&id) {
                session.last_question = question;
                session.question_observed = true;
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
