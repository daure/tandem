use ratatui::{
    Frame,
    layout::{Constraint, Rect},
    style::Style,
    text::{Line, Span, Text},
};
use std::{
    cell::RefCell,
    collections::{BTreeMap, BTreeSet},
    rc::Rc,
    time::Duration,
};
use tuicore::{
    AnimationSettings, Button, ChildKey, ChildSlot, Column, DataView, DataViewTypedEvent, EventCtx,
    EventOutcome, EventRoute, Flex, FlexItem, FocusCtx, FocusId, FocusTarget, HotkeyLabelMode,
    KeySpec, LayoutCtx, LayoutProposal, LayoutResult, LayoutSizeHint, LifecycleCtx, RenderCtx,
    TickResult, TreeAdapter, TuiEvent, TuiNode,
};

use super::{
    Msg,
    events::{clean, profile_icon},
};
use crate::store::providers::{Action, ActionError, Provider, Snapshot, Status};

mod rows;
mod streams;
use rows::{Row, from_snapshot, search_text, stream_text};

pub(super) const FOCUS: &str = "provider-list";
const DATA_SLOT: &str = "provider-data";
#[derive(Default)]
pub(super) struct State {
    pub snapshot: Snapshot,
    pub selected: Option<String>,
    pub requested: Option<String>,
}
pub(super) type SharedState = Rc<RefCell<State>>;
type ProviderView = DataView<Row, String>;

pub(super) struct PendingAction {
    pub name: String,
    pub action: Action,
    pub receiver: tokio::sync::oneshot::Receiver<Result<String, ActionError>>,
}

pub(super) struct Providers {
    view: ProviderView,
    controls: [ChildSlot<Button<Msg>, Msg>; 1],
    control_areas: [Rect; 1],
    view_area: Rect,
    shared: SharedState,
    snapshot: Snapshot,
    events: super::events::SharedState,
    totals: BTreeMap<String, u64>,
    handovers: BTreeMap<String, u64>,
}

impl Providers {
    pub(super) fn new(shared: SharedState, events: super::events::SharedState) -> Self {
        let counts = events.clone();
        let data = DataView::new(Vec::new(), |row: &Row| row.id.clone())
            .focus_id(FOCUS)
            .hotkey("shift+h")
            .columns(vec![
                Column::multiline(
                    "provider",
                    "Streams",
                    Constraint::Fill(1),
                    move |row: &Row, _| {
                        if let Some(stream) = &row.stream {
                            return stream_text(stream);
                        }
                        let total = row
                            .provider
                            .manifest
                            .as_ref()
                            .and_then(|manifest| {
                                counts.borrow().provider_totals.get(&manifest.name).copied()
                            })
                            .unwrap_or_default();
                        let handovers = row
                            .provider
                            .manifest
                            .as_ref()
                            .and_then(|manifest| {
                                counts
                                    .borrow()
                                    .provider_handovers
                                    .get(&manifest.name)
                                    .copied()
                            })
                            .unwrap_or_default();
                        row_text(&row.provider, handovers, total)
                    },
                )
                .search_key(search_text),
            ])
            .tree(TreeAdapter::parent_id(|row: &Row| row.parent.clone()))
            .headers(false)
            .action_bar(true)
            .filter_controls(false)
            .row_height(1)
            .empty_state(tuicore::SeasonalEmptyState::new(
                "No provider packages found under templates/providers.",
            ));
        Self {
            view: data,
            controls: [{
                let state = shared.clone();
                ChildSlot::new(
                    "provider-start-all",
                    Button::new("Start all")
                        .hotkey("shift+s")
                        .hotkey_focus_enabled(false)
                        .hotkey_label_mode(HotkeyLabelMode::Inline)
                        .on_press(move || {
                            let snapshot = &state.borrow().snapshot;
                            Msg::ProviderBulkAction(snapshot.start_stop_action())
                        }),
                )
            }],
            view_area: Rect::default(),
            control_areas: [Rect::default(); 1],
            shared,
            snapshot: Snapshot::default(),
            events,
            totals: BTreeMap::new(),
            handovers: BTreeMap::new(),
        }
    }

