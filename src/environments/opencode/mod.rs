pub(crate) mod cleanup;
mod conversation;
mod discovery;
pub(crate) mod events;
mod history;
mod navigation;
mod order;
mod prompted;
mod purge;
mod resources;
mod rule_destination;
mod server;
mod sessions;
mod tabs;
mod transport;
mod v2;

pub(crate) use conversation::load as conversation;
pub(crate) use history::clear as clear_history;
pub(crate) use sessions::rename;

use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use crate::store::opencode::observation::{Evidence, Exclusions, Failure};
use crate::store::opencode::{Activity, Client, Pane, Session, Snapshot, conversation::LatestTurn};
use serde::Deserialize;
use transport::{get, local_server, zellij};

const QUESTION_REFRESH_LIMIT: usize = 16;
pub(crate) const SESSION_DIRECTORY_WINDOW: usize = 21;
const CLIENT_STARTUP_GRACE_MILLISECONDS: u64 = 10_000;

#[derive(Clone)]
pub(crate) struct Observer {
    pub presence: PathBuf,
    pub daemons: PathBuf,
    pub zellij: PathBuf,
    pub excluded: Exclusions,
}

#[derive(Deserialize, serde::Serialize)]
struct Presence {
    pid: u32,
    observed_at: u64,
    id: String,
    title: String,
    directory: String,
    server: String,
    tab_control: Option<tabs::Control>,
    last_question: Option<String>,
    activity: Activity,
    agent: Option<String>,
    agent_color: Option<String>,
    model: Option<String>,
    model_name: Option<String>,
    provider_name: Option<String>,
    variant: Option<String>,
    context_tokens: Option<u64>,
    context_limit: Option<u64>,
    zellij_session: String,
    pane_id: Option<u32>,
    #[serde(default)]
    tabs: Vec<Presence>,
    #[serde(default = "presence_active")]
    active: bool,
    tab_index: Option<usize>,
}

