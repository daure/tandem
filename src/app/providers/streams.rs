use tuicore::EventCtx;

use crate::{
    app::{App, Msg, dialogs},
    store::providers::{Action, Stream},
};

impl App {
    pub(in crate::app) fn request_provider_stream_action(
        &mut self,
        name: String,
        stream: String,
        action: Action,
        ctx: &mut EventCtx<Msg>,
    ) {
        let reason =
            if self.provider_actions.iter().any(|pending| {
                pending.name == name || pending.name.starts_with(&format!("{name}/"))
            }) {
                Some("A provider or stream operation is already in progress")
            } else {
                self.pages_mut()
                    .provider_stream_action_unavailable(&name, &stream, action)
            };
        if let Some(reason) = reason {
            self.notify(tuicore::Notification::warning(
                "Stream action unavailable",
                reason,
            ));
            return;
        }
        let dialog = dialogs::confirmation(
            &format!("{action:?} stream"),
            format!("{action:?} {name}/{stream}? Resume collects live events only."),
        );
        self.provider_confirmation = None;
        self.provider_bulk_confirmation = None;
        self.provider_stream_confirmation = Some((name, stream, action));
        self.intent = None;
        self.open_compact(dialog, ctx);
    }

    pub(in crate::app) fn start_provider_stream_action(
        &mut self,
        name: String,
        stream: String,
        action: Action,
    ) {
        let receiver =
            self.service
                .provider_stream_action(name.clone(), stream.clone(), action, true);
        self.provider_actions.push(super::PendingAction {
            name: format!("{name}/{stream}"),
            action,
            receiver,
        });
    }

    pub(in crate::app) fn open_provider_stream_details(
        &mut self,
        name: &str,
        stream: &Stream,
        ctx: &mut EventCtx<Msg>,
    ) {
        let text = serde_json::to_string_pretty(
            &serde_json::json!({"provider": name, "stream": stream, "resume_policy": "live_only"}),
        )
        .unwrap_or_default();
        self.open_provider_text("Stream details", text, ctx);
    }
}
