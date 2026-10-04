use std::{cell::RefCell, rc::Rc};

use ratatui::{
    style::Style,
    text::{Line, Span, Text},
};
use tuicore::{EventCtx, KeySpec, Notification, TuiEvent};

use super::{Msg, events::clean, row_actions::Command, rows::Row};
use crate::store::{
    environments::EnvironmentSnapshot,
    opencode,
    rules::{Acceptance, AcceptanceWorkspace, DispatchStatus},
};
mod actions;

#[derive(Clone, Default, PartialEq)]
pub(super) struct Context {
    pub inventory: EnvironmentSnapshot,
    pub opencode: opencode::Snapshot,
}
pub(super) type SharedContext = Rc<RefCell<Context>>;

#[derive(Clone, PartialEq)]
pub(crate) struct Target {
    pub(super) acceptance: Acceptance,
    pub(super) instance: Option<Row>,
    pub(super) workspace: Option<String>,
    pub(super) conversations: Vec<Row>,
    pub(super) selected: Option<Row>,
    pub(super) missing: &'static str,
    pub(super) available: bool,
    relative_date: String,
    has_conversations: bool,
}

impl std::fmt::Debug for Target {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("AcceptanceTarget")
            .field("acceptance", &self.acceptance.id)
            .field("instance", &self.acceptance.instance)
            .finish()
    }
}

impl Target {
    pub(super) fn new(
        acceptance: &Acceptance,
        history: Option<&AcceptanceWorkspace>,
        context: &Context,
        show_saved: bool,
    ) -> Self {
        let instance = super::rows::from_snapshot_with_operations(&context.inventory, &[])
            .into_iter()
            .find(|row| {
                row.instance.as_deref() == Some(&acceptance.instance)
                    && row.template == acceptance.rule.definition.template
            })
            .map(|mut row| {
                row.template_capabilities =
                    format!("· {} {}", super::rows::TEMPLATE_ICON, row.template);
                row
            });
        let workspace = instance
            .as_ref()
            .and_then(|row| row.workspace.clone())
            .or_else(|| history.map(|row| row.directory.clone()));
        let mut observed = context.opencode.clone();
        if let Some(history) = history {
            for saved in &history.sessions {
                if !observed
                    .sessions
                    .iter()
                    .any(|session| session.id == saved.id)
                {
                    observed.sessions.push(saved.clone());
                }
            }
        }
        let conversations = workspace.as_deref().map_or_else(Vec::new, |directory| {
            super::opencode::acceptance_rows(acceptance.id, directory, &observed, show_saved)
        });
        let has_conversations = workspace.as_deref().is_some_and(|directory| {
            observed
                .sessions
                .iter()
                .map(|session| session.directory.as_str())
                .chain(
                    observed
                        .clients
                        .iter()
                        .map(|client| client.directory.as_str()),
                )
                .any(|path| {
                    opencode::workspace_owner(path, std::iter::once((directory, directory)))
                        .is_some()
                })
        });
        let available = !context.inventory.loading
            && context.inventory.observed_at_unix_seconds.is_some()
            && context.inventory.error.is_none()
            && context.inventory.runtime_error.is_none();
        let missing = if !available {
            "instance unavailable"
        } else if matches!(
            acceptance.status,
            DispatchStatus::Queued | DispatchStatus::Provisioning | DispatchStatus::Launching
        ) {
            "instance pending"
        } else if acceptance.operation_id.is_none() {
            "instance not created"
        } else {
            "instance deleted"
        };
        let relative_date = chrono::DateTime::parse_from_rfc3339(&acceptance.accepted_at)
            .ok()
            .and_then(|date| time::OffsetDateTime::from_unix_timestamp(date.timestamp()).ok())
            .map(|date| tuicore::RelativeDate::new(date).text().to_owned())
            .unwrap_or_else(|| clean(&acceptance.accepted_at));
        Self {
            acceptance: acceptance.clone(),
            instance,
            workspace,
            conversations,
            selected: None,
            missing,
            available,
            relative_date,
            has_conversations,
        }
    }

    pub(super) fn enabled(&self, command: Command) -> bool {
        match command {
            Command::Instance | Command::PurgeInstance | Command::Routes => self.instance.is_some(),
            Command::CreateInstance => {
                self.instance.is_none()
                    && self.available
                    && !matches!(
                        self.acceptance.status,
                        DispatchStatus::Queued
                            | DispatchStatus::Provisioning
                            | DispatchStatus::Launching
                    )
            }
            Command::NewSession => self.instance.is_some() && self.workspace.is_some(),
            Command::Session => self.has_conversations,
            _ => true,
        }
    }

    pub(super) fn text(&self, spinner: &str, width: Option<u16>) -> Text<'static> {
        let theme = tuicore::theme();
        let muted = Style::default().fg(theme.muted_fg());
        let color = match self.acceptance.status {
            DispatchStatus::Launched => theme.success_fg(),
            DispatchStatus::Failed | DispatchStatus::Uncertain => theme.error_fg(),
            _ => theme.info_fg(),
        };
        let mut header = vec![
            Span::styled(" ", Style::default().fg(color)),
            Span::raw(clean(&self.acceptance.rule_name)),
            Span::styled(
                format!(
                    " #{} · event #{} · ",
                    self.acceptance.id, self.acceptance.event_sequence
                ),
                muted,
            ),
            Span::styled(
                format!("{:?}", self.acceptance.status),
                Style::default().fg(color),
            ),
            Span::styled(format!(" · {}", self.relative_date), muted),
        ];
        if self.instance.is_none() {
            header.push(Span::styled(format!(" · {}", self.missing), muted));
        }
        let mut lines = vec![Line::from(header)];
        if let Some(instance) = &self.instance {
            lines.extend(instance.text(spinner, width).lines);
        }
        Text::from(lines)
    }

    pub(super) fn search(&self) -> String {
        format!(
            "#{} event #{} {} {} {} {:?}",
            self.acceptance.id,
            self.acceptance.event_sequence,
            self.acceptance.rule_name,
            self.acceptance.event_summary,
            self.acceptance.instance,
            self.acceptance.status
        )
    }

    pub(super) fn action(&self, event: &TuiEvent, ctx: &mut EventCtx<Msg>) -> bool {
        let TuiEvent::Key(key) = event else {
            return false;
        };
        if KeySpec::plain('.').matches(*key) {
            ctx.emit(Msg::OpenRowMenu(
                super::row_actions::Target::AcceptanceContext(Box::new(self.clone())),
            ));
        } else if super::open_route_key().matches(*key) {
            if self.enabled(Command::Routes) {
                ctx.emit(Msg::AcceptanceAction(
                    Box::new(self.clone()),
                    Command::Routes,
                ));
            } else {
                ctx.notify(Notification::warning(
                    "Routes unavailable",
                    "This acceptance has no available instance",
                ));
            }
        } else if let Some(command) = [
            Command::Rule,
            Command::CreateInstance,
            Command::Instance,
            Command::NewSession,
            Command::Session,
            Command::PurgeInstance,
        ]
        .into_iter()
        .find(|command| KeySpec::plain(command.hotkey().chars().next().unwrap()).matches(*key))
        {
            if self.enabled(command) {
                ctx.emit(Msg::AcceptanceAction(Box::new(self.clone()), command));
            } else {
                ctx.notify(Notification::warning("Action unavailable", match command {
                    Command::CreateInstance => "The instance exists, inventory is unverified, or dispatch is still active",
                    Command::Session => "No conversations are known for this acceptance",
                    _ => "This acceptance has no available instance workspace",
                }));
            }
        } else {
            return false;
        }
        ctx.stop_propagation();
        true
    }
}