fn presence_active() -> bool {
    true
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
struct RemoteQuestion {
    #[serde(rename = "sessionID")]
    session_id: String,
}

#[derive(Deserialize)]
struct RemotePane {
    id: u32,
    is_plugin: bool,
    exited: bool,
    tab_id: u32,
    tab_name: String,
    #[serde(default)]
    is_floating: bool,
    #[serde(default)]
    pane_cwd: Option<String>,
    #[serde(default)]
    pane_command: Option<String>,
    #[serde(default)]
    terminal_command: Option<String>,
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
            excluded: Exclusions::default(),
        }
    }

    fn presences(&self) -> Vec<Presence> {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;
        entries(&self.presence)
            .filter(|path| {
                path.extension()
                    .is_some_and(|extension| extension == "json")
            })
            .filter_map(|path| {
                let presence: Presence =
                    serde_json::from_str(&read_bounded(&path, 1_048_576)?).ok()?;
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
            .flat_map(|mut presence| {
                let tabs = std::mem::take(&mut presence.tabs)
                    .into_iter()
                    .filter(|tab| valid_id(&tab.id) && Path::new(&tab.directory).is_absolute())
                    .map(|mut tab| {
                        tab.pid = presence.pid;
                        tab.observed_at = presence.observed_at;
                        tab.server = presence.server.clone();
                        tab.zellij_session = presence.zellij_session.clone();
                        tab.pane_id = presence.pane_id;
                        tab.tab_control = presence.tab_control.clone();
                        tab.tabs.clear();
                        tab.active = false;
                        tab
                    })
                    .collect::<Vec<_>>();
                std::iter::once(presence).chain(tabs)
            })
            .collect()
    }

    fn inventory(&self) -> (Vec<Presence>, BTreeMap<String, BTreeSet<String>>) {
        let presences = self.presences();
        let listening = discovery::listening_ports();
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
                let server = format!("http://127.0.0.1:{port}");
                if !servers.contains_key(&server) && !listening.contains(&port) {
                    continue;
                }
                let directories = servers.entry(server).or_default();
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

    fn observed_presences(&self) -> Vec<Presence> {
        self.presences()
            .into_iter()
            .filter(|presence| {
                !self.excluded.sessions.contains(&presence.id)
                    && !self.excluded.servers.contains(&presence.server)
                    && !presence.pane_id.is_some_and(|id| {
                        self.excluded
                            .panes
                            .contains(&(presence.zellij_session.clone(), id))
                    })
            })
            .collect()
    }

    pub async fn observe(&self, roots: &[String], previous: Snapshot) -> Result<Snapshot, String> {
        let mut snapshot = self.observe_changes(roots, previous, true).await?;
        snapshot.resources = self.sample_resources(snapshot.resources).await?;
        Ok(snapshot)
    }

    pub(crate) async fn observe_changes(
        &self,
        roots: &[String],
        previous: Snapshot,
        remote: bool,
    ) -> Result<Snapshot, String> {
        let observed_at = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;
        let observer = self.clone();
        let mut directories = previous
            .workspace_directories()
            .map(str::to_owned)
            .collect::<BTreeSet<_>>();
        let previous_resources = previous.resources;
        let mut unfinished_sessions = previous.observation.unfinished_sessions;
        for session in &previous.sessions {
            if matches!(session.activity, Activity::Busy | Activity::AwaitingAnswer)
                || session.approval_pending == Some(true)
            {
                unfinished_sessions.insert(session.id.clone());
            } else if !session.stale
                && session.activity == Activity::Idle
                && session.approval_pending == Some(false)
            {
                unfinished_sessions.remove(&session.id);
            }
        }
        let previous_sessions = previous.sessions.clone();
        let (presences, servers) = tokio::task::spawn_blocking(move || {
            observer.inventory_with_sessions(&previous_sessions)
        })
        .await
        .map_err(|error| error.to_string())?;
        directories.extend(presences.iter().map(|presence| presence.directory.clone()));
        let client = transport::client()?;
        let servers_for_questions = servers.keys().cloned().collect::<BTreeSet<_>>();
        let previous_panes: BTreeMap<_, _> = previous
            .sessions
            .iter()
            .flat_map(|session| &session.panes)
            .chain(previous.clients.iter().map(|client| &client.pane))
            .map(|pane| ((pane.session.clone(), pane.id), pane.clone()))
            .collect();
        let previously_attached: BTreeSet<_> = previous
            .sessions
            .iter()
            .flat_map(|session| &session.panes)
            .map(|pane| (pane.session.clone(), pane.id))
            .collect();
        let mut sessions: BTreeMap<_, _> = previous
            .sessions
            .into_iter()
            .filter(|session| {
                !self.excluded.sessions.contains(&session.id)
                    && !self.excluded.servers.contains(&session.server)
            })
            .map(|mut session| {
                session.panes.clear();
                session.tab_position = None;
                if remote && servers.contains_key(&session.server) {
                    session.stale = true;
                    session.activity = Activity::Unknown;
                    session.approval_pending = None;
                }
                (session.id.clone(), session)
            })
            .collect();
        let known_directories = servers
            .values()
            .flat_map(|directories| directories.iter().cloned())
            .collect::<Vec<_>>();
        directories.extend(known_directories.iter().cloned());
        let mut errors = Vec::new();
        let mut failed_status_directories = BTreeMap::<String, BTreeSet<String>>::new();
        let mut observed_sessions = BTreeSet::new();
        let mut deleted = BTreeSet::new();
        let mut question_refresh = BTreeSet::new();
        let mut server_tasks = tokio::task::JoinSet::new();
        for (server, directories) in servers {
            if !remote {
                continue;
            }
            let candidates = presences
                .iter()
                .filter(|presence| local_server(&presence.server).as_ref() == Some(&server))
                .map(|presence| presence.id.clone())
                .collect();
            let client = client.clone();
            // Unfinished servers stay stale even if their worker fails.
            failed_status_directories.insert(server.clone(), directories.clone());
            server_tasks.spawn(async move {
                let result = server::observe(client, &server, &directories, candidates).await;
                (server, directories, result)
            });
        }
        let mut observations = BTreeMap::new();
        while let Some(result) = server_tasks.join_next().await {
            let (server, directories, observation) = match result {
                Ok(result) => result,
                Err(error) => {
                    errors.push(Failure::global(error.to_string()));
                    continue;
                }
            };
            observations.insert(server, (directories, observation));
        }
        for (server, (directories, observation)) in observations {
            let failed_status = failed_status_directories.entry(server.clone()).or_default();
            *failed_status = observation.failed_status;
            errors.extend(observation.errors);
            sessions.retain(|id, session| {
                session.server != server
                    || failed_status.contains(&session.directory)
                    || observation.failed_history.contains(&session.directory)
                    || observation.sessions.contains_key(id)
            });
            for id in observation.deleted {
                if !observed_sessions.contains(&id) {
                    sessions.remove(&id);
                }
                deleted.insert(id);
            }
            for (_, item) in observation.sessions {
                if item.parent_id.is_some()
                    || self.excluded.sessions.contains(&item.id)
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
                } else if observation.questions.contains(&item.id) {
                    Activity::AwaitingAnswer
                } else {
                    match observation.statuses.get(&item.id).map(|s| s.kind.as_str()) {
                        Some("busy" | "retry") => Activity::Busy,
                        None | Some("idle") => Activity::Idle,
                        _ => Activity::Unknown,
                    }
                };
                observed_sessions.insert(item.id.clone());
                let mut candidate = Session {
                    id: item.id.clone(),
                    title: clean(&item.title),
                    approval_pending: observation
                        .approval_directories
                        .contains(&item.directory)
                        .then(|| observation.approvals.contains(&item.id)),
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
                current.approval_pending = candidate.approval_pending;
                if current.stale
                    || candidate.activity == Activity::Busy
                    || candidate.activity != current.activity
                    || candidate.updated > current.updated
                {
                    // Status/history responses do not carry conversation metadata.
                    current.title = candidate.title;
                    current.directory = candidate.directory;
                    current.server = candidate.server;
                    current.activity = candidate.activity;
                    current.stale = candidate.stale;
                    current.updated = candidate.updated;
                    current.activity_started_at_milliseconds =
                        candidate.activity_started_at_milliseconds;
                    current.activity_elapsed_milliseconds = candidate.activity_elapsed_milliseconds;
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
        let listed_sessions = if let Ok(names) = names {
            let names = names
                .lines()
                .filter(|name| !name.is_empty())
                .take(64)
                .map(str::to_owned)
                .collect::<BTreeSet<_>>();
            let mut pane_tasks = tokio::task::JoinSet::new();
            for name in &names {
                let observer = self.clone();
                let name = name.to_owned();
                pane_tasks.spawn(async move {
                    let result = observer.list_panes(&name).await;
                    (name, result)
                });
            }
            while let Some(result) = pane_tasks.join_next().await {
                match result {
                    Ok((name, Ok(found))) => {
                        panes.insert(name, found);
                    }
                    Ok((name, Err(error))) => {
                        for (session, id) in previous_panes
                            .keys()
                            .filter(|(session, _)| *session == name)
                        {
                            errors.push(Failure::pane(session, *id, error.clone()));
                        }
                        for presence in &presences {
                            if presence.zellij_session == name
                                && let Some(id) = presence.pane_id
                            {
                                errors.push(Failure::pane(&name, id, error.clone()));
                            }
                        }
                    }
                    Err(error) => {
                        errors.push(Failure::global(error.to_string()));
                    }
                }
            }
            Some(names)
        } else {
            if !sessions.is_empty() || !presences.is_empty() {
                for (session, id) in previous_panes.keys() {
                    errors.push(Failure::pane(
                        session,
                        *id,
                        "Zellij unavailable; attachment observation incomplete",
                    ));
                }
                for presence in &presences {
                    if let Some(id) = presence.pane_id {
                        errors.push(Failure::pane(
                            &presence.zellij_session,
                            id,
                            "Zellij unavailable; attachment observation incomplete",
                        ));
                    }
                }
                for session in sessions.values_mut() {
                    session.stale = true;
                }
            }
            None
        };
        let attached_pane = |presence: &Presence| {
            let id = presence.pane_id?;
            match panes.get(&presence.zellij_session) {
                Some(panes) => panes
                    .iter()
                    .find(|pane| pane.id == id && !pane.is_plugin && !pane.exited)
                    .map(|pane| Pane {
                        session: presence.zellij_session.clone(),
                        id: pane.id,
                        tab_id: pane.tab_id,
                        tab_name: clean(&pane.tab_name),
                    }),
                // A fresh receipt confirms the route; a failed query cannot prove closure.
                None if listed_sessions
                    .as_ref()
                    .is_none_or(|names| names.contains(&presence.zellij_session)) =>
                {
                    previous_panes
                        .get(&(presence.zellij_session.clone(), id))
                        .cloned()
                }
                None => None,
            }
        };
        // Server queries can outlast a route switch. Join attachment and resource
        // identity from fresh receipts rather than the discovery-time home route.
        let observer = self.clone();
        let (presences, mut resources) = tokio::task::spawn_blocking(move || {
            let presences = observer.observed_presences();
            let resources = previous_resources;
            (presences, resources)
        })
        .await
        .map_err(|error| error.to_string())?;
        resources.retain_mut(|process| {
            let Some(presence) = presences
                .iter()
                .filter(|presence| presence.active && presence.pid == process.pid)
                .max_by_key(|presence| presence.observed_at)
            else {
                return false;
            };
            process.session_id = presence.id.clone();
            process.directory = presence.directory.clone();
            process.zellij_session = presence.zellij_session.clone();
            process.pane_id = presence.pane_id;
            true
        });
        let mut tracked = BTreeSet::new();
        let mut verified_panes = BTreeSet::new();
        let mut clients = BTreeMap::new();
        for presence in presences {
            if presence.id.is_empty()
                || deleted.contains(&presence.id) && !observed_sessions.contains(&presence.id)
            {
                if let Some(pane) = attached_pane(&presence) {
                    tracked.insert((presence.zellij_session.clone(), pane.id));
                    if panes.contains_key(&pane.session) {
                        verified_panes.insert((pane.session.clone(), pane.id));
                    }
                    if presence.id.is_empty() {
                        let stale = !panes.contains_key(&pane.session);
                        clients.insert(
                            (pane.session.clone(), pane.id),
                            Client {
                                title: clean(&presence.title),
                                directory: presence.directory,
                                server: local_server(&presence.server).unwrap_or_default(),
                                pane,
                                stale,
                                awaiting_presence_since: None,
                            },
                        );
                    }
                }
                continue;
            }
            let status_failed = local_server(&presence.server)
                .and_then(|server| failed_status_directories.get(&server))
                .is_some_and(|directories| directories.contains(&presence.directory));
            let pane = attached_pane(&presence);
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
            if matches!(presence.activity, Activity::Busy | Activity::AwaitingAnswer) {
                unfinished_sessions.insert(session.id.clone());
            }
            session.title = clean(&presence.title);
            if let Some(question) = &presence.last_question {
                let question = question
                    .chars()
                    .filter(|c| !c.is_control() || matches!(c, '\n' | '\t'))
                    .collect::<String>()
                    .split_whitespace()
                    .collect::<Vec<_>>()
                    .join(" ")
                    .chars()
                    .take(4096)
                    .collect::<String>();
                if !question.is_empty() {
                    session.last_question = Some(question);
                    session.question_observed = true;
                }
            }
            if presence.agent.is_some() {
                session.agent = presence.agent.clone();
            }
            if presence.agent_color.is_some() {
                session.agent_color = presence.agent_color.clone();
            }
            if presence.model.is_some() {
                session.model = presence.model.clone();
            }
            if presence.model_name.is_some() {
                session.model_name = presence.model_name.clone();
            }
            if presence.provider_name.is_some() {
                session.provider_name = presence.provider_name.clone();
            }
            if presence.variant.is_some() {
                session.variant = presence.variant.clone();
            }
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
            if let Some(pane) = pane {
                if panes.contains_key(&pane.session) {
                    verified_panes.insert((pane.session.clone(), pane.id));
                }
                if let Some(index) = presence.tab_index {
                    let position = crate::store::opencode::TabPosition {
                        zellij_session: pane.session.clone(),
                        pane_id: pane.id,
                        index,
                    };
                    if session
                        .tab_position
                        .as_ref()
                        .is_none_or(|current| position < *current)
                    {
                        session.tab_position = Some(position);
                    }
                }
                tracked.insert((presence.zellij_session.clone(), pane.id));
                if !session.panes.contains(&pane) {
                    session.panes.push(pane);
                }
            }
        }
        for (name, panes) in panes {
            for pane in panes.iter().filter(|pane| {
                !pane.is_plugin
                    && !pane.exited
                    && !tracked.contains(&(name.clone(), pane.id))
                    && !self.excluded.panes.contains(&(name.clone(), pane.id))
            }) {
                if pane.pane_command.as_ref().is_some_and(|command| {
                    command.contains("opencode") || command.contains("oc-pane")
                }) && let Some(directory) = &pane.pane_cwd
                    && (belongs(directory, &known_directories) || belongs(directory, roots))
                {
                    // Pane discovery can precede the companion's first receipt. A known
                    // client losing its receipt is a failure, not another startup.
                    let since = previous
                        .clients
                        .iter()
                        .find(|client| client.pane.session == name && client.pane.id == pane.id)
                        .map(|client| client.awaiting_presence_since.unwrap_or(0))
                        .unwrap_or_else(|| {
                            if previously_attached.contains(&(name.clone(), pane.id)) {
                                0
                            } else {
                                observed_at
                            }
                        });
                    let stale =
                        observed_at.saturating_sub(since) >= CLIENT_STARTUP_GRACE_MILLISECONDS;
                    if stale {
                        errors.push(Failure::pane(&name, pane.id, "OpenCode panes need the Tandem TUI companion; run tandem opencode-setup and reopen those clients"));
                    }
                    clients.insert(
                        (name.clone(), pane.id),
                        Client {
                            title: "OpenCode".into(),
                            directory: directory.clone(),
                            server: String::new(),
                            pane: Pane {
                                session: name.clone(),
                                id: pane.id,
                                tab_id: pane.tab_id,
                                tab_name: clean(&pane.tab_name),
                            },
                            stale,
                            awaiting_presence_since: Some(since),
                        },
                    );
                }
            }
        }
        let mut questions: Vec<_> = sessions
            .values()
            .filter(|_| remote)
            .filter(|session| servers_for_questions.contains(&session.server))
            .filter(|session| question_refresh.contains(&session.id) || !session.question_observed)
            .filter(|session| !session.saved())
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
                let turn = tokio::time::timeout(
                    std::time::Duration::from_secs(2),
                    conversation::latest_turn(&client, &session),
                )
                .await
                .unwrap_or_else(|_| Err("OpenCode question observation timed out".into()));
                (id, turn)
            });
        }
        while let Some(result) = question_tasks.join_next().await {
            let Ok((id, Ok(turn))) = result else {
                continue;
            };
            if let Some(session) = sessions.get_mut(&id) {
                session.question_observed = true;
                if let Some(turn) = turn {
                    session.last_question = turn.question.clone();
                    if session.agent != turn.agent || turn.agent_color.is_some() {
                        session.agent_color = turn.agent_color.clone();
                    }
                    session.agent = turn.agent.clone();
                    let model = turn
                        .provider
                        .as_ref()
                        .zip(turn.model.as_ref())
                        .map(|(provider, model)| format!("{provider}/{model}"));
                    if session.model != model || session.model_name.is_none() {
                        session.model_name = turn.model_name.clone().or(turn.model.clone());
                        session.provider_name =
                            turn.provider_name.clone().or(turn.provider.clone());
                    }
                    session.model = model;
                    session.variant = turn.variant.clone();
                    apply_turn_timing(session, &turn, observed_at);
                }
            }
        }
        resources.retain(|process| {
            if process.session_id.is_empty() {
                process.pane_id.is_some_and(|pane| {
                    clients.contains_key(&(process.zellij_session.clone(), pane))
                })
            } else {
                sessions.get(&process.session_id).is_some_and(|session| {
                    process.pane_id.is_none_or(|id| {
                        session
                            .panes
                            .iter()
                            .any(|pane| pane.id == id && pane.session == process.zellij_session)
                    })
                })
            }
        });
        directories.extend(sessions.values().map(|session| session.directory.clone()));
        directories.extend(clients.values().map(|client| client.directory.clone()));
        for session in sessions.values() {
            if !session.stale {
                if matches!(session.activity, Activity::Busy | Activity::AwaitingAnswer)
                    || session.approval_pending == Some(true)
                {
                    unfinished_sessions.insert(session.id.clone());
                } else if session.activity == Activity::Idle
                    && (session.approval_pending == Some(false) || session.server.is_empty())
                {
                    unfinished_sessions.remove(&session.id);
                }
            }
        }
        unfinished_sessions.retain(|id| sessions.contains_key(id));
        let checked_directories = directories.clone();
        let missing_directories = tokio::task::spawn_blocking(move || {
            checked_directories
                .into_iter()
                .filter(|directory| matches!(Path::new(directory).try_exists(), Ok(false)))
                .collect()
        })
        .await
        .map_err(|error| error.to_string())?;
        let mut snapshot = Snapshot {
            directories: directories.into_iter().collect(),
            sessions: sessions.into_values().collect(),
            clients: clients.into_values().collect(),
            zellij_tabs: previous.zellij_tabs,
            resources,
            error: None,
            observation: Evidence {
                missing_directories,
                verified_panes,
                unfinished_sessions,
                failures: errors,
            },
        };
        snapshot.error = snapshot.observation_error();
        self.refresh_tab_order(&mut snapshot).await;
        Ok(snapshot)
    }

    pub(crate) async fn sample_resources(
        &self,
        previous: Vec<crate::store::opencode::resources::ProcessResource>,
    ) -> Result<Vec<crate::store::opencode::resources::ProcessResource>, String> {
        let observer = self.clone();
        tokio::task::spawn_blocking(move || {
            resources::collect(&observer.observed_presences(), previous)
        })
        .await
        .map_err(|error| error.to_string())
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
    read_bounded(path, 16_384)
}

fn read_bounded(path: &Path, limit: u64) -> Option<String> {
    if !fs::symlink_metadata(path).ok()?.is_file() {
        return None;
    }
    let mut text = String::new();
    fs::File::open(path)
        .ok()?
        .take(limit + 1)
        .read_to_string(&mut text)
        .ok()?;
    (text.len() as u64 <= limit).then_some(text)
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
        Activity::Idle | Activity::AwaitingAnswer => {
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
        Activity::AwaitingAnswer | Activity::Unknown => {}
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
    super::config::private_file(&directory.join("session-input.mjs"), false)
        .and_then(|mut file| file.write_all(include_bytes!("session-input.mjs")))
        .map_err(|error| error.to_string())?;
    super::config::private_file(&directory.join("bridge.mjs"), false)
        .and_then(|mut file| file.write_all(include_bytes!("bridge.mjs")))
        .map_err(|error| error.to_string())?;
    super::config::private_file(&directory.join("tui.js"), false)
        .and_then(|mut file| file.write_all(b"export { default } from './bridge.mjs'\n"))
        .map_err(|error| error.to_string())?;
    Ok(format!(
        "Add this directory to the plugins array in your OpenCode cli.json (V1: bridge.mjs in tui.json), then reopen OpenCode clients:\n{}",
        serde_json::to_string(&directory.display().to_string())
            .map_err(|error| error.to_string())?
    ))
}

#[cfg(test)]
#[path = "tests/mod.rs"]
pub(crate) mod tests;
