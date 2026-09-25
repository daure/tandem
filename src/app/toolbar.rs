use std::{cell::RefCell, rc::Rc, time::Duration};

use ratatui::{
    Frame,
    layout::Rect,
    style::Style,
    text::{Line, Span},
};
use tuicore::{
    Animated, AnimationSettings, Button, ChildKey, EventCtx, EventOutcome, EventRoute, FocusCtx,
    FocusId, FocusTarget, HotkeyLabelMode, KeySpec, LayoutCtx, LayoutProposal, LayoutResult,
    LayoutSizeHint, LifecycleCtx, RenderCtx, Spinner, TickResult, Toggle, TuiEvent, TuiNode,
};

use super::{MOBILE_TABS_WIDTH, Msg, details, rows};
use crate::store::environments::{EnvironmentSnapshot, UsageSummary};

#[derive(Default)]
pub(super) struct State {
    pub totals: UsageSummary,
    pub available_memory_bytes: Option<u64>,
    pub stop_targets: Vec<String>,
    pub purge_targets: Vec<String>,
    pub running_only: bool,
}

impl State {
    pub(super) fn from_snapshot(snapshot: &EnvironmentSnapshot) -> Self {
        Self {
            totals: UsageSummary::instances(snapshot.instances.iter()),
            available_memory_bytes: snapshot.available_memory_bytes,
            stop_targets: snapshot
                .instances
                .iter()
                .filter(|instance| instance.can_stop())
                .map(|instance| instance.name.clone())
                .collect(),
            purge_targets: snapshot
                .instances
                .iter()
                .map(|instance| instance.name.clone())
                .collect(),
            running_only: false,
        }
    }
}

pub(super) type SharedState = Rc<RefCell<State>>;

pub(super) struct Toolbar {
    template: Button<Msg>,
    refresh: Button<Msg>,
    running: Toggle<Msg>,
    stop: Button<Msg>,
    purge: Button<Msg>,
    refresh_key: KeySpec,
    running_key: KeySpec,
    bulk_keys: [KeySpec; 2],
    template_area: Rect,
    refresh_area: Rect,
    running_area: Rect,
    stop_area: Rect,
    purge_area: Rect,
    state: SharedState,
    spinner: Spinner,
    totals_area: Rect,
    totals_width: usize,
}

fn hotkey(key: KeySpec) -> String {
    let label = key.label();
    if label.len() == 1 && label.as_bytes()[0].is_ascii_uppercase() {
        format!("shift+{}", label.to_ascii_lowercase())
    } else {
        label
    }
}

impl Toolbar {
    pub(super) fn new(
        template_key: KeySpec,
        refresh_key: KeySpec,
        stop_key: KeySpec,
        purge_key: KeySpec,
        state: SharedState,
    ) -> Self {
        let stop_disabled = state.borrow().stop_targets.is_empty();
        let purge_disabled = state.borrow().purge_targets.is_empty();
        let running_key = KeySpec::shifted('u');
        Self {
            template: Button::new("Template")
                .hotkey(hotkey(template_key))
                .hotkey_label_mode(HotkeyLabelMode::Inline)
                .on_press(|| Msg::NewTemplate),
            refresh: Button::new("󰑓 Refresh")
                .hotkey(hotkey(refresh_key))
                .on_press(|| Msg::Refresh),
            running: Toggle::new("󰑮")
                .hotkey(hotkey(running_key))
                .preserve_focus_on_hotkey(true)
                .on_change(Msg::SetRunningOnly),
            stop: Button::new(" Stop all")
                .hotkey(hotkey(stop_key))
                .hotkey_label_mode(HotkeyLabelMode::Inline)
                .on_press(|| Msg::StopAll)
                .disabled(stop_disabled),
            purge: Button::new(" Purge all")
                .hotkey(hotkey(purge_key))
                .hotkey_label_mode(HotkeyLabelMode::Inline)
                .on_press(|| Msg::PurgeAll)
                .disabled(purge_disabled),
            refresh_key,
            running_key,
            bulk_keys: [stop_key, purge_key],
            template_area: Rect::default(),
            refresh_area: Rect::default(),
            running_area: Rect::default(),
            stop_area: Rect::default(),
            purge_area: Rect::default(),
            state,
            spinner: Spinner::new(),
            totals_area: Rect::default(),
            totals_width: 0,
        }
    }

