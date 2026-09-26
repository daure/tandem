use tuicore::{Button, EventCtx, Flex, FlexItem, Notification};

use super::{
    App, Msg, initial_focus,
    properties::Property,
    rows::{Row, Tone, compact_duration},
};
use crate::store::opencode::{Activity, Client, Counts, Pane, Session, Snapshot, workspace_owner};

mod conversation;

const FINISHED_ICON: &str = "";
const NEW_SESSION_ICON: &str = "";
const UNKNOWN_ICON: &str = "";
const STALE_ICON: &str = "";

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Target {
    Session { id: String, pane: Option<Pane> },
}

impl Target {
    pub(super) fn attached(&self) -> bool {
        match self {
            Self::Session { pane, .. } => pane.is_some(),
        }
    }
}

pub(super) fn append_rows(rows: &mut Vec<Row>, snapshot: &Snapshot, show_saved: bool) {
    let owners: Vec<_> = rows
        .iter()
        .filter_map(|row| Some((row.instance.clone()?, row.workspace.clone()?)))
        .collect();
    let mut children_by_instance = Vec::new();
    for row in rows.iter_mut().filter(|row| row.instance.is_some()) {
        let instance = row.instance.as_deref().unwrap();
        let mut children = Vec::new();
        let mut sessions: Vec<_> = snapshot
            .sessions
            .iter()
            .filter(|session| {
                workspace_owner(
                    &session.directory,
                    owners
                        .iter()
                        .map(|(name, workspace)| (name.as_str(), workspace.as_str())),
                ) == Some(instance)
            })
            .collect();
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
        sessions.sort_by(|a, b| {
            b.live()
                .cmp(&a.live())
                .then_with(|| b.updated.cmp(&a.updated))
                .then_with(|| a.id.cmp(&b.id))
        });
        clients.sort_by(|a, b| {
            a.pane
                .session
                .cmp(&b.pane.session)
                .then_with(|| a.pane.tab_name.cmp(&b.pane.tab_name))
                .then_with(|| a.pane.id.cmp(&b.pane.id))
        });
        let mut counts = Counts::of(sessions.iter().copied());
        counts.live += clients.len();
        counts.attached += clients.len();
        let summary = if snapshot.error.is_some()
            || sessions.iter().any(|session| session.stale)
            || clients.iter().any(|client| client.stale)
        {
            "OpenCode: observation incomplete".to_owned()
        } else {
            format!(
                "OpenCode: {} live · {} attached · {} busy",
                counts.live, counts.attached, counts.busy,
            )
        };
        row.status_detail = Some(match row.status_detail.take() {
            Some(detail) => format!("{detail} · {summary}"),
            None => summary,
        });
        for client in clients {
            children.push(client_row(row, instance, client));
        }
        for session in sessions
            .into_iter()
            .filter(|session| !session.saved() || show_saved)
        {
            let id = format!("opencode:{}:{}", instance, session.id);
            let status = if session.stale {
                "observation stale"
            } else {
                session.label()
            };
            let attached = session.attached();
            let (secondary_icon, secondary_tone) = if session.stale {
                (STALE_ICON, Tone::Warning)
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
                .or_else(|| session.question_observed.then_some("New session"));
            let elapsed = session.activity_elapsed_milliseconds.map(compact_duration);
            let label = secondary.map_or_else(
                || session.title.clone(),
                |secondary| format!("{}\n{secondary}", session.title),
            );
            let mut details = vec![
                Property::new("Session", &session.id),
                Property::new("Directory", &session.directory),
                Property::new("Server", &session.server),
                Property::new("Status", status),
            ];
            for pane in &session.panes {
                details.push(Property::new(
                    "Pane",
                    format!("{} / {} / {}", pane.session, pane.tab_name, pane.id),
                ));
            }
            children.push(Row {
                id: id.clone(),
                parent: Some(row.id.clone()),
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
                secondary_loading: session.activity == Activity::Busy && !session.stale,
                activity_timer: (session.activity == Activity::Busy && !session.stale)
                    .then_some(
                        session
                            .activity_started_at_milliseconds
                            .zip(session.activity_elapsed_milliseconds),
                    )
                    .flatten(),
                status_detail: elapsed,
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
                }),
                ..Default::default()
            });
            if session.panes.len() > 1 {
                for pane in &session.panes {
                    children.push(Row {
                        id: format!("{id}:{}:{}", pane.session, pane.id),
                        parent: Some(id.clone()),
                        label: format!("{} / {} · pane {}", pane.session, pane.tab_name, pane.id),
                        icon: "▣",
                        hide_resources: true,
                        opencode: Some(Target::Session {
                            id: session.id.clone(),
                            pane: Some(pane.clone()),
                        }),
                        ..Default::default()
                    });
                }
            }
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
    super::rows::assign_alternating_backgrounds(rows, "");
}

fn client_row(row: &Row, instance: &str, client: &Client) -> Row {
    let status = if client.stale {
        "observation stale"
    } else {
        "attached · no conversation"
    };
    let secondary = if client.stale {
        "Observation stale"
    } else {
        "New session"
    };
    Row {
        id: format!(
            "opencode-client:{instance}:{}:{}",
            client.pane.session, client.pane.id
        ),
        parent: Some(row.id.clone()),
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
            Tone::Muted
        },
        hide_resources: true,
        details: vec![
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
        ],
        ..Default::default()
    }
}

impl App {
    pub(super) fn open_opencode_dialog(&mut self, row: &Row, ctx: &mut EventCtx<Msg>) -> bool {
        let Some(Target::Session { id, .. }) = &row.opencode else {
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
        let tabs = tuicore::Tabs::dialog(vec![
            tuicore::Tab::new("Conversation", history),
            tuicore::Tab::new(
                "Details",
                super::properties::Properties::new(row.details.clone()),
            ),
            tuicore::Tab::new("Actions", navigation_buttons(&session)),
        ])
        .variant(tuicore::TabsVariant::OneRow)
        .edge_borders(ratatui::widgets::Borders::TOP)
        .on_close(|_| Msg::Close);
        self.intent = None;
        self.view.first_mut().replace_layer(Box::new(tabs), ctx);
        self.details_open = true;
        self.resize_details_dialog();
        self.view.first_mut().set_active_with_context(true, ctx);
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
            Target::Session { id, pane } => self.submit_opencode(id, pane.clone(), ctx),
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
                self.opencode_action = Some(reply);
                self.view.first_mut().set_active_with_context(false, ctx);
                ctx.focus(initial_focus());
            }
            Err(error) => ctx.notify(Notification::error("Cannot open OpenCode", error)),
        }
    }

    pub(super) fn poll_opencode_action(&mut self) {
        let Some(reply) = &mut self.opencode_action else {
            return;
        };
        let result = match reply.try_recv() {
            Ok(result) => result,
            Err(tokio::sync::oneshot::error::TryRecvError::Empty) => return,
            Err(tokio::sync::oneshot::error::TryRecvError::Closed) => {
                Err("OpenCode navigation worker stopped".into())
            }
        };
        self.opencode_action = None;
        if let Err(error) = result
            && self.service.opencode_enabled()
        {
            self.notify(Notification::error("Cannot open OpenCode", error));
        }
    }
}

fn navigation_buttons(session: &Session) -> Flex<Msg> {
    let mut body = Flex::column();
    if session.panes.is_empty() {
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