    fn sync(&mut self) -> bool {
        let requested = self.shared.borrow_mut().requested.take();
        let snapshot = self.shared.borrow().snapshot.clone();
        let totals = self.events.borrow().provider_totals.clone();
        let handovers = self.events.borrow().provider_handovers.clone();
        if snapshot == self.snapshot && totals == self.totals && handovers == self.handovers {
            if let Some(name) = requested {
                self.view.highlight_id(&name);
                self.shared.borrow_mut().selected = Some(name);
                return true;
            }
            return false;
        }
        self.view.set_rows(from_snapshot(&snapshot));
        for provider in &snapshot.providers {
            if !self
                .snapshot
                .providers
                .iter()
                .any(|previous| previous.name == provider.name && !previous.streams.is_empty())
            {
                self.view.expand(&provider.name);
            }
        }
        if let Some(name) = requested {
            self.view.highlight_id(&name);
        }
        self.shared.borrow_mut().selected = self.view.highlighted_id();
        self.snapshot = snapshot;
        self.totals = totals;
        self.handovers = handovers;
        true
    }

    fn sync_controls(&mut self, compact: bool) {
        let snapshot = &self.shared.borrow().snapshot;
        for (control, action) in self.controls.iter_mut().zip([snapshot.start_stop_action()]) {
            let label = match action {
                Action::Start => "Start",
                Action::Stop => "Stop",
                Action::Logs => unreachable!(),
            };
            control.child_mut().set_label(if compact {
                label.into()
            } else {
                format!("{label} all")
            });
            control
                .child_mut()
                .set_disabled(snapshot.action_targets(action).is_empty());
        }
    }

    fn drain(&mut self, ctx: &mut EventCtx<Msg>) {
        for event in self.view.take_events() {
            match event {
                DataViewTypedEvent::HighlightChanged { row_id } => {
                    self.shared.borrow_mut().selected = row_id
                }
                DataViewTypedEvent::Activated { row_id } => {
                    if let Some(row) = self.view.rows().iter().find(|row| row.id == row_id) {
                        ctx.emit(if let Some(stream) = &row.stream {
                            Msg::ProviderStreamDetails(
                                row.provider.name.clone(),
                                Box::new(stream.clone()),
                            )
                        } else {
                            Msg::ProviderDetails(Box::new(row.provider.clone()))
                        });
                    }
                }
                _ => {}
            }
        }
    }

