use std::time::Duration;

use ratatui::{Frame, layout::Rect};
use tuicore::{
    AnimationSettings, Button, ChildKey, EventCtx, EventOutcome, EventRoute, FocusCtx, FocusId,
    FocusTarget, HotkeyLabelMode, KeySpec, LayoutCtx, LayoutProposal, LayoutResult, LayoutSizeHint,
    LifecycleCtx, RenderCtx, TickResult, TuiEvent, TuiNode,
};

use super::{MOBILE_TABS_WIDTH, Msg};

pub(super) struct Toolbar {
    template: Button<Msg>,
    refresh: Button<Msg>,
    refresh_key: KeySpec,
    template_area: Rect,
    refresh_area: Rect,
}

fn hotkey(key: char) -> String {
    if key.is_ascii_uppercase() {
        format!("shift+{}", key.to_ascii_lowercase())
    } else {
        key.to_string()
    }
}

impl Toolbar {
    pub(super) fn new(template_key: char, refresh_key: char) -> Self {
        Self {
            template: Button::new("Template")
                .hotkey(hotkey(template_key))
                .hotkey_label_mode(HotkeyLabelMode::Inline)
                .on_press(|| Msg::NewTemplate),
            refresh: Button::new("󰑓 Refresh")
                .hotkey(hotkey(refresh_key))
                .on_press(|| Msg::Refresh),
            refresh_key: if refresh_key.is_ascii_uppercase() {
                KeySpec::shifted(refresh_key.to_ascii_lowercase())
            } else {
                KeySpec::plain(refresh_key)
            },
            template_area: Rect::default(),
            refresh_area: Rect::default(),
        }
    }

    fn refresh_hotkey(&self, event: &TuiEvent, ctx: &mut EventCtx<Msg>) -> bool {
        if let TuiEvent::Key(key) = event
            && self.refresh_key.matches(*key)
        {
            ctx.emit(Msg::Refresh);
            ctx.stop_propagation();
            return true;
        }
        false
    }
}

impl TuiNode<Msg> for Toolbar {
    fn measure(&self, proposal: LayoutProposal) -> LayoutSizeHint {
        let width = self
            .template
            .measure(proposal)
            .preferred
            .width
            .saturating_add(self.refresh.measure(proposal).preferred.width)
            .saturating_add(1);
        LayoutSizeHint::content(width, 1).normalized(proposal)
    }

    fn layout(&mut self, area: Rect, ctx: &mut LayoutCtx) -> LayoutResult {
        let compact = area.width < MOBILE_TABS_WIDTH;
        self.refresh.set_label(if compact {
            format!("󰑓 {}", self.refresh_key.label())
        } else {
            "󰑓 Refresh".into()
        });
        let proposal = LayoutProposal::at_most(area.width, area.height.min(1));
        let refresh_width = self
            .refresh
            .measure(proposal)
            .preferred
            .width
            .min(area.width);
        let template_width = self
            .template
            .measure(proposal)
            .preferred
            .width
            .min(area.width.saturating_sub(refresh_width.saturating_add(1)));
        self.template_area = Rect::new(area.x, area.y, template_width, area.height.min(1));
        self.refresh_area = Rect::new(
            area.right().saturating_sub(refresh_width),
            area.y,
            refresh_width,
            area.height.min(1),
        );
        ctx.push_slot(ChildKey::new("new-template"), self.template_area, |ctx| {
            self.template.layout(self.template_area, ctx)
        });
        ctx.push_slot(ChildKey::new("refresh"), self.refresh_area, |ctx| {
            self.refresh.layout(self.refresh_area, ctx)
        });
        LayoutResult::new(area)
    }

    fn render<'a>(&'a self, frame: &mut Frame, _area: Rect, ctx: &mut RenderCtx<'a>) {
        <Button<Msg> as TuiNode<Msg>>::render(&self.template, frame, self.template_area, ctx);
        <Button<Msg> as TuiNode<Msg>>::render(&self.refresh, frame, self.refresh_area, ctx);
    }

    fn event(&mut self, event: &TuiEvent, ctx: &mut EventCtx<Msg>) -> EventOutcome {
        if self.refresh_hotkey(event, ctx)
            || self.template.event(event, ctx) == EventOutcome::Handled
        {
            return EventOutcome::Handled;
        }
        self.refresh.event(event, ctx)
    }

    fn dispatch_event(
        &mut self,
        route: &EventRoute,
        event: &TuiEvent,
        ctx: &mut EventCtx<Msg>,
    ) -> EventOutcome {
        if self.refresh_hotkey(event, ctx) {
            return EventOutcome::Handled;
        }
        if let Some(path) = route.path.without_first_if(&ChildKey::new("new-template")) {
            return self
                .template
                .dispatch_event(&EventRoute::new(path), event, ctx);
        }
        if let Some(path) = route.path.without_first_if(&ChildKey::new("refresh")) {
            return self
                .refresh
                .dispatch_event(&EventRoute::new(path), event, ctx);
        }
        EventOutcome::Ignored
    }

    fn tick(&mut self, dt: Duration, settings: AnimationSettings) -> TickResult {
        <Button<Msg> as TuiNode<Msg>>::tick(&mut self.template, dt, settings).merge(
            <Button<Msg> as TuiNode<Msg>>::tick(&mut self.refresh, dt, settings),
        )
    }

    fn focus(&mut self, target: Option<&FocusId>, focused: bool, ctx: &mut FocusCtx<Msg>) {
        self.template.focus(target, focused, ctx);
        self.refresh.focus(target, focused, ctx);
    }

    fn dispatch_focus(&mut self, target: &FocusTarget, focused: bool, ctx: &mut FocusCtx<Msg>) {
        if let Some(target) = target.for_child(&ChildKey::new("new-template")) {
            self.template.dispatch_focus(&target, focused, ctx);
        }
        if let Some(target) = target.for_child(&ChildKey::new("refresh")) {
            self.refresh.dispatch_focus(&target, focused, ctx);
        }
    }

    fn init(&mut self, ctx: &mut LifecycleCtx<Msg>) {
        self.template.init(ctx);
        self.refresh.init(ctx);
    }
    fn mount(&mut self, ctx: &mut LifecycleCtx<Msg>) {
        self.template.mount(ctx);
        self.refresh.mount(ctx);
    }
    fn unmount(&mut self, ctx: &mut LifecycleCtx<Msg>) {
        self.template.unmount(ctx);
        self.refresh.unmount(ctx);
    }
    fn destroy(&mut self, ctx: &mut LifecycleCtx<Msg>) {
        self.template.destroy(ctx);
        self.refresh.destroy(ctx);
    }
}
