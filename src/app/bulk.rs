use tuicore::{EventCtx, Notification};

use super::{App, Intent, Msg, dialogs};

impl App {
    pub(super) fn confirm_stop_all(&mut self, ctx: &mut EventCtx<Msg>) {
        let targets = self.toolbar_state.borrow().stop_targets.clone();
        if targets.is_empty() {
            return;
        }
        let modal = dialogs::confirm_stop_all(targets.len(), self.keys[8]);
        self.intent = Some(Intent::StopAll(targets));
        self.open(modal, ctx);
    }

    pub(super) fn confirm_purge_all(&mut self, ctx: &mut EventCtx<Msg>) {
        let targets = self.toolbar_state.borrow().purge_targets.clone();
        if targets.is_empty() {
            return;
        }
        let modal = dialogs::confirm_purge_all(targets.len(), self.keys[9]);
        self.intent = Some(Intent::PurgeAll(targets));
        self.open(modal, ctx);
    }

    pub(super) fn submit_instance_batch(
        &mut self,
        action: &str,
        targets: Vec<String>,
        ctx: &mut EventCtx<Msg>,
    ) {
        let batch = match self.service.submit_instance_batch(action, &targets, true) {
            Ok(batch) => batch,
            Err(error) => {
                self.view.first_mut().layer_mut().set_bottom_left(error);
                ctx.request_redraw();
                return;
            }
        };
        if batch.operations.is_empty() && !batch.errors.is_empty() {
            self.view
                .first_mut()
                .layer_mut()
                .set_bottom_left(batch.errors.join("; "));
            ctx.request_redraw();
            return;
        }
        for operation in batch.operations {
            self.operation_accepted(operation);
        }
        if !batch.errors.is_empty() {
            self.notify(Notification::warning(
                "Some instances could not be queued",
                batch.errors.join("; "),
            ));
        }
        self.handle_message(Msg::Close, ctx);
        ctx.request_layout();
    }
}
