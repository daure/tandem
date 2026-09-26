use std::{collections::BTreeMap, path::Path};

use tuicore::{Button, EventCtx, Flex, FlexItem, Notification};

use super::{
    App, Msg, initial_focus,
    properties::Property,
    rows::{Row, TEMPLATE_ICON, Tone, compact_duration},
};
use crate::store::opencode::{Activity, Client, Counts, Pane, Session, Snapshot, workspace_owner};

mod conversation;

const FINISHED_ICON: &str = "";
const NEW_SESSION_ICON: &str = "";
const UNKNOWN_ICON: &str = "";
const STALE_ICON: &str = "";
const SESSION_DISPLAY_LIMIT: usize = 20;

enum SessionChild<'a> {
    Session(&'a Session),
    Client(&'a Client),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Target {
    Workspace,
    Client {
        pane: Pane,
    },
    Session {
        id: String,
        pane: Option<Pane>,
        owned: bool,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct ClosingPane {
    session_id: Option<String>,
    pane: Pane,
}

pub(super) struct PendingAction {
    reply: tokio::sync::oneshot::Receiver<Result<(), String>>,
    error_title: &'static str,
    closing: Vec<ClosingPane>,
}

impl PendingAction {
    pub(super) fn new(
        reply: tokio::sync::oneshot::Receiver<Result<(), String>>,
        error_title: &'static str,
        closing: Vec<ClosingPane>,
    ) -> Self {
        Self {
            reply,
            error_title,
            closing,
        }
    }
}

impl ClosingPane {
    fn new(session_id: Option<&str>, pane: &Pane) -> Self {
        Self {
            session_id: session_id.map(str::to_owned),
            pane: pane.clone(),
        }
    }

    fn matches_session(&self, session_id: &str, pane: &Pane) -> bool {
        self.session_id.as_deref() == Some(session_id) && pane == &self.pane
    }

    fn matches_client(&self, pane: &Pane) -> bool {
        self.session_id.is_none() && pane == &self.pane
    }
}

impl Target {
    pub(super) fn attached(&self) -> bool {
        match self {
            Self::Session { pane, .. } => pane.is_some(),
            Self::Workspace | Self::Client { .. } => false,
        }
    }

    pub(super) fn session_action(&self) -> Option<(bool, bool)> {
        match self {
            Self::Session { pane, owned, .. } => Some((pane.is_some(), *owned)),
            Self::Workspace | Self::Client { .. } => None,
        }
    }

    pub(super) fn closeable(&self) -> bool {
        matches!(
            self,
            Self::Session { pane: Some(_), .. } | Self::Client { .. }
        )
    }

    pub(super) fn external_observation(&self) -> bool {
        match self {
            Self::Session { owned, .. } => !owned,
            Self::Workspace | Self::Client { .. } => true,
        }
    }
}

#[cfg(test)]
pub(super) fn attached_rows(rows: Vec<Row>, snapshot: &Snapshot) -> Vec<Row> {
    let owners = rows
        .iter()
        .filter_map(|row| Some((row.instance.clone()?, row.workspace.clone()?)))
        .collect::<Vec<_>>();
    attached_rows_for_owners(rows, snapshot, &owners, false)
}

pub(super) fn attached_rows_for_owners(
    mut rows: Vec<Row>,
    snapshot: &Snapshot,
    owners: &[(String, String)],
    show_saved: bool,
) -> Vec<Row> {
    append_rows_for_owners(&mut rows, snapshot, show_saved, false, owners, false);
    let visible_session = |row: &Row| {
        row.opencode.as_ref().is_some_and(|target| {
            target.attached() || show_saved && matches!(target, Target::Session { .. })
        })
    };
    let instance_ids: std::collections::HashSet<_> = rows
        .iter()
        .filter(|row| row.instance.is_some())
        .map(|row| row.id.clone())
        .collect();
    let session_instance_ids: std::collections::HashSet<_> = rows
        .iter()
        .filter(|row| visible_session(row))
        .filter_map(|row| row.parent.as_ref())
        .filter(|parent| instance_ids.contains(*parent))
        .cloned()
        .collect();
    let mut grouped: Vec<_> = rows
        .into_iter()
        .filter_map(|mut row| {
            if row.instance.is_some() && session_instance_ids.contains(&row.id) {
                row.parent = None;
                row.template_capabilities = format!("· {TEMPLATE_ICON} {}", row.template);
                row.status_detail = None;
                row.hide_resources = true;
                return Some(row);
            }
            let parent = row.parent.as_ref()?;
            (session_instance_ids.contains(parent)
                && (visible_session(&row)
                    || row.informational && row.id.starts_with("opencode-more:")))
            .then_some(row)
        })
        .collect();
    append_external_rows(&mut grouped, snapshot, show_saved, false, owners);
    super::rows::assign_alternating_backgrounds(&mut grouped, "");
    grouped
}

pub(super) fn hide_closing_panes(snapshot: &mut Snapshot, closing: &mut Vec<ClosingPane>) {
    closing.retain(|target| {
        target.session_id.as_deref().map_or_else(
            || {
                snapshot
                    .clients
                    .iter()
                    .any(|client| target.matches_client(&client.pane))
            },
            |id| {
                snapshot.sessions.iter().any(|session| {
                    session.id == id
                        && session
                            .panes
                            .iter()
                            .any(|pane| target.matches_session(id, pane))
                })
            },
        )
    });
    for session in &mut snapshot.sessions {
        let id = &session.id;
        session.panes.retain(|pane| {
            !closing
                .iter()
                .any(|target| target.matches_session(id, pane))
        });
    }
    snapshot.clients.retain(|client| {
        !closing
            .iter()
            .any(|target| target.matches_client(&client.pane))
    });
}

#[cfg(test)]
pub(super) fn append_rows(rows: &mut Vec<Row>, snapshot: &Snapshot, show_saved: bool) {
    append_rows_with_grouping(rows, snapshot, show_saved, true);
}

#[cfg(test)]
fn append_rows_with_grouping(
    rows: &mut Vec<Row>,
    snapshot: &Snapshot,
    show_saved: bool,
    group_sessions: bool,
) {
    let owners: Vec<_> = rows
        .iter()
        .filter_map(|row| Some((row.instance.clone()?, row.workspace.clone()?)))
        .collect();
    append_rows_for_owners(rows, snapshot, show_saved, group_sessions, &owners, true);
}

fn append_rows_for_owners(
    rows: &mut Vec<Row>,
    snapshot: &Snapshot,
    show_saved: bool,
    group_sessions: bool,
    owners: &[(String, String)],
    include_external: bool,
) {
    let mut children_by_instance = Vec::new();
    for row in rows.iter_mut().filter(|row| row.instance.is_some()) {
        let instance = row.instance.as_deref().unwrap();
        let mut children = Vec::new();
        let (mut sessions, sessions_truncated) =
            recent_sessions_per_directory(snapshot.sessions.iter().filter(|session| {
                workspace_owner(
                    &session.directory,
                    owners
                        .iter()
                        .map(|(name, workspace)| (name.as_str(), workspace.as_str())),
                ) == Some(instance)
                    && (!session.saved() || show_saved)
                    && (group_sessions || show_saved || session.attached())
            }));
        let mut clients: Vec<_> = snapshot
            .clients
            .iter()
            .filter(|client| {
                workspace_owner(
                    &client.directory,
                    owners
                        .iter()
                        .map(|(name, workspace)| (name.as_str(), workspace.as_str())),
                ) == Some(instance)
            })
            .collect();
        if let Some(error) = &snapshot.error {
            row.details
                .push(Property::new("OpenCode observation", error).tone(Tone::Warning));
        }
        if sessions.is_empty() && clients.is_empty() && snapshot.error.is_none() {
            continue;
        }
        sessions.sort_by(|a, b| compare_sessions(a, b));
        clients.sort_by(|a, b| {
            a.pane
                .session
                .cmp(&b.pane.session)
                .then_with(|| a.pane.tab_name.cmp(&b.pane.tab_name))
                .then_with(|| a.pane.id.cmp(&b.pane.id))
        });
        let session_parent = if group_sessions {
            format!("sessions:{instance}")
        } else {
            row.id.clone()
        };
        let ordered_children = sessions
            .iter()
            .copied()
            .filter(|session| session.activity != Activity::Busy)
            .map(SessionChild::Session)
            .chain(clients.into_iter().map(SessionChild::Client))
            .chain(
                sessions
                    .iter()
                    .copied()
                    .filter(|session| session.activity == Activity::Busy)
                    .map(SessionChild::Session),
            );
        for child in ordered_children {
            let SessionChild::Session(session) = child else {
                let SessionChild::Client(client) = child else {
                    unreachable!();
                };
                children.push(client_row(&session_parent, instance, client, false));
                continue;
            };
            append_session_rows(&mut children, &session_parent, instance, session, true);
        }
        if sessions_truncated {
            children.push(see_more_row(&session_parent, instance));
        }
        if group_sessions && !children.is_empty() {
            children.insert(0, sessions_group(row, instance, &children));
        }
        children_by_instance.push((row.id.clone(), children));
    }
    for (parent, children) in children_by_instance {
        if children.is_empty() {
            continue;
        }
        let insertion = rows
            .iter()
            .position(|row| row.parent.as_deref() == Some(parent.as_str()))
            .or_else(|| {
                rows.iter()
                    .position(|row| row.id == parent)
                    .map(|index| index + 1)
            })
            .unwrap_or(rows.len());
        rows.splice(insertion..insertion, children);
    }
    if include_external {
        append_external_rows(rows, snapshot, show_saved, group_sessions, owners);
    }
    super::rows::assign_alternating_backgrounds(rows, "");
}

fn append_session_rows(
    rows: &mut Vec<Row>,
    parent: &str,
    scope: &str,
    session: &Session,
    owned: bool,
) {
    let id = format!("opencode:{scope}:{}", session.id);
    let status = if session.stale {
        "observation stale"
    } else {
        session.label()
    };
    let attached = session.attached();
    let new_session = session.last_question.is_none() && session.question_observed;
    let (secondary_icon, secondary_tone) = if session.stale {
        (STALE_ICON, Tone::Warning)
    } else if new_session {
        (NEW_SESSION_ICON, Tone::Subtle)
    } else {
        match session.activity {
            Activity::Busy => ("", Tone::Info),
            Activity::Idle => (
                FINISHED_ICON,
                if attached { Tone::Success } else { Tone::Muted },
            ),
            Activity::Unknown => (UNKNOWN_ICON, Tone::Muted),
        }
    };
    let secondary = session
        .last_question
        .as_deref()
        .or_else(|| session.question_observed.then_some("(new session)"));
    let elapsed = session.activity_elapsed_milliseconds.map(compact_duration);
    let context = context_detail(session);
    let label = secondary.map_or_else(
        || session.title.clone(),
        |secondary| format!("{}\n{secondary}", session.title),
    );
    let mut details = vec![Property::new("Session", &session.id)];
    if !owned {
        details.push(Property::new("Ownership", "Outside Tandem"));
    }
    details.extend([
        Property::new("Directory", &session.directory),
        Property::new("Server", &session.server),
        Property::new("Status", status),
    ]);
    for pane in &session.panes {
        details.push(Property::new(
            "Pane",
            format!("{} / {} / {}", pane.session, pane.tab_name, pane.id),
        ));
    }
    rows.push(Row {
        id: id.clone(),
        parent: Some(parent.to_owned()),
        label,
        icon: "󰚩",
        tone: if session.stale {
            Tone::Warning
        } else if session.activity == Activity::Busy {
            Tone::Info
        } else if attached {
            Tone::Success
        } else {
            Tone::Muted
        },
        secondary_icon: secondary.map_or("", |_| secondary_icon),
        secondary_tone,
        secondary_text_tone: new_session.then_some(Tone::Subtle),
        secondary_loading: session.activity == Activity::Busy && !session.stale && !new_session,
        activity_timer: (session.activity == Activity::Busy && !session.stale)
            .then_some(
                session
                    .activity_started_at_milliseconds
                    .zip(session.activity_elapsed_milliseconds),
            )
            .flatten(),
        status_detail: [elapsed, context]
            .into_iter()
            .flatten()
            .reduce(|detail, context| format!("{detail} · {context}")),
        detail_tone: if session.activity == Activity::Busy {
            Tone::Info
        } else {
            Tone::Muted
        },
        hide_resources: true,
        details,
        opencode: Some(Target::Session {
            id: session.id.clone(),
            pane: session.panes.first().cloned(),
            owned,
        }),
        ..Default::default()
    });
    if session.panes.len() > 1 {
        for pane in &session.panes {
            rows.push(Row {
                id: format!("{id}:{}:{}", pane.session, pane.id),
                parent: Some(id.clone()),
                label: format!("{} / {} · pane {}", pane.session, pane.tab_name, pane.id),
                icon: "▣",
                hide_resources: true,
                opencode: Some(Target::Session {
                    id: session.id.clone(),
                    pane: Some(pane.clone()),
                    owned,
                }),
                ..Default::default()
            });
        }
    }
}

fn context_detail(session: &Session) -> Option<String> {
    let tokens = session.context_tokens?;
    let limit = session.context_limit.filter(|limit| *limit > 0)?;
    let percent = (u128::from(tokens) * 100 + u128::from(limit) / 2) / u128::from(limit);
    Some(format!(
        "{}/{} ({percent}%)",
        compact_tokens(tokens),
        compact_tokens(limit)
    ))
}

fn compact_tokens(tokens: u64) -> String {
    if tokens >= 1_000_000 {
        let tenths = tokens.saturating_add(50_000) / 100_000;
        return if tenths.is_multiple_of(10) {
            format!("{}m", tenths / 10)
        } else {
            format!("{}.{}m", tenths / 10, tenths % 10)
        };
    }
    if tokens >= 1_000 {
        return format!("{}k", tokens.saturating_add(500) / 1_000);
    }
    tokens.to_string()
}

fn append_external_rows(
    rows: &mut Vec<Row>,
    snapshot: &Snapshot,
    show_saved: bool,
    overview: bool,
    owners: &[(String, String)],
) {
    let outside_tandem = |directory: &str| {
        workspace_owner(
            directory,
            owners
                .iter()
                .map(|(name, workspace)| (name.as_str(), workspace.as_str())),
        )
        .is_none()
    };
    let mut workspaces: BTreeMap<String, (Vec<&Session>, Vec<&Client>)> = BTreeMap::new();
    for session in snapshot.sessions.iter().filter(|session| {
        outside_tandem(&session.directory)
            && (!session.saved() || show_saved)
            && (overview || show_saved || session.attached())
    }) {
        workspaces
            .entry(session.directory.clone())
            .or_default()
            .0
            .push(session);
    }
    for client in snapshot
        .clients
        .iter()
        .filter(|client| outside_tandem(&client.directory))
    {
        workspaces
            .entry(client.directory.clone())
            .or_default()
            .1
            .push(client);
    }
    if workspaces.is_empty() {
        return;
    }

    let mut projected = Vec::new();
    for (directory, (sessions, mut clients)) in workspaces {
        let (mut sessions, sessions_truncated) = recent_sessions_per_directory(sessions);
        let client_count = clients.len();
        sessions.sort_by(|a, b| compare_sessions(a, b));
        clients.sort_by(|a, b| {
            a.pane
                .session
                .cmp(&b.pane.session)
                .then_with(|| a.pane.tab_name.cmp(&b.pane.tab_name))
                .then_with(|| a.pane.id.cmp(&b.pane.id))
        });
        let workspace_id = format!("opencode-workspace:{directory}");
        let scope = format!("external:{directory}");
        let mut children = Vec::new();
        let ordered_children = sessions
            .iter()
            .copied()
            .filter(|session| session.activity != Activity::Busy)
            .map(SessionChild::Session)
            .chain(clients.into_iter().map(SessionChild::Client))
            .chain(
                sessions
                    .iter()
                    .copied()
                    .filter(|session| session.activity == Activity::Busy)
                    .map(SessionChild::Session),
            );
        for child in ordered_children {
            match child {
                SessionChild::Session(session) => {
                    append_session_rows(&mut children, &workspace_id, &scope, session, false)
                }
                SessionChild::Client(client) => {
                    children.push(client_row(&workspace_id, &scope, client, true));
                }
            }
        }
        if sessions_truncated {
            children.push(see_more_row(&workspace_id, &scope));
        }
        let (tone, loading) = group_tone(&children);
        let shown_directory = display_directory(&directory);
        let label = if overview {
            shown_directory
        } else {
            let name = Path::new(&directory)
                .file_name()
                .and_then(|name| name.to_str())
                .filter(|name| !name.is_empty())
                .unwrap_or(&directory);
            format!("{name}\n{shown_directory}")
        };
        projected.push(Row {
            id: workspace_id.clone(),
            parent: overview.then(|| "opencode-workspaces".into()),
            label,
            template_capabilities: " external".into(),
            icon: "",
            tone,
            loading,
            workspace: Some(directory.clone()),
            hide_resources: true,
            details: vec![
                Property::new("Ownership", "Outside Tandem"),
                Property::new("Directory", &directory),
                Property::new("Sessions", sessions.len()),
                Property::new("Clients without a conversation", client_count),
            ],
            opencode: Some(Target::Workspace),
            ..Default::default()
        });
        projected.extend(children);
    }

    if overview {
        let sessions = snapshot
            .sessions
            .iter()
            .filter(|session| outside_tandem(&session.directory))
            .collect::<Vec<_>>();
        let clients = snapshot
            .clients
            .iter()
            .filter(|client| outside_tandem(&client.directory))
            .collect::<Vec<_>>();
        let mut counts = Counts::of(sessions.iter().copied());
        counts.live += clients.len();
        counts.attached += clients.len();
        let incomplete = snapshot.error.is_some()
            || sessions.iter().any(|session| session.stale)
            || clients.iter().any(|client| client.stale);
        projected.insert(
            0,
            Row {
                id: "opencode-workspaces".into(),
                label: "Other OpenCode workspaces".into(),
                icon: "󰚩",
                tone: if incomplete {
                    Tone::Warning
                } else {
                    Tone::Normal
                },
                status_detail: Some(if incomplete {
                    "observation incomplete".into()
                } else {
                    format!(
                        "{} live · {} attached · {} busy",
                        counts.live, counts.attached, counts.busy
                    )
                }),
                hide_resources: true,
                details: vec![
                    Property::new("Ownership", "Outside Tandem"),
                    Property::new("Live", counts.live),
                    Property::new("Attached", counts.attached),
                    Property::new("Busy", counts.busy),
                ],
                opencode: Some(Target::Workspace),
                ..Default::default()
            },
        );
        rows.splice(0..0, projected);
    } else {
        rows.extend(projected);
    }
}

fn display_directory(directory: &str) -> String {
    let Some(home) = dirs::home_dir() else {
        return directory.to_owned();
    };
    let Ok(relative) = Path::new(directory).strip_prefix(&home) else {
        return directory.to_owned();
    };
    if relative.as_os_str().is_empty() {
        "~".into()
    } else {
        format!("~/{}", relative.display())
    }
}

fn recent_sessions_per_directory<'a>(
    sessions: impl IntoIterator<Item = &'a Session>,
) -> (Vec<&'a Session>, bool) {
    let mut by_directory = BTreeMap::<&str, Vec<&Session>>::new();
    for session in sessions {
        by_directory
            .entry(&session.directory)
            .or_default()
            .push(session);
    }
    let mut recent = Vec::new();
    let mut truncated = false;
    for sessions in by_directory.values_mut() {
        sessions.sort_by(|a, b| b.updated.cmp(&a.updated).then_with(|| a.id.cmp(&b.id)));
        truncated |= sessions.len() > SESSION_DISPLAY_LIMIT;
        recent.extend(sessions.iter().take(SESSION_DISPLAY_LIMIT).copied());
    }
    (recent, truncated)
}

fn see_more_row(parent: &str, scope: &str) -> Row {
    Row {
        id: format!("opencode-more:{scope}"),
        parent: Some(parent.to_owned()),
        label: "(see more sessions in opencode)".into(),
        tone: Tone::Muted,
        informational: true,
        hide_resources: true,
        ..Default::default()
    }
}

fn group_tone(rows: &[Row]) -> (Tone, bool) {
    let loading = rows.iter().any(|row| row.tone == Tone::Info);
    let tone = if loading {
        Tone::Info
    } else if rows.iter().any(|row| row.tone == Tone::Success) {
        Tone::Success
    } else if rows.iter().any(|row| row.tone == Tone::Warning) {
        Tone::Warning
    } else {
        Tone::Muted
    };
    (tone, loading)
}

fn compare_sessions(a: &Session, b: &Session) -> std::cmp::Ordering {
    session_order(a)
        .cmp(&session_order(b))
        .then_with(|| match a.activity {
            Activity::Busy => a
                .activity_started_at_milliseconds
                .unwrap_or(a.updated)
                .cmp(&b.activity_started_at_milliseconds.unwrap_or(b.updated)),
            Activity::Idle | Activity::Unknown => b.updated.cmp(&a.updated),
        })
        .then_with(|| a.id.cmp(&b.id))
}

fn session_order(session: &Session) -> u8 {
    match (session.activity, session.attached()) {
        (Activity::Idle, true) => 0,
        (Activity::Idle, false) => 1,
        (Activity::Unknown, _) => 2,
        (Activity::Busy, _) => 3,
    }
}

fn sessions_group(row: &Row, instance: &str, children: &[Row]) -> Row {
    let id = format!("sessions:{instance}");
    let session_rows = children
        .iter()
        .filter(|child| child.parent.as_deref() == Some(id.as_str()))
        .collect::<Vec<_>>();
    let loading = session_rows.iter().any(|child| child.tone == Tone::Info);
    let tone = if loading {
        Tone::Info
    } else if session_rows.iter().any(|child| child.tone == Tone::Success) {
        Tone::Success
    } else if session_rows.iter().any(|child| child.tone == Tone::Warning) {
        Tone::Warning
    } else {
        Tone::Muted
    };
    Row {
        id,
        parent: Some(row.id.clone()),
        label: "Sessions".into(),
        icon: "󰚩",
        tone,
        loading,
        template: row.template.clone(),
        directory: row.directory.clone(),
        hide_resources: true,
        ..Default::default()
    }
}

fn client_row(parent: &str, scope: &str, client: &Client, external: bool) -> Row {
    let status = if client.stale {
        "observation stale"
    } else {
        "attached · no conversation"
    };
    let secondary = if client.stale {
        "Observation stale"
    } else {
        "(new session)"
    };
    Row {
        id: format!(
            "opencode-client:{scope}:{}:{}",
            client.pane.session, client.pane.id
        ),
        parent: Some(parent.to_owned()),
        label: format!("{}\n{secondary}", client.title),
        icon: "󰚩",
        tone: if client.stale {
            Tone::Warning
        } else {
            Tone::Success
        },
        secondary_icon: if client.stale {
            STALE_ICON
        } else {
            NEW_SESSION_ICON
        },
        secondary_tone: if client.stale {
            Tone::Warning
        } else {
            Tone::Subtle
        },
        secondary_text_tone: (!client.stale).then_some(Tone::Subtle),
        hide_resources: true,
        details: [external.then(|| Property::new("Ownership", "Outside Tandem"))]
            .into_iter()
            .flatten()
            .chain([
                Property::new("Directory", &client.directory),
                Property::new("Server", &client.server),
                Property::new("Status", status),
                Property::new(
                    "Pane",
                    format!(
                        "{} / {} / {}",
                        client.pane.session, client.pane.tab_name, client.pane.id
                    ),
                ),
            ])
            .collect(),
        opencode: external.then(|| Target::Client {
            pane: client.pane.clone(),
        }),
        ..Default::default()
    }
}

impl App {
    pub(super) fn project_rows(
        &self,
        snapshot: &crate::store::environments::EnvironmentSnapshot,
        operations: &[crate::store::environments::Operation],
    ) -> Vec<Row> {
        let mut rows = super::visible_rows(snapshot, operations, self.running_only);
        let owners = snapshot
            .instances
            .iter()
            .map(|instance| (instance.name.clone(), instance.workspace.clone()))
            .collect::<Vec<_>>();
        if self.attached_sessions_only {
            attached_rows_for_owners(
                rows,
                &self.opencode_snapshot,
                &owners,
                self.opencode_history,
            )
        } else {
            append_rows_for_owners(
                &mut rows,
                &self.opencode_snapshot,
                self.opencode_history,
                true,
                &owners,
                true,
            );
            rows
        }
    }

    pub(super) fn open_opencode_dialog(&mut self, row: &Row, ctx: &mut EventCtx<Msg>) -> bool {
        let Some(Target::Session { id, owned, .. }) = &row.opencode else {
            return false;
        };
        if !self.service.opencode_enabled() {
            return true;
        }
        let Some(session) = self
            .opencode_snapshot
            .sessions
            .iter()
            .find(|session| session.id == *id)
            .cloned()
        else {
            return true;
        };
        let history = conversation::Conversation::new(self.service.opencode_conversation(id));
        let mut tabs = vec![
            tuicore::Tab::new("Conversation", history),
            tuicore::Tab::new(
                "Details",
                super::properties::Properties::new(row.details.clone()),
            ),
        ];
        if *owned || session.attached() {
            tabs.push(tuicore::Tab::new(
                "Actions",
                navigation_buttons(&session, *owned),
            ));
        }
        let tabs = tuicore::Tabs::dialog(tabs)
            .variant(tuicore::TabsVariant::OneRow)
            .edge_borders(ratatui::widgets::Borders::TOP)
            .on_close(|_| Msg::Close);
        self.intent = None;
        self.view.replace_layer(Box::new(tabs), ctx);
        self.details_open = true;
        self.resize_details_dialog();
        self.view.set_active_with_context(true, ctx);
        true
    }

    pub(super) fn activate_opencode(&mut self, row: &Row, ctx: &mut EventCtx<Msg>) -> bool {
        let Some(target) = &row.opencode else {
            return false;
        };
        if !self.service.opencode_enabled() {
            return true;
        }
        match target {
            Target::Session { id, pane, owned } => {
                if *owned || pane.is_some() {
                    self.submit_opencode(id, pane.clone(), ctx);
                }
            }
            Target::Client { pane } => self.submit_opencode_client(pane.clone(), ctx),
            Target::Workspace => return false,
        }
        true
    }

    pub(super) fn submit_opencode(
        &mut self,
        id: &str,
        pane: Option<Pane>,
        ctx: &mut EventCtx<Msg>,
    ) {
        if self.opencode_action.is_some() {
            return;
        }
        match self.service.open_opencode(id, pane) {
            Ok(reply) => {
                self.opencode_action = Some(PendingAction::new(
                    reply,
                    "Cannot open OpenCode",
                    Vec::new(),
                ));
                self.view.set_active_with_context(false, ctx);
                ctx.focus(initial_focus());
            }
            Err(error) => ctx.notify(Notification::error("Cannot open OpenCode", error)),
        }
    }

    fn submit_opencode_client(&mut self, pane: Pane, ctx: &mut EventCtx<Msg>) {
        if self.opencode_action.is_some() {
            return;
        }
        match self.service.open_opencode_client(pane) {
            Ok(reply) => {
                self.opencode_action = Some(PendingAction::new(
                    reply,
                    "Cannot open OpenCode",
                    Vec::new(),
                ));
                self.view.set_active_with_context(false, ctx);
                ctx.focus(initial_focus());
            }
            Err(error) => ctx.notify(Notification::error("Cannot open OpenCode", error)),
        }
    }

    pub(super) fn close_opencode(&mut self, row: &Row, ctx: &mut EventCtx<Msg>) -> bool {
        let (id, pane) = match &row.opencode {
            Some(Target::Session {
                id,
                pane: Some(pane),
                ..
            }) => (Some(id.as_str()), pane),
            Some(Target::Client { pane }) => (None, pane),
            Some(Target::Workspace | Target::Session { pane: None, .. }) | None => return false,
        };
        if self.opencode_action.is_some() {
            return true;
        }
        let result = if let Some(id) = id {
            self.service.close_opencode(id, pane.clone())
        } else {
            self.service.close_opencode_client(pane.clone())
        };
        let id = id.map(str::to_owned);
        match result {
            Ok(reply) => {
                let closing = if let Some(id) = id.as_deref() {
                    self.optimistically_close_opencode_pane(id, pane)
                } else {
                    let closing = ClosingPane::new(None, pane);
                    if !self.closing_opencode_panes.contains(&closing) {
                        self.closing_opencode_panes.push(closing.clone());
                    }
                    self.update_snapshot(self.snapshot.clone());
                    closing
                };
                self.opencode_action = Some(PendingAction::new(
                    reply,
                    "Cannot close OpenCode session",
                    vec![closing],
                ));
                ctx.focus(initial_focus());
                ctx.request_layout();
                ctx.request_redraw();
            }
            Err(error) => ctx.notify(Notification::error("Cannot close OpenCode session", error)),
        }
        true
    }

    pub(super) fn optimistically_close_opencode_pane(
        &mut self,
        id: &str,
        pane: &Pane,
    ) -> ClosingPane {
        let closing = ClosingPane::new(Some(id), pane);
        if !self.closing_opencode_panes.contains(&closing) {
            self.closing_opencode_panes.push(closing.clone());
        }
        self.update_snapshot(self.snapshot.clone());
        closing
    }

    fn optimistically_close_opencode_panes(&mut self, panes: &[Pane]) -> Vec<ClosingPane> {
        let closing = panes
            .iter()
            .map(|pane| {
                let session_id = self
                    .opencode_snapshot
                    .sessions
                    .iter()
                    .find(|session| session.panes.contains(pane))
                    .map(|session| session.id.as_str());
                ClosingPane::new(session_id, pane)
            })
            .collect::<Vec<_>>();
        for target in &closing {
            if !self.closing_opencode_panes.contains(target) {
                self.closing_opencode_panes.push(target.clone());
            }
        }
        self.update_snapshot(self.snapshot.clone());
        closing
    }

    pub(super) fn submit_close_instance_opencode(&mut self, name: String, ctx: &mut EventCtx<Msg>) {
        if self.opencode_action.is_some() {
            self.view
                .layer_mut()
                .set_bottom_left("Another OpenCode action is in progress".into());
            return;
        }
        match self.service.close_instance_opencode(&name) {
            Ok(outcome) => {
                let closing = self.optimistically_close_opencode_panes(&outcome.panes);
                self.opencode_action = Some(PendingAction::new(
                    outcome.reply,
                    "Cannot close OpenCode sessions",
                    closing,
                ));
                self.view.set_active_with_context(false, ctx);
                self.intent = None;
                self.details_open = false;
                ctx.focus(initial_focus());
                ctx.request_layout();
                ctx.request_redraw();
            }
            Err(error) => self.view.layer_mut().set_bottom_left(error),
        }
    }

    pub(super) fn poll_opencode_action(&mut self) {
        let Some(action) = &mut self.opencode_action else {
            return;
        };
        let error_title = action.error_title;
        let closing = action.closing.clone();
        let result = match action.reply.try_recv() {
            Ok(result) => result,
            Err(tokio::sync::oneshot::error::TryRecvError::Empty) => return,
            Err(tokio::sync::oneshot::error::TryRecvError::Closed) => {
                Err("OpenCode navigation worker stopped".into())
            }
        };
        self.opencode_action = None;
        if let Err(error) = result {
            if !closing.is_empty() {
                self.closing_opencode_panes
                    .retain(|candidate| !closing.contains(candidate));
                self.update_snapshot(self.snapshot.clone());
            }
            if self.service.opencode_enabled() {
                self.notify(Notification::error(error_title, error));
            }
        }
    }
}

fn navigation_buttons(session: &Session, owned: bool) -> Flex<Msg> {
    let mut body = Flex::column();
    if session.panes.is_empty() && owned {
        let id = session.id.clone();
        body = body.child(
            "attach",
            Button::new(if session.activity == Activity::Busy {
                "Attach to conversation"
            } else {
                "Resume conversation"
            })
            .on_press(move || Msg::OpenOpencode(id.clone(), None)),
            FlexItem::fit_content(),
        );
    } else {
        for (index, pane) in session.panes.iter().cloned().enumerate() {
            let id = session.id.clone();
            body = body.child(
                format!("pane-{index}"),
                Button::new(format!(
                    "Jump to {} / {} · pane {}",
                    pane.session, pane.tab_name, pane.id
                ))
                .on_press(move || Msg::OpenOpencode(id.clone(), Some(pane.clone()))),
                FlexItem::fit_content(),
            );
        }
    }
    body
}