    fn action(&mut self, event: &TuiEvent, ctx: &mut EventCtx<Msg>) -> bool {
        if super::App::returns_to_data_view(event)
            && self
                .controls
                .iter()
                .any(|control| control.child().is_focused())
        {
            ctx.focus(tuicore::FocusRequest::Target(FocusId::new(FOCUS)));
            ctx.request_redraw();
            ctx.stop_propagation();
            return true;
        }
        if super::App::overview_requested(event) {
            self.sync();
            self.view.clear_search();
            self.view.clear_filters();
            self.view.expand_all();
            if self.view.is_searching() {
                self.view.on_key(tuicore::Key::Enter, Rect::default());
            }
            if let Some(id) = self.view.rows().first().map(|row| row.id.clone()) {
                self.view.highlight_id(&id);
            }
            self.view.reveal_highlighted();
            self.drain(ctx);
            self.shared.borrow_mut().selected = self.view.highlighted_id();
            ctx.focus(tuicore::FocusRequest::Target(FocusId::new(FOCUS)));
            ctx.request_layout();
            ctx.request_redraw();
            ctx.stop_propagation();
            return true;
        }
        if self.view.is_searching() {
            return false;
        }
        let TuiEvent::Key(key) = event else {
            return false;
        };
        if KeySpec::shifted('s').matches(*key) {
            let action = self.snapshot.start_stop_action();
            if !self.snapshot.action_targets(action).is_empty() {
                ctx.emit(Msg::ProviderBulkAction(action));
            }
            ctx.stop_propagation();
            return true;
        }
        let Some(name) = self.view.highlighted_id() else {
            return false;
        };
        let Some(row) = self.view.rows().iter().find(|row| row.id == name) else {
            return false;
        };
        let provider = &row.provider;
        if KeySpec::key(tuicore::Key::Enter).matches(*key) && self.view.is_focused() {
            let target = if let Some(stream) = &row.stream {
                super::row_actions::Target::Stream(Box::new((provider.clone(), stream.clone())))
            } else {
                super::row_actions::Target::Provider(Box::new(provider.clone()))
            };
            if let Some(message) = target.enter_message() {
                ctx.emit(message);
            }
            ctx.stop_propagation();
            return true;
        }
        if let Some(stream) = &row.stream {
            if KeySpec::plain('.').matches(*key) {
                ctx.emit(Msg::OpenRowMenu(super::row_actions::Target::Stream(
                    Box::new((provider.clone(), stream.clone())),
                )));
            } else if KeySpec::plain('e').matches(*key) {
                if let Some(manifest) = &provider.manifest {
                    ctx.emit(Msg::ProviderStreamEvents(
                        manifest.name.clone(),
                        stream.name.clone(),
                    ));
                }
            } else if KeySpec::plain('s').matches(*key) {
                let action = stream.start_stop_action(provider);
                ctx.emit(Msg::ProviderStreamAction(
                    provider.name.clone(),
                    stream.name.clone(),
                    action,
                ));
            } else {
                return false;
            }
            ctx.stop_propagation();
            return true;
        }
        if KeySpec::plain('.').matches(*key) {
            ctx.emit(Msg::OpenRowMenu(super::row_actions::Target::Provider(
                Box::new(provider.clone()),
            )));
        } else {
            let action = [
                (
                    's',
                    if matches!(provider.status, Status::Running | Status::Paused) {
                        Action::Stop
                    } else {
                        Action::Start
                    },
                ),
                ('o', Action::Logs),
            ]
            .into_iter()
            .find(|(character, _)| KeySpec::plain(*character).matches(*key))
            .map(|(_, action)| action);
            let Some(action) = action else {
                return false;
            };
            ctx.emit(Msg::ProviderAction(provider.name.clone(), action));
        }
        ctx.stop_propagation();
        true
    }
}

fn provider_label(provider: &Provider) -> &str {
    provider
        .manifest
        .as_ref()
        .map(|manifest| manifest.name.as_str())
        .unwrap_or("Invalid provider")
}

fn display_operation(provider: &Provider) -> Option<Action> {
    if let Some(operation) = provider.operation.filter(|action| *action != Action::Logs) {
        return Some(operation);
    }
    if provider
        .streams
        .iter()
        .any(|stream| stream.operation == Some(Action::Stop))
        && provider.streams.iter().all(|stream| {
            !stream.controllable || !stream.enabled || stream.operation == Some(Action::Stop)
        })
    {
        return Some(Action::Stop);
    }
    let collecting = provider
        .streams
        .iter()
        .any(|stream| stream.status == Status::Running);
    if provider.status != Status::Unknown
        && !collecting
        && provider.streams.iter().any(|stream| {
            stream.operation == Some(Action::Start) || stream.status == Status::Starting
        })
    {
        return Some(Action::Start);
    }
    None
}

fn row_text(provider: &Provider, handovers: u64, total: u64) -> Text<'static> {
    let theme = tuicore::theme();
    let icon = profile_icon(
        provider
            .manifest
            .as_ref()
            .map(|manifest| manifest.profile.as_str())
            .unwrap_or("generic"),
    );
    let operation = display_operation(provider);
    let color = match (operation, provider.status) {
        (Some(Action::Start | Action::Stop), _) => theme.info_fg(),
        (_, Status::Running) => theme.success_fg(),
        (_, Status::Paused) => theme.warning_fg(),
        (_, Status::Unknown) => theme.error_fg(),
        _ => theme.muted_fg(),
    };
    let status = match operation {
        Some(Action::Start) => "Starting...".to_owned(),
        Some(Action::Stop) => "Stopping...".to_owned(),
        _ => match provider.status {
            Status::Running => "Healthy".to_owned(),
            Status::NotStarted => "Not started".to_owned(),
            status => format!("{status:?}"),
        },
    };
    let streams = if provider.streams.is_empty() {
        String::new()
    } else {
        format!(
            " · {}/{} collecting",
            provider
                .streams
                .iter()
                .filter(|stream| stream.status == Status::Running)
                .count(),
            provider.streams.len()
        )
    };
    Text::from(Line::from(vec![
        Span::styled(
            icon,
            if matches!(operation, Some(Action::Start | Action::Stop)) {
                Style::default().fg(theme.info_fg())
            } else if provider.status == Status::Running {
                Style::default().fg(theme.success_fg())
            } else {
                Style::default()
            },
        ),
        Span::raw(format!(
            " {} ·  {handovers}/{total} · ",
            clean(provider_label(provider))
        )),
        Span::styled(status, Style::default().fg(color)),
        Span::raw(streams),
    ]))
}

