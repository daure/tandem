use tuicore::{EventCtx, KeySpec, Notification, TuiEvent};

use super::{App, Msg, PendingAction, Row, Target};
use crate::app::{Intent, dialogs};

pub(in crate::app) fn clearable_folder(row: &Row) -> Option<&str> {
    (matches!(row.opencode, Some(Target::Workspace))
        && row.tone == crate::app::rows::Tone::Muted
        && !row.loading)
        .then_some(row.workspace.as_deref())
        .flatten()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SessionAction {
    Rename,
    Delete,
}

impl SessionAction {
    pub(in crate::app) fn from_event(event: &TuiEvent) -> Option<Self> {
        let TuiEvent::Key(key) = event else {
            return None;
        };
        if KeySpec::plain('r').matches(*key) {
            Some(Self::Rename)
        } else if KeySpec::plain('x').matches(*key) {
            Some(Self::Delete)
        } else {
            None
        }
    }
}

pub(in crate::app) fn session_hotkey(row: &Row, event: &TuiEvent, ctx: &mut EventCtx<Msg>) -> bool {
    let Some(Target::Session { id, .. }) = &row.opencode else {
        return false;
    };
    let Some(action) = SessionAction::from_event(event) else {
        return false;
    };
    ctx.emit(Msg::OpencodeSessionAction(id.clone(), action));
    ctx.stop_propagation();
    true
}

impl App {
    pub(in crate::app) fn request_clear_opencode_folder(
        &mut self,
        directory: String,
        ctx: &mut EventCtx<Msg>,
    ) {
        let modal = dialogs::clear_opencode_folder(&directory);
        self.intent = Some(Intent::ClearOpencodeFolder(directory));
        self.open_compact(modal, ctx);
    }

    pub(in crate::app) fn request_opencode_session_action(
        &mut self,
        id: String,
        action: SessionAction,
        ctx: &mut EventCtx<Msg>,
    ) {
        if self.opencode_action.is_some() {
            ctx.notify(Notification::warning(
                "OpenCode action unavailable",
                "Wait for the current OpenCode action to finish",
            ));
            return;
        }
        let Some(session) = self.service.known_opencode_session(&id) else {
            ctx.notify(Notification::warning(
                "Conversation unavailable",
                "Refresh and try again",
            ));
            return;
        };
        let modal = match action {
            SessionAction::Rename => {
                self.name = session.title;
                self.intent = Some(Intent::RenameOpencodeSession(id));
                dialogs::rename_opencode_session(&self.name)
            }
            SessionAction::Delete => {
                self.intent = Some(Intent::DeleteOpencodeSession(id));
                dialogs::delete_opencode_session(&session.title)
            }
        };
        self.open_compact(modal, ctx);
    }

    pub(in crate::app) fn submit_opencode_session_edit(&mut self, ctx: &mut EventCtx<Msg>) -> bool {
        if let Some(Intent::ClearOpencodeFolder(directory)) = &self.intent {
            match self.service.clear_opencode_folder(directory) {
                Ok(reply) => {
                    self.opencode_cleanups.push(reply);
                    self.notify(Notification::info(
                        "OpenCode cleanup",
                        format!(
                            "Background cleanup task started for {}. Folder files stay untouched.",
                            super::display_directory(directory)
                        ),
                    ));
                    self.handle_message(Msg::Close, ctx);
                    self.update_snapshot(self.snapshot.clone());
                    ctx.request_layout();
                    ctx.request_redraw();
                }
                Err(error) => ctx.notify(Notification::error("Cannot clear OpenCode data", error)),
            }
            return true;
        }
        let (result, title) = match &self.intent {
            Some(Intent::RenameOpencodeSession(id)) => (
                self.service.rename_opencode_session(id, self.name.clone()),
                "Cannot rename OpenCode session",
            ),
            Some(Intent::DeleteOpencodeSession(id)) => (
                self.service.delete_opencode_session(id),
                "Cannot delete OpenCode session",
            ),
            _ => return false,
        };
        match result {
            Ok(reply) => {
                self.opencode_action = Some(PendingAction::new(reply, title, Vec::new()));
                self.handle_message(Msg::Close, ctx);
            }
            Err(error) => ctx.notify(Notification::error(title, error)),
        }
        true
    }

    pub(in crate::app) fn poll_opencode_cleanups(&mut self) {
        self.opencode_cleanups.retain_mut(|reply| {
            matches!(
                reply.try_recv(),
                Err(tokio::sync::oneshot::error::TryRecvError::Empty)
            )
        });
    }
}
