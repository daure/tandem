use std::time::Duration;

use ratatui::{Frame, layout::Rect};
use tokio::sync::oneshot;
use tuicore::{
    AnimationSettings, EventCtx, EventOutcome, EventRoute, FocusCtx, FocusId, FocusTarget,
    LayoutCtx, LayoutProposal, LayoutResult, LayoutSizeHint, LifecycleCtx, RenderCtx,
    SyntaxHighlighter, TickResult, TuiEvent, TuiNode,
};

use crate::{
    app::Msg,
    service::AppService,
    store::events::{
        Error,
        diagnostics::{Diagnostics, EvaluationOutcome},
    },
};

pub(super) struct Pane {
    view: SyntaxHighlighter,
    receiver: Option<oneshot::Receiver<Result<Diagnostics, Error>>>,
    text: String,
}

impl Pane {
    pub(super) fn new(service: AppService, sequence: i64) -> Self {
        Self {
            view: SyntaxHighlighter::new(
                "Loading event diagnostics…",
                tuicore::Language::PlainText,
            )
            .wrap(true),
            receiver: Some(service.event_diagnostics(sequence)),
            text: String::new(),
        }
    }

    fn poll(&mut self) -> bool {
        let Some(receiver) = &mut self.receiver else {
            return false;
        };
        let result = match receiver.try_recv() {
            Ok(result) => result,
            Err(oneshot::error::TryRecvError::Empty) => return false,
            Err(oneshot::error::TryRecvError::Closed) => {
                Err(Error::Storage("event worker stopped".into()))
            }
        };
        self.receiver = None;
        let text = match result {
            Ok(diagnostics) => format_diagnostics(&diagnostics),
            Err(error) => format!("Cannot read event diagnostics: {error}"),
        };
        let text = text
            .lines()
            .map(super::clean)
            .collect::<Vec<_>>()
            .join("\n");
        if text == self.text {
            return false;
        }
        self.view.set_code(text.clone());
        self.text = text;
        true
    }
}

fn format_diagnostics(diagnostics: &Diagnostics) -> String {
    let mut lines = vec![
        format!("Event #{} · {} · {}", diagnostics.sequence, diagnostics.provider, diagnostics.event_id),
        diagnostics.summary.clone(),
        format!("Evaluation/dispatch: {} errors · {} warnings", diagnostics.counts.errors, diagnostics.counts.warnings),
        "Recorded evaluations use their pinned rule revisions. Launched confirms session linkage, not task completion.".into(),
        "Reopen the dialog to refresh diagnostics.".into(),
    ];
    if diagnostics.attempts_truncated {
        lines.push(format!(
            "Showing the latest {} of {} attempts.",
            diagnostics.attempts.len(),
            diagnostics.attempts_total
        ));
    }
    for attempt in &diagnostics.attempts {
        lines.push(String::new());
        lines.push(format!(
            "Attempt #{}{} · {}",
            attempt.attempt.id,
            if attempt.attempt.replay {
                " (replay)"
            } else {
                ""
            },
            attempt.attempt.created_at
        ));
        if attempt.rules.is_empty() {
            lines.push("  No rule evaluations were recorded for this attempt.".into());
        }
        for rule in &attempt.rules {
            let outcome = match rule.outcome {
                EvaluationOutcome::Pending => "pending evaluation",
                EvaluationOutcome::Matched => "matched (predicate returned true)",
                EvaluationOutcome::NoMatch => "no match (predicate returned false)",
                EvaluationOutcome::Failed => "evaluation failed",
                EvaluationOutcome::Deferred => "pending: waiting for throttle timer",
                EvaluationOutcome::Throttled => "warning: throttled (match rejected)",
            };
            lines.push(format!(
                "  {} · revision {} · {outcome}",
                rule.rule.definition.name, rule.rule.revision
            ));
            if let Some(error) = &rule.error {
                lines.push(format!("    Error: {error}"));
            }
            if let Some(warning) = &rule.warning {
                lines.push(format!("    Warning: {warning}"));
            }
            if let Some(acceptance) = &rule.acceptance {
                lines.push(format!(
                    "    Acceptance #{} · {:?} · instance {}",
                    acceptance.id, acceptance.status, acceptance.instance
                ));
                if let Some(error) = &acceptance.error {
                    lines.push(format!("    Dispatch error: {error}"));
                }
                if let Some(session) = &acceptance.session_id {
                    lines.push(format!("    Session: {session}"));
                }
                if let Some(operation) = &acceptance.operation_id {
                    lines.push(format!("    Origin operation: {operation}"));
                }
            }
            if let Some(startup) = &rule.startup {
                lines.push(format!(
                    "    Retained startup {} · {:?}",
                    startup.operation_id, startup.state
                ));
                if let Some(error) = &startup.error {
                    lines.push(format!("    Startup error: {error}"));
                }
                for warning in &startup.warnings {
                    lines.push(format!("    Warning: {warning}"));
                }
                for progress in &startup.progress {
                    lines.push(format!("    {progress}"));
                }
                if startup.logs_truncated {
                    lines.push("    Retained logs truncated to the bounded tail.".into());
                }
            }
            if let Some(reason) = &rule.details_unavailable {
                lines.push(format!("    {reason}"));
            }
        }
    }
    lines.join("\n")
}