impl TuiNode<Msg> for Providers {
    fn measure(&self, proposal: LayoutProposal) -> LayoutSizeHint {
        let mut hint = <ProviderView as TuiNode<Msg>>::measure(&self.view, proposal);
        hint.preferred.height = hint.preferred.height.saturating_add(1);
        hint.normalized(proposal)
    }
    fn layout(&mut self, area: Rect, ctx: &mut LayoutCtx) -> LayoutResult {
        let alternate: BTreeSet<_> = self
            .view
            .rows()
            .iter()
            .filter(|row| {
                tuicore::search_match(
                    &self.view.transform_state().search,
                    &search_text(row),
                    tuicore::SearchMode::Fuzzy,
                )
                .is_some()
            })
            .enumerate()
            .filter(|(index, _)| index % 2 == 0)
            .map(|(_, row)| row.id.clone())
            .collect();
        self.view.set_row_style_by(move |row| {
            alternate
                .contains(&row.id)
                .then(|| Style::default().bg(tuicore::theme().surface_bg()))
        });
        self.sync_controls(area.width < 60);
        let proposal = LayoutProposal::at_most(area.width, area.height.min(1));
        let widths = self
            .controls
            .each_ref()
            .map(|control| control.measure(proposal).preferred.width);
        let total = widths.iter().copied().sum::<u16>();
        let mut x = area.right().saturating_sub(total).max(area.x);
        for ((control, control_area), width) in self
            .controls
            .iter_mut()
            .zip(&mut self.control_areas)
            .zip(widths)
        {
            let width = width.min(area.right().saturating_sub(x));
            *control_area = Rect::new(x, area.y, width, area.height.min(1));
            control.layout(*control_area, ctx);
            x = x.saturating_add(width).saturating_add(1);
        }
        self.view_area = Rect::new(
            area.x,
            area.y.saturating_add(1),
            area.width,
            area.height.saturating_sub(1),
        );
        ctx.push_slot(ChildKey::new(DATA_SLOT), self.view_area, |ctx| {
            <ProviderView as TuiNode<Msg>>::layout(&mut self.view, self.view_area, ctx)
        });
        LayoutResult::new(area)
    }
    fn render<'a>(&'a self, frame: &mut Frame, _area: Rect, ctx: &mut RenderCtx<'a>) {
        for (control, area) in self.controls.iter().zip(self.control_areas) {
            control.render(frame, area, ctx);
        }
        <ProviderView as TuiNode<Msg>>::render(&self.view, frame, self.view_area, ctx);
    }
    fn event(&mut self, event: &TuiEvent, ctx: &mut EventCtx<Msg>) -> EventOutcome {
        if !self.view.is_searching() {
            for control in &mut self.controls {
                if control.child_mut().event(event, ctx) == EventOutcome::Handled {
                    ctx.stop_propagation();
                    return EventOutcome::Handled;
                }
            }
        }
        if self.action(event, ctx) {
            return EventOutcome::Handled;
        }
        let outcome = self.view.event(event, ctx);
        self.drain(ctx);
        outcome
    }
    fn dispatch_event(
        &mut self,
        route: &EventRoute,
        event: &TuiEvent,
        ctx: &mut EventCtx<Msg>,
    ) -> EventOutcome {
        if !self.view.is_searching() {
            for control in &mut self.controls {
                if control.dispatch_event(route, event, ctx) == EventOutcome::Handled {
                    ctx.stop_propagation();
                    return EventOutcome::Handled;
                }
            }
        }
        if self.action(event, ctx) {
            return EventOutcome::Handled;
        }
        let Some(path) = route.path.without_first_if(&ChildKey::new(DATA_SLOT)) else {
            return EventOutcome::Ignored;
        };
        let outcome = self.view.dispatch_event(&EventRoute::new(path), event, ctx);
        self.drain(ctx);
        outcome
    }
    fn tick(&mut self, dt: Duration, settings: AnimationSettings) -> TickResult {
        let changed = self.sync();
        let mut result = <ProviderView as TuiNode<Msg>>::tick(&mut self.view, dt, settings);
        for control in &mut self.controls {
            result = result.merge(control.tick(dt, settings));
        }
        result.merge(if changed {
            TickResult {
                layout: true,
                ..TickResult::CHANGED
            }
        } else {
            TickResult::IDLE
        })
    }
    fn take_pending_focus_request(&mut self) -> Option<tuicore::FocusRequest> {
        <ProviderView as TuiNode<Msg>>::take_pending_focus_request(&mut self.view)
    }
    fn take_pending_clipboard_request(&mut self) -> Option<String> {
        <ProviderView as TuiNode<Msg>>::take_pending_clipboard_request(&mut self.view)
    }
    fn focus(&mut self, target: Option<&FocusId>, focused: bool, ctx: &mut FocusCtx<Msg>) {
        self.view.focus(target, focused, ctx);
        for control in &mut self.controls {
            control.child_mut().focus(target, focused, ctx);
        }
    }
    fn dispatch_focus(&mut self, target: &FocusTarget, focused: bool, ctx: &mut FocusCtx<Msg>) {
        if let Some(target) = target.for_child(&ChildKey::new(DATA_SLOT)) {
            self.view.dispatch_focus(&target, focused, ctx);
        }
        for control in &mut self.controls {
            control.dispatch_focus(target, focused, ctx);
        }
    }
    fn focus_reveal_area(&self, target: &FocusTarget) -> Option<Rect> {
        if let Some(area) = self
            .controls
            .iter()
            .find_map(|control| control.focus_reveal_area(target))
        {
            return Some(area);
        }
        let target = target.for_child(&ChildKey::new(DATA_SLOT))?;
        <ProviderView as TuiNode<Msg>>::focus_reveal_area(&self.view, &target)
    }
    fn focus_reveal_centered(&self, target: &FocusTarget) -> bool {
        target
            .for_child(&ChildKey::new(DATA_SLOT))
            .is_some_and(|target| {
                <ProviderView as TuiNode<Msg>>::focus_reveal_centered(&self.view, &target)
            })
    }
    fn init(&mut self, ctx: &mut LifecycleCtx<Msg>) {
        self.view.init(ctx);
        for control in &mut self.controls {
            control.init(ctx);
        }
    }
    fn mount(&mut self, ctx: &mut LifecycleCtx<Msg>) {
        self.view.mount(ctx);
        for control in &mut self.controls {
            control.mount(ctx);
        }
    }
    fn unmount(&mut self, ctx: &mut LifecycleCtx<Msg>) {
        self.view.unmount(ctx);
        for control in &mut self.controls {
            control.unmount(ctx);
        }
    }
    fn destroy(&mut self, ctx: &mut LifecycleCtx<Msg>) {
        self.view.destroy(ctx);
        for control in &mut self.controls {
            control.destroy(ctx);
        }
    }
}

