use tuicore::{EventCtx, Notification};

use super::{App, Msg, PendingAction, Row, Target};
use crate::{
    app::{Intent, dialogs},
    store::opencode::CloseScope,
};

pub(in crate::app) fn new_session_directory(row: &Row) -> Option<&str> {
    if row.informational {
        return None;
    }
    row.workspace.as_deref()
}

pub(in crate::app) fn close_scope(row: &Row) -> Option<CloseScope> {
    if row.informational || row.service.is_some() {
        return None;
    }
    if matches!(row.opencode, Some(Target::Workspace)) {
        return Some(
            row.workspace
                .as_ref()
                .map_or(CloseScope::ExternalWorkspaces, |directory| {
                    CloseScope::Directory(directory.clone())
                }),
        );
    }
    if row.opencode.is_some() {
        return None;
    }
    row.instance
        .as_deref()
        .or_else(|| {
            row.id
                .strip_prefix("sessions:")
                .filter(|name| !name.is_empty())
        })
        .map(|name| CloseScope::Instance(name.to_owned()))
}

impl App {
    pub(in crate::app) fn create_opencode_session(
        &mut self,
        row: &Row,
        ctx: &mut EventCtx<Msg>,
    ) -> bool {
        if !self.service.opencode_enabled() {
            return false;
        }
        let Some(directory) = new_session_directory(row) else {
            return false;
        };
        if self.opencode_action.is_some() {
            return true;
        }
        let pane = match &row.opencode {
            Some(Target::Session { pane, .. }) => pane.clone(),
            Some(Target::Client { pane }) => Some(pane.clone()),
            _ => None,
        };
        match self.service.new_opencode_session(directory, pane) {
            Ok(reply) => self.opencode_action = Some(PendingAction::creation(reply)),
            Err(error) => ctx.notify(Notification::error("Cannot create OpenCode session", error)),
        }
        true
    }

    pub(in crate::app) fn confirm_close_opencode_scope(
        &mut self,
        row: &Row,
        ctx: &mut EventCtx<Msg>,
    ) -> bool {
        if !self.service.opencode_enabled() {
            return false;
        }
        let Some(scope) = close_scope(row) else {
            return false;
        };
        let label = match &scope {
            CloseScope::Instance(name) | CloseScope::Directory(name) => name.as_str(),
            CloseScope::ExternalWorkspaces => "all external workspaces",
        };
        let dialog = dialogs::confirm_close_opencode_sessions(label);
        self.intent = Some(Intent::CloseOpencodeSessions(scope));
        self.open(dialog, ctx);
        true
    }
}
