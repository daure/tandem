use std::{cell::RefCell, rc::Rc, time::Duration};

use ratatui::{Frame, layout::Rect};
use tuicore::{
    AnimationSettings, Dropdown, EventCtx, EventOutcome, EventRoute, FocusCtx, FocusId,
    FocusTarget, LayoutCtx, LayoutProposal, LayoutResult, LayoutSizeHint, LifecycleCtx, RenderCtx,
    TickResult, TuiEvent, TuiNode,
};

use crate::{app::Msg, store::completion::SoundChoice};

type Picker = Dropdown<SoundChoice, String>;

pub(super) struct SoundDropdown {
    picker: Picker,
    pending: Rc<RefCell<Option<Msg>>>,
    selected_message: fn(String) -> Msg,
}

impl SoundDropdown {
    pub(super) fn new(
        picker: Picker,
        pending: Rc<RefCell<Option<Msg>>>,
        selected_message: fn(String) -> Msg,
    ) -> Self {
        Self {
            picker,
            pending,
            selected_message,
        }
    }

    fn state(&self) -> (bool, Option<String>) {
        (self.picker.is_open(), self.picker.selected_id())
    }

    fn transition(&self, previous: (bool, Option<String>)) -> Option<Msg> {
        let (open, selected) = self.state();
        if open == previous.0 {
            return None;
        }
        let value = selected.clone().unwrap_or_default();
        Some(if !open && selected != previous.1 {
            (self.selected_message)(value)
        } else {
            Msg::PreviewSound(value)
        })
    }
}

impl TuiNode<Msg> for SoundDropdown {
    fn measure(&self, proposal: LayoutProposal) -> LayoutSizeHint {
        <Picker as TuiNode<Msg>>::measure(&self.picker, proposal)
    }

    fn layout(&mut self, area: Rect, ctx: &mut LayoutCtx) -> LayoutResult {
        <Picker as TuiNode<Msg>>::layout(&mut self.picker, area, ctx)
    }

    fn render<'a>(&'a self, frame: &mut Frame, area: Rect, ctx: &mut RenderCtx<'a>) {
        <Picker as TuiNode<Msg>>::render(&self.picker, frame, area, ctx);
    }

    fn event(&mut self, event: &TuiEvent, ctx: &mut EventCtx<Msg>) -> EventOutcome {
        let previous = self.state();
        let outcome = self.picker.event(event, ctx);
        if let Some(message) = self.transition(previous) {
            *self.pending.borrow_mut() = Some(message);
        }
        outcome
    }

    fn dispatch_event(
        &mut self,
        route: &EventRoute,
        event: &TuiEvent,
        ctx: &mut EventCtx<Msg>,
    ) -> EventOutcome {
        let previous = self.state();
        let outcome = self.picker.dispatch_event(route, event, ctx);
        if let Some(message) = self.transition(previous) {
            *self.pending.borrow_mut() = Some(message);
        }
        outcome
    }

    fn tick(&mut self, dt: Duration, settings: AnimationSettings) -> TickResult {
        <Picker as TuiNode<Msg>>::tick(&mut self.picker, dt, settings)
    }

    fn focus(&mut self, target: Option<&FocusId>, focused: bool, ctx: &mut FocusCtx<Msg>) {
        let previous = self.state();
        self.picker.focus(target, focused, ctx);
        if let Some(message) = self.transition(previous) {
            ctx.emit(message);
        }
    }

    fn dispatch_focus(&mut self, target: &FocusTarget, focused: bool, ctx: &mut FocusCtx<Msg>) {
        let previous = self.state();
        self.picker.dispatch_focus(target, focused, ctx);
        if let Some(message) = self.transition(previous) {
            ctx.emit(message);
        }
    }

    fn focus_reveal_area(&self, target: &FocusTarget) -> Option<Rect> {
        <Picker as TuiNode<Msg>>::focus_reveal_area(&self.picker, target)
    }

    fn focus_reveal_centered(&self, target: &FocusTarget) -> bool {
        <Picker as TuiNode<Msg>>::focus_reveal_centered(&self.picker, target)
    }

    fn init(&mut self, ctx: &mut LifecycleCtx<Msg>) {
        self.picker.init(ctx);
    }

    fn mount(&mut self, ctx: &mut LifecycleCtx<Msg>) {
        self.picker.mount(ctx);
    }

    fn unmount(&mut self, ctx: &mut LifecycleCtx<Msg>) {
        self.picker.unmount(ctx);
    }

    fn destroy(&mut self, ctx: &mut LifecycleCtx<Msg>) {
        self.picker.destroy(ctx);
    }
}