impl super::App {
    pub(super) fn request_provider_bulk_action(&mut self, action: Action, ctx: &mut EventCtx<Msg>) {
        let mut names = self.pages_mut().provider_bulk_targets(action);
        names.retain(|name| {
            !self.provider_actions.iter().any(|pending| {
                &pending.name == name || pending.name.starts_with(&format!("{name}/"))
            })
        });
        if names.is_empty() {
            self.notify(tuicore::Notification::warning(
                "Provider action unavailable",
                "No providers are available for this action",
            ));
            return;
        }
        if action == Action::Logs {
            return;
        }
        let dialog = super::dialogs::confirmation(
            &format!("{action:?} all providers"),
            format!("{action:?} {} providers?", names.len()),
        );
        self.provider_confirmation = None;
        self.provider_stream_confirmation = None;
        self.provider_bulk_confirmation = Some((names, action));
        self.intent = None;
        self.open_compact(dialog, ctx);
    }

    pub(super) fn request_provider_action(
        &mut self,
        name: String,
        action: Action,
        ctx: &mut EventCtx<Msg>,
    ) {
        let unavailable =
            if self.provider_actions.iter().any(|pending| {
                pending.name == name || pending.name.starts_with(&format!("{name}/"))
            }) {
                Some("An action for this provider is already in progress; wait for it to finish")
            } else {
                self.pages_mut().provider_action_unavailable(&name, action)
            };
        if let Some(reason) = unavailable {
            self.notify(tuicore::Notification::warning(
                "Provider action unavailable",
                reason,
            ));
            return;
        }
        if action == Action::Logs {
            self.start_provider_action(name, action, false);
            return;
        }
        self.provider_confirmation = Some((name.clone(), action));
        self.provider_stream_confirmation = None;
        self.provider_bulk_confirmation = None;
        let dialog = super::dialogs::confirmation(
            &format!("{action:?} provider"),
            format!("{action:?} {name} (all streams)?"),
        );
        self.intent = None;
        self.open_compact(dialog, ctx);
    }

