use std::{cell::RefCell, rc::Rc, time::Duration};

use ratatui::{Frame, layout::Rect};
use tuicore::{
    AnimationSettings, EventCtx, EventOutcome, EventRoute, FocusCtx, FocusTarget, FormField,
    LayoutCtx, LayoutProposal, LayoutResult, LayoutSizeHint, LifecycleCtx, RenderCtx,
    TextareaInput, TickResult, TuiEvent, TuiNode,
};

use super::Draft;
use crate::app::Msg;

pub(super) struct PromptField {
    field: FormField<TextareaInput<Msg>, Msg>,
    draft: Rc<RefCell<Draft>>,
}

impl PromptField {
    pub(super) fn new(input: TextareaInput<Msg>, draft: Rc<RefCell<Draft>>) -> Self {
        let mut field = Self {
            field: FormField::new("Initial prompt", input),
            draft,
        };
        field.sync_error();
        field
    }

    fn sync_error(&mut self) -> bool {
        let draft = self.draft.borrow();
        let error = draft
            .prompt_error
            .as_deref()
            .and_then(|error| error.lines().next());
        if self.field.error() == error {
            return false;
        }
        self.field.set_error(error.map(str::to_owned));
        true
    }
}

impl TuiNode<Msg> for PromptField {
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
