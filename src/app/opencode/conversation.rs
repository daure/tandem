use std::time::Duration;

use ratatui::{Frame, layout::Rect};
use tuicore::{
    AnimationSettings, EventCtx, EventOutcome, EventRoute, FocusCtx, FocusId, FocusTarget,
    Language, LayoutCtx, LayoutProposal, LayoutResult, LayoutSizeHint, LifecycleCtx, RenderCtx,
    SyntaxHighlighter, TickResult, TuiEvent, TuiNode,
};

use crate::app::Msg;

type Reply = tokio::sync::oneshot::Receiver<Result<String, String>>;

pub(super) struct Conversation {
    text: SyntaxHighlighter,
    pending: Option<Reply>,
}

impl Conversation {
    pub(super) fn new(reply: Result<Reply, String>) -> Self {
        let (text, pending) = match reply {
            Ok(reply) => ("Loading conversation…".into(), Some(reply)),
            Err(error) => (format!("Unable to load conversation\n\n{error}"), None),
        };
        Self {
            text: SyntaxHighlighter::new(text, Language::guess(Some("conversation.md"), ""))
                .wrap(true),
            pending,
        }
    }
}

impl TuiNode<Msg> for Conversation {
    fn measure(&self, proposal: LayoutProposal) -> LayoutSizeHint {
        <SyntaxHighlighter as TuiNode<Msg>>::measure(&self.text, proposal)
    }
    fn layout(&mut self, area: Rect, ctx: &mut LayoutCtx) -> LayoutResult {
        <SyntaxHighlighter as TuiNode<Msg>>::layout(&mut self.text, area, ctx)
    }
    fn render<'a>(&'a self, frame: &mut Frame, area: Rect, ctx: &mut RenderCtx<'a>) {
        <SyntaxHighlighter as TuiNode<Msg>>::render(&self.text, frame, area, ctx);
    }
    fn event(&mut self, event: &TuiEvent, ctx: &mut EventCtx<Msg>) -> EventOutcome {
        self.text.event(event, ctx)
    }
    fn dispatch_event(
        &mut self,
        route: &EventRoute,
        event: &TuiEvent,
        ctx: &mut EventCtx<Msg>,
    ) -> EventOutcome {
        self.text.dispatch_event(route, event, ctx)
    }
    fn tick(&mut self, dt: Duration, settings: AnimationSettings) -> TickResult {
        let mut result = <SyntaxHighlighter as TuiNode<Msg>>::tick(&mut self.text, dt, settings);
        let reply = self
            .pending
            .as_mut()
            .and_then(|reply| match reply.try_recv() {
                Ok(result) => Some(result),
                Err(tokio::sync::oneshot::error::TryRecvError::Empty) => None,
                Err(tokio::sync::oneshot::error::TryRecvError::Closed) => {
                    Some(Err("Conversation loading was cancelled".into()))
                }
            });
        if let Some(reply) = reply {
            self.pending = None;
            self.text.set_code(
                reply.unwrap_or_else(|error| format!("Unable to load conversation\n\n{error}")),
            );
            result.changed = true;
            result.layout = true;
        }
        if self.pending.is_some() {
            result = result.merge(TickResult::scheduled_after(Duration::from_millis(100)));
        }
        result
    }
    fn focus(&mut self, target: Option<&FocusId>, focused: bool, ctx: &mut FocusCtx<Msg>) {
        self.text.focus(target, focused, ctx);
    }
    fn dispatch_focus(&mut self, target: &FocusTarget, focused: bool, ctx: &mut FocusCtx<Msg>) {
        self.text.dispatch_focus(target, focused, ctx);
    }
    fn focus_reveal_area(&self, target: &FocusTarget) -> Option<Rect> {
        <SyntaxHighlighter as TuiNode<Msg>>::focus_reveal_area(&self.text, target)
    }
    fn init(&mut self, ctx: &mut LifecycleCtx<Msg>) {
        self.text.init(ctx);
    }
    fn mount(&mut self, ctx: &mut LifecycleCtx<Msg>) {
        self.text.mount(ctx);
    }
    fn unmount(&mut self, ctx: &mut LifecycleCtx<Msg>) {
        self.text.unmount(ctx);
    }
    fn destroy(&mut self, ctx: &mut LifecycleCtx<Msg>) {
        self.pending = None;
        self.text.destroy(ctx);
    }
}

#[cfg(test)]
#[path = "../tests/opencode_conversation.rs"]
mod tests;