impl TuiNode<Msg> for Pane {
    fn measure(&self, proposal: LayoutProposal) -> LayoutSizeHint {
        <SyntaxHighlighter as TuiNode<Msg>>::measure(&self.view, proposal)
    }
    fn layout(&mut self, area: Rect, ctx: &mut LayoutCtx) -> LayoutResult {
        <SyntaxHighlighter as TuiNode<Msg>>::layout(&mut self.view, area, ctx)
    }
    fn render<'a>(&'a self, frame: &mut Frame, area: Rect, ctx: &mut RenderCtx<'a>) {
        <SyntaxHighlighter as TuiNode<Msg>>::render(&self.view, frame, area, ctx);
    }
    fn event(&mut self, event: &TuiEvent, ctx: &mut EventCtx<Msg>) -> EventOutcome {
        self.view.event(event, ctx)
    }
    fn dispatch_event(
        &mut self,
        route: &EventRoute,
        event: &TuiEvent,
        ctx: &mut EventCtx<Msg>,
    ) -> EventOutcome {
        self.view.dispatch_event(route, event, ctx)
    }
    fn tick(&mut self, dt: Duration, settings: AnimationSettings) -> TickResult {
        let changed = self.poll();
        let result = <SyntaxHighlighter as TuiNode<Msg>>::tick(&mut self.view, dt, settings);
        result
            .merge(if changed {
                TickResult {
                    layout: true,
                    ..TickResult::CHANGED
                }
            } else {
                TickResult::IDLE
            })
            .merge(if self.receiver.is_some() {
                TickResult::scheduled_after(Duration::from_millis(250))
            } else {
                TickResult::IDLE
            })
    }
    fn focus(&mut self, target: Option<&FocusId>, focused: bool, ctx: &mut FocusCtx<Msg>) {
        self.view.focus(target, focused, ctx);
    }
    fn dispatch_focus(&mut self, target: &FocusTarget, focused: bool, ctx: &mut FocusCtx<Msg>) {
        self.view.dispatch_focus(target, focused, ctx);
    }
    fn focus_reveal_area(&self, target: &FocusTarget) -> Option<Rect> {
        <SyntaxHighlighter as TuiNode<Msg>>::focus_reveal_area(&self.view, target)
    }
    fn focus_reveal_centered(&self, target: &FocusTarget) -> bool {
        <SyntaxHighlighter as TuiNode<Msg>>::focus_reveal_centered(&self.view, target)
    }
    fn take_pending_focus_request(&mut self) -> Option<tuicore::FocusRequest> {
        <SyntaxHighlighter as TuiNode<Msg>>::take_pending_focus_request(&mut self.view)
    }
    fn take_pending_clipboard_request(&mut self) -> Option<String> {
        <SyntaxHighlighter as TuiNode<Msg>>::take_pending_clipboard_request(&mut self.view)
    }
    fn init(&mut self, ctx: &mut LifecycleCtx<Msg>) {
        <SyntaxHighlighter as TuiNode<Msg>>::init(&mut self.view, ctx);
    }
    fn mount(&mut self, ctx: &mut LifecycleCtx<Msg>) {
        <SyntaxHighlighter as TuiNode<Msg>>::mount(&mut self.view, ctx);
        ctx.request_tick();
    }
    fn unmount(&mut self, ctx: &mut LifecycleCtx<Msg>) {
        <SyntaxHighlighter as TuiNode<Msg>>::unmount(&mut self.view, ctx);
    }
    fn destroy(&mut self, ctx: &mut LifecycleCtx<Msg>) {
        <SyntaxHighlighter as TuiNode<Msg>>::destroy(&mut self.view, ctx);
    }
}

#[cfg(test)]
#[path = "tests/diagnostics.rs"]
mod tests;
