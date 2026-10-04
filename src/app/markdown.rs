use std::time::Duration;

use ratatui::{Frame, layout::Rect};
use tuicore::{
    AnimationSettings, EventCtx, EventOutcome, EventRoute, FocusCtx, FocusId, FocusTarget,
    Language, LayoutCtx, LayoutProposal, LayoutResult, LayoutSizeHint, LifecycleCtx, RenderCtx,
    SyntaxHighlighter, TextareaInput, TickResult, TuiEvent, TuiNode,
};

use crate::app::Msg;

type Reply<T> = tokio::sync::oneshot::Receiver<Result<T, String>>;

pub(super) struct Markdown<T, V = SyntaxHighlighter> {
    text: V,
    pending: Option<Reply<T>>,
    label: &'static str,
    document: fn(T) -> String,
    set_document: fn(&mut V, String),
}

impl<T> Markdown<T> {
    pub(super) fn new(
        label: &'static str,
        reply: Result<Reply<T>, String>,
        document: fn(T) -> String,
    ) -> Self {
        let (text, pending) = match reply {
            Ok(reply) => (format!("Loading {label}…"), Some(reply)),
            Err(error) => (format!("Unable to load {label}\n\n{error}"), None),
        };
        Self {
            text: SyntaxHighlighter::new(text, Language::guess(Some("conversation.md"), ""))
                .wrap(true),
            pending,
            label,
            document,
            set_document: SyntaxHighlighter::set_code,
        }
    }
}

impl<T> Markdown<T, TextareaInput<Msg>> {
    pub(super) fn readonly(
        label: &'static str,
        reply: Result<Reply<T>, String>,
        document: fn(T) -> String,
    ) -> Self {
        let (text, pending) = match reply {
            Ok(reply) => (format!("Loading {label}…"), Some(reply)),
            Err(error) => (format!("Unable to load {label}\n\n{error}"), None),
        };
        Self {
            text: TextareaInput::new()
                .panel("Report")
                .disabled(true)
                .fill_height(true)
                .language(Language::guess(Some("report.md"), ""))
                .value(text),
            pending,
            label,
            document,
            set_document: TextareaInput::set_value,
        }
    }
}

impl<T, V: TuiNode<Msg>> TuiNode<Msg> for Markdown<T, V> {
    fn measure(&self, proposal: LayoutProposal) -> LayoutSizeHint {
        self.text.measure(proposal)
    }
    fn layout(&mut self, area: Rect, ctx: &mut LayoutCtx) -> LayoutResult {
        self.text.layout(area, ctx)
    }
    fn render<'a>(&'a self, frame: &mut Frame, area: Rect, ctx: &mut RenderCtx<'a>) {
        self.text.render(frame, area, ctx);
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
        let mut result = self.text.tick(dt, settings);
        let reply = self
            .pending
            .as_mut()
            .and_then(|reply| match reply.try_recv() {
                Ok(result) => Some(result),
                Err(tokio::sync::oneshot::error::TryRecvError::Empty) => None,
                Err(tokio::sync::oneshot::error::TryRecvError::Closed) => {
                    Some(Err(format!("{} loading was cancelled", self.label)))
                }
            });
        if let Some(reply) = reply {
            self.pending = None;
            (self.set_document)(
                &mut self.text,
                reply
                    .map(self.document)
                    .unwrap_or_else(|error| format!("Unable to load {}\n\n{error}", self.label)),
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
        self.text.focus_reveal_area(target)
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
#[path = "tests/markdown.rs"]
mod tests;
