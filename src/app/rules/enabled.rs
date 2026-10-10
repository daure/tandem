use std::{cell::RefCell, rc::Rc, time::Duration};

use ratatui::{Frame, layout::Rect};
use tuicore::{
    AnimationSettings, EventCtx, EventOutcome, EventRoute, FocusCtx, FocusId, FocusTarget,
    LayoutCtx, LayoutProposal, LayoutResult, LayoutSizeHint, LifecycleCtx, RenderCtx, TickResult,
    Toggle, TuiEvent, TuiNode,
};

use super::{Draft, PendingToggle};
use crate::app::Msg;

pub(super) struct SettingsToggle {
    toggle: Toggle<Msg>,
    draft: Rc<RefCell<Draft>>,
    value: fn(&Draft) -> bool,
}

impl SettingsToggle {
    pub(super) fn enabled(draft: Rc<RefCell<Draft>>) -> Self {
        let changed = draft.clone();
        let value: fn(&Draft) -> bool = |draft| match draft.pending_toggle {
            Some(PendingToggle::Enabled(enabled)) => enabled,
            _ => draft.rule.definition.enabled,
        };
        let toggle = Toggle::new("Active")
            .checked(value(&draft.borrow()))
            .on_change(move |enabled| Msg::RuleDraftEnabled(changed.clone(), enabled));
        Self {
            toggle,
            draft,
            value,
        }
    }

    pub(super) fn start_instance(draft: Rc<RefCell<Draft>>) -> Self {
        let changed = draft.clone();
        let value: fn(&Draft) -> bool = |draft| match draft.pending_toggle {
            Some(PendingToggle::StartInstance(start)) => start,
            _ => draft.rule.definition.start_instance,
        };
        let toggle = Toggle::new("Start instance")
            .checked(value(&draft.borrow()))
            .on_change(move |start| Msg::RuleDraftStartInstance(changed.clone(), start));
        Self {
            toggle,
            draft,
            value,
        }
    }

    pub(super) fn focus_pane(draft: Rc<RefCell<Draft>>) -> Self {
        let changed = draft.clone();
        let value: fn(&Draft) -> bool = |draft| match draft.pending_toggle {
            Some(PendingToggle::FocusPane(focus)) => focus,
            _ => draft.rule.definition.focus_pane,
        };
        let toggle = Toggle::new("Focus new pane")
            .checked(value(&draft.borrow()))
            .on_change(move |focus| Msg::RuleDraftFocusPane(changed.clone(), focus));
        Self {
            toggle,
            draft,
            value,
        }
    }

    pub(super) fn trigger_at_end(draft: Rc<RefCell<Draft>>) -> Self {
        let changed = draft.clone();
        let value: fn(&Draft) -> bool = |draft| match draft.pending_toggle {
            Some(PendingToggle::TriggerAtEnd(trigger)) => trigger,
            _ => draft.rule.definition.trigger_at_end,
        };
        let toggle = Toggle::new("Trigger at end")
            .checked(value(&draft.borrow()))
            .on_change(move |trigger| Msg::RuleDraftTriggerAtEnd(changed.clone(), trigger));
        Self {
            toggle,
            draft,
            value,
        }
    }
}

impl TuiNode<Msg> for SettingsToggle {
    fn measure(&self, proposal: LayoutProposal) -> LayoutSizeHint {
        self.toggle.measure(proposal)
    }
    fn layout(&mut self, area: Rect, ctx: &mut LayoutCtx) -> LayoutResult {
        self.toggle.layout(area, ctx)
    }
    fn render<'a>(&'a self, frame: &mut Frame, area: Rect, ctx: &mut RenderCtx<'a>) {
        <Toggle<Msg> as TuiNode<Msg>>::render(&self.toggle, frame, area, ctx);
    }
    fn event(&mut self, event: &TuiEvent, ctx: &mut EventCtx<Msg>) -> EventOutcome {
        self.toggle.event(event, ctx)
    }
    fn dispatch_event(
        &mut self,
        route: &EventRoute,
        event: &TuiEvent,
        ctx: &mut EventCtx<Msg>,
    ) -> EventOutcome {
        self.toggle.dispatch_event(route, event, ctx)
    }
    fn tick(&mut self, dt: Duration, settings: AnimationSettings) -> TickResult {
        let value = (self.value)(&self.draft.borrow());
        let changed = self.toggle.is_checked() != value;
        if changed {
            self.toggle.set_value(value);
        }
        let result = self.toggle.tick(dt, settings);
        if changed {
            result.merge(TickResult::CHANGED)
        } else {
            result
        }
    }
    fn focus(&mut self, target: Option<&FocusId>, focused: bool, ctx: &mut FocusCtx<Msg>) {
        self.toggle.focus(target, focused, ctx);
    }
    fn dispatch_focus(&mut self, target: &FocusTarget, focused: bool, ctx: &mut FocusCtx<Msg>) {
        self.toggle.dispatch_focus(target, focused, ctx);
    }
    fn focus_reveal_area(&self, target: &FocusTarget) -> Option<Rect> {
        self.toggle.focus_reveal_area(target)
    }
    fn init(&mut self, ctx: &mut LifecycleCtx<Msg>) {
        self.toggle.init(ctx);
    }
    fn mount(&mut self, ctx: &mut LifecycleCtx<Msg>) {
        self.toggle.mount(ctx);
    }
    fn unmount(&mut self, ctx: &mut LifecycleCtx<Msg>) {
        self.toggle.unmount(ctx);
    }
    fn destroy(&mut self, ctx: &mut LifecycleCtx<Msg>) {
        self.toggle.destroy(ctx);
    }
}