    fn totals_text(&self) -> Line<'static> {
        let state = self.state.borrow();
        let totals = &state.totals;
        let waiting = totals.memory_waiting || totals.cpu_waiting;
        let complete = totals.memory_bytes.is_some() && totals.cpu_basis_points.is_some();
        if waiting && !complete {
            return Line::from(Span::styled(
                self.spinner.glyph().to_owned(),
                Style::default().fg(tuicore::theme().muted_fg()),
            ));
        }
        let mut resources =
            rows::resource_text_with_single_spinner(totals, None, self.spinner.glyph()).lines;
        let mut cpu = resources.pop().unwrap_or_default();
        let memory = resources.pop().unwrap_or_default();
        if totals.cpu_basis_points.is_none() {
            for span in &mut cpu.spans {
                let content = span.content.trim_start_matches('—').trim_start();
                let content = content
                    .strip_prefix('·')
                    .map(str::trim_start)
                    .unwrap_or(content);
                span.content = content.to_owned().into();
            }
            cpu.spans.retain(|span| !span.content.is_empty());
        }
        let memory_style = memory
            .spans
            .first()
            .map(|span| span.style)
            .unwrap_or_default();
        let mut spans = vec![
            Span::raw(" "),
            Span::raw(
                state
                    .available_memory_bytes
                    .map(details::memory)
                    .unwrap_or_else(|| "—".into()),
            ),
        ];
        spans.push(Span::raw(" · "));
        spans.push(Span::styled(" ", memory_style));
        spans.extend(memory.spans);
        if !cpu.spans.is_empty() {
            spans.push(Span::raw(" · "));
            spans.extend(cpu.spans);
        }
        Line::from(spans)
    }

    fn buttons_mut(&mut self) -> [(&'static str, &mut Button<Msg>); 4] {
        [
            ("new-template", &mut self.template),
            ("stop-all", &mut self.stop),
            ("purge-all", &mut self.purge),
            ("refresh", &mut self.refresh),
        ]
    }

    fn sync_disabled(&mut self) -> bool {
        let state = self.state.borrow();
        let stop_disabled = state.stop_targets.is_empty();
        let purge_disabled = state.purge_targets.is_empty();
        let running_only = state.running_only;
        let changed = self.stop.is_disabled() != stop_disabled
            || self.purge.is_disabled() != purge_disabled
            || self.running.is_checked() != running_only;
        self.stop.set_disabled(stop_disabled);
        self.purge.set_disabled(purge_disabled);
        if self.running.is_checked() != running_only {
            self.running.set_value(running_only);
        }
        changed
    }

    fn action_hotkey(&mut self, event: &TuiEvent, ctx: &mut EventCtx<Msg>) -> bool {
        let TuiEvent::Key(key) = event else {
            return false;
        };
        if self.running_key.matches(*key) {
            let outcome = self.running.toggle();
            if outcome.changed {
                ctx.emit(Msg::SetRunningOnly(outcome.value));
                ctx.request_redraw();
            }
            ctx.stop_propagation();
            return true;
        }
        for (binding, disabled, message) in [
            (self.refresh_key, false, Msg::Refresh),
            (self.bulk_keys[0], self.stop.is_disabled(), Msg::StopAll),
            (self.bulk_keys[1], self.purge.is_disabled(), Msg::PurgeAll),
        ] {
            if !disabled && binding.matches(*key) {
                ctx.emit(message);
                ctx.stop_propagation();
                return true;
            }
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
            .saturating_add(self.running.measure(proposal).preferred.width)
            .saturating_add(self.stop.measure(proposal).preferred.width)
            .saturating_add(self.purge.measure(proposal).preferred.width)
            .saturating_add(self.totals_text().width().min(u16::MAX as usize) as u16)
            .saturating_add(4);
        LayoutSizeHint::content(width, 1).normalized(proposal)
    }

    fn layout(&mut self, area: Rect, ctx: &mut LayoutCtx) -> LayoutResult {
        self.sync_disabled();
        let compact = area.width < MOBILE_TABS_WIDTH;
        self.stop
            .set_label(if compact { "" } else { " Stop all" });
        self.purge
            .set_label(if compact { "" } else { " Purge all" });
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
        let purge_width = self
            .purge
            .measure(proposal)
            .preferred
            .width
            .min(area.width.saturating_sub(refresh_width.saturating_add(1)));
        let stop_width = self.stop.measure(proposal).preferred.width.min(
            area.width
                .saturating_sub(refresh_width.saturating_add(purge_width).saturating_add(2)),
        );
        let running_width = self.running.measure(proposal).preferred.width.min(
            area.width.saturating_sub(
                refresh_width
                    .saturating_add(purge_width)
                    .saturating_add(stop_width)
                    .saturating_add(2),
            ),
        );
        let bulk_width = purge_width
            .saturating_add(stop_width)
            .saturating_add(running_width)
            .saturating_add(2);
        let template_width = self.template.measure(proposal).preferred.width.min(
            area.width
                .saturating_sub(bulk_width.saturating_add(refresh_width).saturating_add(1)),
        );
        self.template_area = Rect::new(area.x, area.y, template_width, area.height.min(1));
        self.refresh_area = Rect::new(
            area.right().saturating_sub(refresh_width),
            area.y,
            refresh_width,
            area.height.min(1),
        );
        self.purge_area = Rect::new(
            self.refresh_area
                .x
                .saturating_sub(purge_width.saturating_add(1))
                .max(area.x),
            area.y,
            purge_width,
            area.height.min(1),
        );
        self.stop_area = Rect::new(
            self.purge_area
                .x
                .saturating_sub(stop_width.saturating_add(1))
                .max(area.x),
            area.y,
            stop_width,
            area.height.min(1),
        );
        self.running_area = Rect::new(
            self.stop_area.x.saturating_sub(running_width).max(area.x),
            area.y,
            running_width,
            area.height.min(1),
        );
        self.totals_width = self.totals_text().width();
        let totals_width = self.totals_width.min(u16::MAX as usize) as u16;
        let available = self
            .running_area
            .x
            .saturating_sub(self.template_area.right().saturating_add(2));
        self.totals_area = if totals_width <= available {
            Rect::new(
                self.running_area.x.saturating_sub(totals_width + 1),
                area.y,
                totals_width,
                area.height.min(1),
            )
        } else {
            Rect::default()
        };
        ctx.push_slot(ChildKey::new("new-template"), self.template_area, |ctx| {
            self.template.layout(self.template_area, ctx)
        });
        ctx.push_slot(ChildKey::new("running-only"), self.running_area, |ctx| {
            self.running.layout(self.running_area, ctx)
        });
        ctx.push_slot(ChildKey::new("stop-all"), self.stop_area, |ctx| {
            self.stop.layout(self.stop_area, ctx)
        });
        ctx.push_slot(ChildKey::new("purge-all"), self.purge_area, |ctx| {
            self.purge.layout(self.purge_area, ctx)
        });
        ctx.push_slot(ChildKey::new("refresh"), self.refresh_area, |ctx| {
            self.refresh.layout(self.refresh_area, ctx)
        });
        LayoutResult::new(area)
    }

    fn render<'a>(&'a self, frame: &mut Frame, _area: Rect, ctx: &mut RenderCtx<'a>) {
        <Button<Msg> as TuiNode<Msg>>::render(&self.template, frame, self.template_area, ctx);
        <Button<Msg> as TuiNode<Msg>>::render(&self.refresh, frame, self.refresh_area, ctx);
        <Toggle<Msg> as TuiNode<Msg>>::render(&self.running, frame, self.running_area, ctx);
        <Button<Msg> as TuiNode<Msg>>::render(&self.stop, frame, self.stop_area, ctx);
        <Button<Msg> as TuiNode<Msg>>::render(&self.purge, frame, self.purge_area, ctx);
        frame.render_widget(self.totals_text(), self.totals_area);
    }

    fn event(&mut self, event: &TuiEvent, ctx: &mut EventCtx<Msg>) -> EventOutcome {
        self.sync_disabled();
        if self.action_hotkey(event, ctx) {
            return EventOutcome::Handled;
        }
        if (self.running.is_focused() || matches!(event, TuiEvent::Mouse(_)))
            && self.running.event(event, ctx) == EventOutcome::Handled
        {
            return EventOutcome::Handled;
        }
        for (_, button) in self.buttons_mut() {
            if button.event(event, ctx) == EventOutcome::Handled {
                return EventOutcome::Handled;
            }
        }
        EventOutcome::Ignored
    }

    fn dispatch_event(
        &mut self,
        route: &EventRoute,
        event: &TuiEvent,
        ctx: &mut EventCtx<Msg>,
    ) -> EventOutcome {
        self.sync_disabled();
        if self.action_hotkey(event, ctx) {
            return EventOutcome::Handled;
        }
        if let Some(path) = route.path.without_first_if(&ChildKey::new("running-only")) {
            return self
                .running
                .dispatch_event(&EventRoute::new(path), event, ctx);
        }
        for (key, button) in self.buttons_mut() {
            if let Some(path) = route.path.without_first_if(&ChildKey::new(key)) {
                return button.dispatch_event(&EventRoute::new(path), event, ctx);
            }
        }
        EventOutcome::Ignored
    }

    fn tick(&mut self, dt: Duration, settings: AnimationSettings) -> TickResult {
        let disabled_changed = self.sync_disabled();
        let mut result = TickResult::IDLE;
        for (_, button) in self.buttons_mut() {
            result = result.merge(<Button<Msg> as TuiNode<Msg>>::tick(button, dt, settings));
        }
        result = result.merge(<Toggle<Msg> as TuiNode<Msg>>::tick(
            &mut self.running,
            dt,
            settings,
        ));
        let totals_waiting = {
            let totals = &self.state.borrow().totals;
            totals.memory_waiting || totals.cpu_waiting
        };
        if totals_waiting {
            result = result.merge(Animated::tick(&mut self.spinner, dt, settings));
        }
        if disabled_changed || self.totals_text().width() != self.totals_width {
            result.changed = true;
            result.layout = true;
        }
        result
    }

    fn focus(&mut self, target: Option<&FocusId>, focused: bool, ctx: &mut FocusCtx<Msg>) {
        for (_, button) in self.buttons_mut() {
            button.focus(target, focused, ctx);
        }
        let running_focused = focused && target.is_some_and(|target| target.as_str() == "toggle");
        self.running.focus(target, running_focused, ctx);
    }

    fn dispatch_focus(&mut self, target: &FocusTarget, focused: bool, ctx: &mut FocusCtx<Msg>) {
        if let Some(target) = target.for_child(&ChildKey::new("running-only")) {
            self.running.dispatch_focus(&target, focused, ctx);
            return;
        }
        for (key, button) in self.buttons_mut() {
            if let Some(target) = target.for_child(&ChildKey::new(key)) {
                button.dispatch_focus(&target, focused, ctx);
            }
        }
    }

    fn init(&mut self, ctx: &mut LifecycleCtx<Msg>) {
        self.running.init(ctx);
        for (_, button) in self.buttons_mut() {
            button.init(ctx);
        }
    }
    fn mount(&mut self, ctx: &mut LifecycleCtx<Msg>) {
        self.running.mount(ctx);
        for (_, button) in self.buttons_mut() {
            button.mount(ctx);
        }
    }
    fn unmount(&mut self, ctx: &mut LifecycleCtx<Msg>) {
        self.running.unmount(ctx);
        for (_, button) in self.buttons_mut() {
            button.unmount(ctx);
        }
    }
    fn destroy(&mut self, ctx: &mut LifecycleCtx<Msg>) {
        self.running.destroy(ctx);
        for (_, button) in self.buttons_mut() {
            button.destroy(ctx);
        }
    }
}
