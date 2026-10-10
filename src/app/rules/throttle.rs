use std::{cell::RefCell, rc::Rc, time::Duration};

use ratatui::{Frame, layout::Rect};
use tuicore::{
    AnimationSettings, EventCtx, EventOutcome, EventRoute, FocusCtx, FocusTarget, FormField,
    LayoutCtx, LayoutProposal, LayoutResult, LayoutSizeHint, LifecycleCtx, RenderCtx, TextInput,
    TickResult, TuiEvent, TuiNode,
};

use super::Draft;
use crate::app::Msg;

pub(super) struct ThrottleField {
    field: FormField<TextInput<Msg>, Msg>,
    draft: Rc<RefCell<Draft>>,
}

impl ThrottleField {
    pub(super) fn new(draft: Rc<RefCell<Draft>>) -> Self {
        let changed = draft.clone();
        let input = TextInput::new()
            .numbers_only(true)
            .value(&draft.borrow().throttle_input)
            .on_change(move |value| {
                let mut draft = changed.borrow_mut();
                draft.throttle_error = if value.is_empty() {
                    Some("Enter seconds (0 = off)".into())
                } else {
                    match value.parse::<u32>() {
                        Ok(seconds) => {
                            draft.rule.definition.throttle_seconds = seconds;
                            None
                        }
                        Err(_) => Some("Enter seconds from 0 to 4294967295".into()),
                    }
                };
                draft.throttle_input = value;
                Msg::RuleDraftChanged(changed.clone())
            });
        let mut field = Self {
            field: FormField::new("Throttle seconds (0 = off)", input),
            draft,
        };
        field.sync_error();
        field
    }

    fn sync_error(&mut self) -> bool {
        let draft = self.draft.borrow();
        if self.field.error() == draft.throttle_error.as_deref() {
            return false;
        }
        self.field.set_error(draft.throttle_error.clone());
        true
    }
}

impl TuiNode<Msg> for ThrottleField {
    fn measure(&self, proposal: LayoutProposal) -> LayoutSizeHint {
        self.field.measure(proposal)
    }
    fn layout(&mut self, area: Rect, ctx: &mut LayoutCtx) -> LayoutResult {
        self.field.layout(area, ctx)
    }
    fn render<'a>(&'a self, frame: &mut Frame, area: Rect, ctx: &mut RenderCtx<'a>) {
        self.field.render(frame, area, ctx);
    }
    fn event(&mut self, event: &TuiEvent, ctx: &mut EventCtx<Msg>) -> EventOutcome {
        self.field.event(event, ctx)
    }
    fn dispatch_event(
        &mut self,
        route: &EventRoute,
        event: &TuiEvent,
        ctx: &mut EventCtx<Msg>,
    ) -> EventOutcome {
        self.field.dispatch_event(route, event, ctx)
    }
    fn tick(&mut self, dt: Duration, settings: AnimationSettings) -> TickResult {
        let changed = self.sync_error();
        self.field.tick(dt, settings).merge(TickResult {
            changed,
            layout: changed,
            ..TickResult::IDLE
        })
    }
    fn dispatch_focus(&mut self, target: &FocusTarget, focused: bool, ctx: &mut FocusCtx<Msg>) {
        self.field.dispatch_focus(target, focused, ctx);
    }
    fn focus_reveal_area(&self, target: &FocusTarget) -> Option<Rect> {
        self.field.focus_reveal_area(target)
    }
    fn init(&mut self, ctx: &mut LifecycleCtx<Msg>) {
        self.field.init(ctx);
    }
    fn mount(&mut self, ctx: &mut LifecycleCtx<Msg>) {
        self.field.mount(ctx);
    }
    fn unmount(&mut self, ctx: &mut LifecycleCtx<Msg>) {
        self.field.unmount(ctx);
    }
    fn destroy(&mut self, ctx: &mut LifecycleCtx<Msg>) {
        self.field.destroy(ctx);
    }
}