    pub(super) fn open_provider_details(&mut self, provider: &Provider, ctx: &mut EventCtx<Msg>) {
        let text = serde_json::to_string_pretty(provider).unwrap_or_default();
        self.open_provider_text("Provider details", text, ctx);
    }

    fn open_provider_text(&mut self, title: &str, text: String, ctx: &mut EventCtx<Msg>) {
        let text = text.lines().map(clean).collect::<Vec<_>>().join("\n");
        let dialog = super::dialogs::dialog(title).host(
            Flex::column().child(
                "provider-text",
                tuicore::SyntaxHighlighter::new(
                    text,
                    tuicore::Language::guess(Some("provider.txt"), ""),
                )
                .wrap(true),
                FlexItem::fill(1),
            ),
        );
        self.intent = None;
        self.open(Box::new(dialog), ctx);
        self.details_open = true;
        self.resize_details_dialog();
    }

    pub(super) fn start_provider_action(&mut self, name: String, action: Action, confirmed: bool) {
        let receiver = self
            .service
            .provider_action(name.clone(), action, confirmed);
        self.provider_actions.push(PendingAction {
            name,
            action,
            receiver,
        });
    }

    pub(super) fn poll_provider_action(&mut self) -> bool {
        let mut completed = Vec::new();
        self.provider_actions.retain_mut(|pending| {
            let result = match pending.receiver.try_recv() {
                Ok(result) => result,
                Err(tokio::sync::oneshot::error::TryRecvError::Empty) => return true,
                Err(tokio::sync::oneshot::error::TryRecvError::Closed) => {
                    Err(ActionError::Failed("provider worker stopped".into()))
                }
            };
            completed.push((pending.action, result));
            false
        });
        let changed = !completed.is_empty();
        for (action, result) in completed {
            self.finish_provider_action(action, result);
        }
        changed
    }

    fn finish_provider_action(&mut self, action: Action, result: Result<String, ActionError>) {
        match result {
            Ok(text) if action == Action::Logs => {
                let mut ctx = EventCtx::new(tuicore::animation_settings());
                self.open_provider_text("Provider logs", text, &mut ctx);
            }
            Ok(text) => self.notify(tuicore::Notification::info("Provider", text)),
            Err(ActionError::Unavailable(reason)) => self.notify(tuicore::Notification::warning(
                "Provider action unavailable",
                reason,
            )),
            Err(ActionError::Failed(error)) => self.notify(tuicore::Notification::error(
                "Provider action failed",
                error,
            )),
        }
    }
}
