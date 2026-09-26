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
    pub has_running_instances: bool,
    pub stop_targets: Vec<String>,
    pub purge_targets: Vec<String>,
    pub running_only: bool,
    pub opencode_enabled: bool,
    pub show_saved: bool,
    pub attached_sessions_only: bool,
}

impl State {
    pub(super) fn from_snapshot(snapshot: &EnvironmentSnapshot) -> Self {
        Self {
            totals: UsageSummary::instances(snapshot.instances.iter()),
            available_memory_bytes: snapshot.available_memory_bytes,
            has_running_instances: snapshot
                .instances
                .iter()
                .any(|instance| instance.is_running()),
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
            opencode_enabled: false,
            show_saved: false,
            attached_sessions_only: false,
        }
    }
}

pub(super) type SharedState = Rc<RefCell<State>>;

pub(super) struct Toolbar {
    template: Button<Msg>,
    refresh: Button<Msg>,
    running: Toggle<Msg>,
    history: Toggle<Msg>,
    attached: Toggle<Msg>,
    history_visible: bool,
    stop: Button<Msg>,
    purge: Button<Msg>,
    refresh_key: KeySpec,
    template_key: KeySpec,
    running_key: KeySpec,
    history_key: KeySpec,
    attached_key: KeySpec,
    bulk_keys: [KeySpec; 2],
    template_area: Rect,
    refresh_area: Rect,
    running_area: Rect,
    history_area: Rect,
    attached_area: Rect,
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
        let history_key = KeySpec::shifted('o');
        let attached_key = KeySpec::shifted('a');
        let history_visible = state.borrow().opencode_enabled;
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
            history: Toggle::new("󰋚")
                .hotkey(hotkey(history_key))
                .preserve_focus_on_hotkey(true)
                .on_change(Msg::SetOpencodeHistory),
            attached: Toggle::new("󰚩")
                .hotkey(hotkey(attached_key))
                .preserve_focus_on_hotkey(true)
                .on_change(Msg::SetAttachedSessionsOnly),
            history_visible,
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
            template_key,
            running_key,
            history_key,
            attached_key,
            bulk_keys: [stop_key, purge_key],
            template_area: Rect::default(),
            refresh_area: Rect::default(),
            running_area: Rect::default(),
            history_area: Rect::default(),
            attached_area: Rect::default(),
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
        if state.has_running_instances && totals.memory_bytes.is_some() {
            spans.push(Span::raw(" · "));
            spans.push(Span::styled(" ", memory_style));
            spans.extend(memory.spans);
        }
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
        let show_saved = state.show_saved;
        let attached_sessions_only = state.attached_sessions_only;
        let changed = self.stop.is_disabled() != stop_disabled
            || self.purge.is_disabled() != purge_disabled
            || self.running.is_checked() != running_only
            || self.history.is_checked() != show_saved
            || self.attached.is_checked() != attached_sessions_only
            || self.history_visible != state.opencode_enabled;
        self.history_visible = state.opencode_enabled;
        if self.attached.is_checked() != attached_sessions_only {
            self.attached.set_value(attached_sessions_only);
        }
        if self.history.is_checked() != show_saved {
            self.history.set_value(show_saved);
        }
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
        if self.history_visible && self.attached_key.matches(*key) {
            let outcome = self.attached.toggle();
            if outcome.changed {
                ctx.emit(Msg::SetAttachedSessionsOnly(outcome.value));
                ctx.request_redraw();
            }
            ctx.stop_propagation();
            return true;
        }
        if self.history_visible && self.history_key.matches(*key) {
            let outcome = self.history.toggle();
            if outcome.changed {
                ctx.emit(Msg::SetOpencodeHistory(outcome.value));
                ctx.request_redraw();
            }
            ctx.stop_propagation();
            return true;
        }
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
            .saturating_add(if self.state.borrow().opencode_enabled {
                self.history
                    .measure(proposal)
                    .preferred
                    .width
                    .saturating_add(self.attached.measure(proposal).preferred.width)
                    .saturating_add(2)
            } else {
                0
            })
            .saturating_add(self.stop.measure(proposal).preferred.width)
            .saturating_add(self.purge.measure(proposal).preferred.width)
            .saturating_add(self.totals_text().width().min(u16::MAX as usize) as u16)
            .saturating_add(4);
        LayoutSizeHint::content(width, 1).normalized(proposal)
    }

    fn layout(&mut self, area: Rect, ctx: &mut LayoutCtx) -> LayoutResult {
        self.sync_disabled();
        let compact = area.width < MOBILE_TABS_WIDTH;
        let spacing = u16::from(area.width >= 50);
        let bulk_hotkey_mode = if area.width < 50 {
            HotkeyLabelMode::PreferMnemonic
        } else {
            HotkeyLabelMode::Inline
        };
        self.stop.set_hotkey_label_mode(bulk_hotkey_mode);
        self.purge.set_hotkey_label_mode(bulk_hotkey_mode);
        self.template.set_label(if compact {
            format!("󰠲 {}", self.template_key.label())
        } else {
            "Template".into()
        });
        self.stop.set_label(if area.width < 50 {
            format!(" {}", self.bulk_keys[0].label())
        } else if compact {
            "".into()
        } else {
            " Stop all".into()
        });
        self.purge.set_label(if area.width < 50 {
            format!(" {}", self.bulk_keys[1].label())
        } else if compact {
            "".into()
        } else {
            " Purge all".into()
        });
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
        let purge_width = self.purge.measure(proposal).preferred.width.min(
            area.width
                .saturating_sub(refresh_width.saturating_add(spacing)),
        );
        let stop_width = self.stop.measure(proposal).preferred.width.min(
            area.width.saturating_sub(
                refresh_width
                    .saturating_add(purge_width)
                    .saturating_add(2 * spacing),
            ),
        );
        let running_width = self.running.measure(proposal).preferred.width.min(
            area.width.saturating_sub(
                refresh_width
                    .saturating_add(purge_width)
                    .saturating_add(stop_width)
                    .saturating_add(2 * spacing),
            ),
        );
        let history_width = if self.history_visible {
            let measured = self.history.measure(proposal).preferred.width;
            let visible = if compact {
                measured.saturating_sub(4)
            } else {
                measured
            };
            visible.min(area.width.saturating_sub(
                refresh_width + purge_width + stop_width + running_width + 2 * spacing,
            ))
        } else {
            0
        };
        let history_right_padding = u16::from(self.history_visible);
        let attached_width = if self.history_visible {
            let measured = self.attached.measure(proposal).preferred.width;
            let visible = if compact {
                measured.saturating_sub(4)
            } else {
                measured
            };
            visible.min(area.width.saturating_sub(
                refresh_width
                    + purge_width
                    + stop_width
                    + running_width
                    + history_width
                    + history_right_padding
                    + 2 * spacing,
            ))
        } else {
            0
        };
        let attached_right_padding = u16::from(attached_width > 0 && !compact);
        let filters_width = running_width
            .saturating_add(history_width)
            .saturating_add(history_right_padding)
            .saturating_add(attached_width)
            .saturating_add(attached_right_padding);
        let actions_width = purge_width
            .saturating_add(stop_width)
            .saturating_add(refresh_width)
            .saturating_add(2 * spacing);
        let template_width = self.template.measure(proposal).preferred.width.min(
            area.width.saturating_sub(
                filters_width
                    .saturating_add(actions_width)
                    .saturating_add(spacing),
            ),
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
                .saturating_sub(purge_width.saturating_add(spacing))
                .max(area.x),
            area.y,
            purge_width,
            area.height.min(1),
        );
        self.stop_area = Rect::new(
            self.purge_area
                .x
                .saturating_sub(stop_width.saturating_add(spacing))
                .max(area.x),
            area.y,
            stop_width,
            area.height.min(1),
        );
        self.attached_area = Rect::new(
            self.template_area.right().saturating_add(spacing),
            area.y,
            attached_width,
            area.height.min(1),
        );
        self.history_area = Rect::new(
            self.attached_area
                .right()
                .saturating_add(attached_right_padding),
            area.y,
            history_width,
            area.height.min(1),
        );
        self.running_area = Rect::new(
            self.history_area
                .right()
                .saturating_add(history_right_padding),
            area.y,
            running_width,
            area.height.min(1),
        );
        self.totals_width = self.totals_text().width();
        let totals_width = self.totals_width.min(u16::MAX as usize) as u16;
        let available = self
            .stop_area
            .x
            .saturating_sub(self.running_area.right().saturating_add(2));
        self.totals_area = if totals_width <= available {
            Rect::new(
                self.stop_area.x.saturating_sub(totals_width + 1),
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
        if self.history_visible {
            ctx.push_slot(
                ChildKey::new("attached-sessions-only"),
                self.attached_area,
                |ctx| self.attached.layout(self.attached_area, ctx),
            );
            ctx.push_slot(
                ChildKey::new("opencode-history"),
                self.history_area,
                |ctx| self.history.layout(self.history_area, ctx),
            );
        }
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
        if self.history_visible {
            <Toggle<Msg> as TuiNode<Msg>>::render(&self.attached, frame, self.attached_area, ctx);
            <Toggle<Msg> as TuiNode<Msg>>::render(&self.history, frame, self.history_area, ctx);
        }
        <Button<Msg> as TuiNode<Msg>>::render(&self.stop, frame, self.stop_area, ctx);
        <Button<Msg> as TuiNode<Msg>>::render(&self.purge, frame, self.purge_area, ctx);
        frame.render_widget(self.totals_text(), self.totals_area);
    }

    fn event(&mut self, event: &TuiEvent, ctx: &mut EventCtx<Msg>) -> EventOutcome {
        self.sync_disabled();
        if self.action_hotkey(event, ctx) {
            return EventOutcome::Handled;
        }
        if self.history_visible
            && (self.attached.is_focused() || matches!(event, TuiEvent::Mouse(_)))
            && self.attached.event(event, ctx) == EventOutcome::Handled
        {
            return EventOutcome::Handled;
        }
        if self.history_visible
            && (self.history.is_focused() || matches!(event, TuiEvent::Mouse(_)))
            && self.history.event(event, ctx) == EventOutcome::Handled
        {
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
        if self.history_visible
            && let Some(path) = route
                .path
                .without_first_if(&ChildKey::new("attached-sessions-only"))
        {
            return self
                .attached
                .dispatch_event(&EventRoute::new(path), event, ctx);
        }
        if self.history_visible
            && let Some(path) = route
                .path
                .without_first_if(&ChildKey::new("opencode-history"))
        {
            return self
                .history
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
        if self.history_visible {
            result = result.merge(<Toggle<Msg> as TuiNode<Msg>>::tick(
                &mut self.attached,
                dt,
                settings,
            ));
            result = result.merge(<Toggle<Msg> as TuiNode<Msg>>::tick(
                &mut self.history,
                dt,
                settings,
            ));
        }
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
        if !focused {
            self.attached.focus(target, false, ctx);
            self.history.focus(target, false, ctx);
        }
    }

    fn dispatch_focus(&mut self, target: &FocusTarget, focused: bool, ctx: &mut FocusCtx<Msg>) {
        if let Some(target) = target.for_child(&ChildKey::new("attached-sessions-only")) {
            self.attached
                .dispatch_focus(&target, focused && self.history_visible, ctx);
            return;
        }
        if let Some(target) = target.for_child(&ChildKey::new("opencode-history")) {
            self.history
                .dispatch_focus(&target, focused && self.history_visible, ctx);
            return;
        }
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
        self.attached.init(ctx);
        self.history.init(ctx);
        self.running.init(ctx);
        for (_, button) in self.buttons_mut() {
            button.init(ctx);
        }
    }
    fn mount(&mut self, ctx: &mut LifecycleCtx<Msg>) {
        self.attached.mount(ctx);
        self.history.mount(ctx);
        self.running.mount(ctx);
        for (_, button) in self.buttons_mut() {
            button.mount(ctx);
        }
    }
    fn unmount(&mut self, ctx: &mut LifecycleCtx<Msg>) {
        self.attached.unmount(ctx);
        self.history.unmount(ctx);
        self.running.unmount(ctx);
        for (_, button) in self.buttons_mut() {
            button.unmount(ctx);
        }
    }
    fn destroy(&mut self, ctx: &mut LifecycleCtx<Msg>) {
        self.attached.destroy(ctx);
        self.history.destroy(ctx);
        self.running.destroy(ctx);
        for (_, button) in self.buttons_mut() {
            button.destroy(ctx);
        }
    }
}
