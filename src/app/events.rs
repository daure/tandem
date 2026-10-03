use std::{cell::RefCell, collections::BTreeSet, rc::Rc, time::Duration};

use ratatui::{
    Frame,
    layout::{Constraint, Rect},
    style::Style,
    text::Line,
};
use tuicore::{
    AnimationSettings, Column, DataView, DataViewTypedEvent, Dropdown, DropdownLabelPosition,
    DropdownVariant, EventCtx, EventOutcome, EventRoute, FocusCtx, FocusId, FocusTarget, KeySpec,
    LayoutCtx, LayoutProposal, LayoutResult, LayoutSizeHint, LifecycleCtx, Paragraph, RenderCtx,
    Split, TickResult, Toggle, TuiEvent, TuiNode, line_width,
};

use super::Msg;
use crate::store::events::{Record, Snapshot};

mod rows;
pub(super) use rows::{profile_icon, row_text};

pub(super) const FOCUS: &str = "event-feed";
pub(super) type SharedState = Rc<RefCell<Snapshot>>;
pub(super) type FilterState = Rc<RefCell<Option<Vec<String>>>>;
type EventControls = Split<Split<Toggle<Msg>, Toggle<Msg>>, Paragraph>;
type EventView = Split<Split<Dropdown<String, String>, EventControls>, DataView<Record, i64>>;

pub(super) struct Events {
    view: EventView,
    shared: SharedState,
    snapshot: Snapshot,
    filter: FilterState,
    sources: Vec<String>,
    providers: super::providers::SharedState,
    provider_names: Vec<String>,
    handovers_only: bool,
    following: bool,
    newest_visible: Option<i64>,
    focus_feed: bool,
    status: String,
}

impl Events {
    pub(super) fn new(
        shared: SharedState,
        filter: FilterState,
        providers: super::providers::SharedState,
    ) -> Self {
        let selection = filter.clone();
        let provider_filter =
            Dropdown::multi(Vec::new(), |name: &String| name.clone(), |name| clean(name))
                .variant(DropdownVariant::Filled)
                .label("Providers")
                .label_position(DropdownLabelPosition::Inline)
                .placeholder("All providers")
                .show_multi_labels(true)
                .max_popup_width(u16::MAX)
                .hotkey("shift+p")
                .on_select(move |names| *selection.borrow_mut() = Some(names));
        let data = DataView::new(Vec::new(), |row: &Record| row.sequence)
            .focus_id(FOCUS)
            .hotkey("shift+h")
            .columns(vec![
                Column::multiline("event", "Events", Constraint::Fill(1), |row: &Record, _| {
                    row_text(row)
                })
                .search_key(search_text)
                .constrained(),
            ])
            .headers(false)
            .action_bar(true)
            .filter_controls(false)
            .row_height(2)
            .empty_state(tuicore::SeasonalEmptyState::new(
                "No events received. Start a provider to populate this feed.",
            ));
        Self {
            view: Split::vertical(
                Split::horizontal(
                    provider_filter,
                    Split::horizontal(
                        Split::horizontal(
                            Toggle::new("")
                                .hotkey("shift+t")
                                .preserve_focus_on_hotkey(true),
                            Toggle::new("")
                                .checked(true)
                                .hotkey("shift+g")
                                .preserve_focus_on_hotkey(true),
                        )
                        .gap(1),
                        Paragraph::new("Waiting for event providers").wrap(false),
                    )
                    .constraints(Constraint::Length(11), Constraint::Fill(1))
                    .gap(1),
                )
                .constraints(Constraint::Length(1), Constraint::Fill(1))
                .gap(1),
                data,
            )
            .constraints(Constraint::Length(1), Constraint::Fill(1)),
            shared,
            snapshot: Snapshot::default(),
            filter,
            sources: Vec::new(),
            providers,
            provider_names: Vec::new(),
            handovers_only: false,
            following: true,
            newest_visible: None,
            focus_feed: false,
            status: "Waiting for event providers".into(),
        }
    }

    fn sync(&mut self) -> bool {
        let filter = self.filter.borrow_mut().take();
        let filtered = filter.is_some();
        if let Some(names) = filter {
            self.sources = names;
        }
        let snapshot = self.shared.borrow().clone();
        let mut provider_names: Vec<_> = self
            .providers
            .borrow()
            .snapshot
            .providers
            .iter()
            .filter_map(|provider| {
                provider
                    .manifest
                    .as_ref()
                    .map(|manifest| manifest.name.clone())
            })
            .chain(snapshot.records.iter().map(|row| row.provider.clone()))
            .chain(self.sources.iter().cloned())
            .collect();
        provider_names.sort();
        provider_names.dedup();
        let providers_changed = provider_names != self.provider_names;
        let handovers_only = self.view.first().second().first().first().is_checked();
        let handovers_changed = handovers_only != self.handovers_only;
        if snapshot == self.snapshot && !filtered && !providers_changed && !handovers_changed {
            return false;
        }
        if providers_changed {
            self.view
                .first_mut()
                .first_mut()
                .set_rows(provider_names.clone());
            self.provider_names = provider_names;
        }
        if filtered {
            self.view
                .first_mut()
                .first_mut()
                .set_selected(self.sources.clone());
        }
        let mut records: Vec<_> = snapshot
            .records
            .iter()
            .filter(|row| self.sources.is_empty() || self.sources.contains(&row.provider))
            // Acceptance alone does not establish an instance handover; dispatch has no assignments.
            .filter(|_| !handovers_only)
            .cloned()
            .collect();
        records.sort_by_key(|row| row.sequence);
        let status = snapshot
            .error
            .as_deref()
            .map(clean)
            .unwrap_or_else(|| format!("{} of {} events", records.len(), snapshot.total));
        self.view.second_mut().set_rows(records);
        let highlighted = self.view.second().highlighted_id();
        let reset = filtered || handovers_changed;
        self.highlight_bottom();
        if !self.following && !reset {
            if let Some(id) = highlighted {
                self.view.second_mut().highlight_id(&id);
            }
        } else {
            self.set_following(true);
        }
        if reset {
            self.focus_feed = true;
        }
        self.view.second_mut().take_events();
        self.view
            .second_mut()
            .set_empty_state(tuicore::SeasonalEmptyState::new(if handovers_only {
                "No events handed over to Tandem instances."
            } else {
                "No events received. Start a provider to populate this feed."
            }));
        self.status = status;
        self.handovers_only = handovers_only;
        self.snapshot = snapshot;
        true
    }

    fn row(&self, id: i64) -> Option<&Record> {
        self.snapshot.records.iter().find(|row| row.sequence == id)
    }

    fn set_following(&mut self, following: bool) {
        self.following = following;
        self.view
            .first_mut()
            .second_mut()
            .first_mut()
            .second_mut()
            .set_value(following);
    }

    fn highlight_bottom(&mut self) {
        let ids: Vec<_> = self
            .view
            .second()
            .rows()
            .iter()
            .rev()
            .map(|row| row.sequence)
            .collect();
        self.newest_visible = ids
            .into_iter()
            .find(|id| self.view.second_mut().highlight_id(id).handled);
    }

    fn return_to_feed(&mut self, ctx: &mut EventCtx<Msg>) {
        if self.view.second().is_searching() {
            let area = self.view.child_areas().1;
            self.view.second_mut().on_key(tuicore::Key::Enter, area);
        }
        ctx.focus(tuicore::FocusRequest::Target(FocusId::new(FOCUS)));
        self.focus_feed = false;
        ctx.request_layout();
        ctx.request_redraw();
    }

    fn reset(&mut self, ctx: &mut EventCtx<Msg>) {
        self.view.first_mut().first_mut().cancel();
        *self.filter.borrow_mut() = Some(Vec::new());
        self.view
            .first_mut()
            .second_mut()
            .first_mut()
            .first_mut()
            .set_value(false);
        self.view.second_mut().clear_search();
        self.view.second_mut().clear_filters();
        self.sync();
        self.follow_bottom(ctx);
    }

    fn follow_bottom(&mut self, ctx: &mut EventCtx<Msg>) {
        self.set_following(true);
        self.highlight_bottom();
        self.view.second_mut().reveal_highlighted();
        self.view.second_mut().take_events();
        self.return_to_feed(ctx);
    }

    fn action(&mut self, event: &TuiEvent, ctx: &mut EventCtx<Msg>) -> bool {
        if super::App::overview_requested(event) {
            self.reset(ctx);
            ctx.stop_propagation();
            return true;
        }
        if super::App::returns_to_data_view(event)
            && (!self.view.second().is_focused()
                || self.view.second().is_searching()
                || self.view.first().first().is_open())
        {
            self.view.first_mut().first_mut().cancel();
            self.return_to_feed(ctx);
            ctx.stop_propagation();
            return true;
        }
        if self.view.second().is_focused()
            && !self.view.second().is_searching()
            && matches!(event, TuiEvent::Hotkey(tuicore::HotkeyEvent::Commit(sequence)) if sequence == "shift+g")
        {
            self.follow_bottom(ctx);
            ctx.stop_propagation();
            return true;
        }
        if self.view.second().is_searching() || self.view.first().first().is_open() {
            return false;
        }
        let TuiEvent::Key(key) = event else {
            return false;
        };
        if KeySpec::shifted('p').matches(*key) {
            self.view.first_mut().first_mut().open_with_context(ctx);
            ctx.stop_propagation();
            return true;
        }
        if KeySpec::shifted('t').matches(*key) {
            self.view
                .first_mut()
                .second_mut()
                .first_mut()
                .first_mut()
                .event(event, ctx);
            self.drain(ctx);
            ctx.stop_propagation();
            return true;
        }
        if tuicore::keybindings().data_view().bottom_matches(*key)
            || tuicore::keybindings().end_matches(*key)
        {
            self.follow_bottom(ctx);
            ctx.stop_propagation();
            return true;
        }
        let Some(id) = self.view.second().highlighted_id() else {
            return false;
        };
        if KeySpec::plain('.').matches(*key) {
            if let Some(row) = self.row(id) {
                ctx.emit(Msg::OpenRowMenu(super::row_actions::Target::Event(
                    Box::new(row.clone()),
                )));
            }
        } else if KeySpec::plain('p').matches(*key) {
            if let Some(row) = self.row(id) {
                ctx.emit(Msg::ShowProvider(row.provider.clone()));
            }
        } else if KeySpec::plain('r').matches(*key) {
            ctx.emit(Msg::ReplayEvent(id));
        } else {
            return false;
        }
        ctx.stop_propagation();
        true
    }

    fn drain(&mut self, ctx: &mut EventCtx<Msg>) {
        let following = self.view.first().second().first().second().is_checked();
        if following != self.following {
            self.following = following;
            if following {
                self.highlight_bottom();
                self.view.second_mut().reveal_highlighted();
            }
        }
        let mut transform_changed = false;
        for event in self.view.second_mut().take_events() {
            match event {
                DataViewTypedEvent::Activated { row_id } => {
                    if let Some(row) = self.row(row_id) {
                        ctx.emit(Msg::OpenEvent(Box::new(row.clone())));
                    }
                }
                DataViewTypedEvent::HighlightChanged { row_id } => {
                    self.set_following(row_id.is_some() && row_id == self.newest_visible);
                }
                DataViewTypedEvent::TransformChanged { .. } => transform_changed = true,
                _ => {}
            }
        }
        if transform_changed {
            self.highlight_bottom();
            self.set_following(true);
            self.view.second_mut().take_events();
        }
        if self.sync() {
            ctx.request_layout();
            ctx.request_redraw();
        }
        if self.focus_feed {
            self.return_to_feed(ctx);
        }
    }
}

pub(super) fn clean(text: &str) -> String {
    text.chars()
        .map(|character| {
            if character.is_control()
                || matches!(character, '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}')
            {
                ' '
            } else {
                character
            }
        })
        .collect()
}

fn search_text(row: &Record) -> String {
    format!(
        "{} {} {} {} {}",
        row.provider,
        row.event.stream,
        row.event.summary,
        row.event.event_id,
        row.event.payload.profile()
    )
}

impl TuiNode<Msg> for Events {
    fn measure(&self, proposal: LayoutProposal) -> LayoutSizeHint {
        <EventView as TuiNode<Msg>>::measure(&self.view, proposal)
    }
    fn layout(&mut self, area: Rect, ctx: &mut LayoutCtx) -> LayoutResult {
        let data = self.view.second_mut();
        let alternate: BTreeSet<_> = data
            .rows()
            .iter()
            .filter(|row| {
                tuicore::search_match(
                    &data.transform_state().search,
                    &search_text(row),
                    tuicore::SearchMode::Fuzzy,
                )
                .is_some()
            })
            .enumerate()
            .filter(|(index, _)| index % 2 == 0)
            .map(|(_, row)| row.sequence)
            .collect();
        data.set_row_style_by(move |row| {
            alternate
                .contains(&row.sequence)
                .then(|| Style::default().bg(tuicore::theme().surface_bg()))
        });
        let provider_width = <Dropdown<String, String> as TuiNode<Msg>>::measure(
            self.view.first().first(),
            LayoutProposal::unbounded(),
        )
        .preferred
        .width;
        let toggle_width = <Toggle<Msg> as TuiNode<Msg>>::measure(
            self.view.first().second().first().first(),
            LayoutProposal::unbounded(),
        )
        .preferred
        .width;
        let follow_width = <Toggle<Msg> as TuiNode<Msg>>::measure(
            self.view.first().second().first().second(),
            LayoutProposal::unbounded(),
        )
        .preferred
        .width;
        let controls_width = toggle_width.saturating_add(follow_width).saturating_add(1);
        let status_width =
            line_width(&Line::from(self.status.as_str())).min(u16::MAX as usize) as u16;
        self.view.first_mut().set_constraints(
            Constraint::Length(
                area.width
                    .saturating_sub(
                        controls_width
                            .saturating_add(status_width)
                            .saturating_add(2),
                    )
                    .max(1)
                    .min(area.width)
                    .min(provider_width),
            ),
            Constraint::Fill(1),
        );
        self.view
            .first_mut()
            .second_mut()
            .set_constraints(Constraint::Length(controls_width), Constraint::Fill(1));
        self.view
            .first_mut()
            .second_mut()
            .first_mut()
            .set_constraints(
                Constraint::Length(toggle_width),
                Constraint::Length(follow_width),
            );
        let result = <EventView as TuiNode<Msg>>::layout(&mut self.view, area, ctx);
        if self.following {
            self.view.second_mut().reveal_highlighted();
        }
        let width = self.view.first().second().child_areas().1.width;
        self.view
            .first_mut()
            .second_mut()
            .second_mut()
            .set_text(format!(
                "{}{}",
                " ".repeat(usize::from(width.saturating_sub(status_width))),
                self.status,
            ));
        result
    }
    fn render<'a>(&'a self, frame: &mut Frame, area: Rect, ctx: &mut RenderCtx<'a>) {
        <EventView as TuiNode<Msg>>::render(&self.view, frame, area, ctx);
    }
    fn event(&mut self, event: &TuiEvent, ctx: &mut EventCtx<Msg>) -> EventOutcome {
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
        let feed_route = route
            .path
            .keys()
            .first()
            .is_some_and(|key| key.as_str() == "second");
        let controls_escape = !feed_route && super::App::returns_to_data_view(event);
        if controls_escape {
            self.view.first_mut().first_mut().cancel();
            self.return_to_feed(ctx);
            ctx.stop_propagation();
            return EventOutcome::Handled;
        }
        let follow_hotkey = self.view.second().is_focused()
            && matches!(event, TuiEvent::Hotkey(tuicore::HotkeyEvent::Commit(sequence)) if sequence == "shift+g");
        if (feed_route || follow_hotkey || super::App::overview_requested(event))
            && self.action(event, ctx)
        {
            return EventOutcome::Handled;
        }
        let outcome = self.view.dispatch_event(route, event, ctx);
        self.drain(ctx);
        outcome
    }
    fn tick(&mut self, dt: Duration, settings: AnimationSettings) -> TickResult {
        let changed = self.sync();
        <EventView as TuiNode<Msg>>::tick(&mut self.view, dt, settings).merge(if changed {
            TickResult {
                layout: true,
                ..TickResult::CHANGED
            }
        } else {
            TickResult::IDLE
        })
    }
    fn take_pending_focus_request(&mut self) -> Option<tuicore::FocusRequest> {
        if std::mem::take(&mut self.focus_feed) {
            return Some(tuicore::FocusRequest::Target(FocusId::new(FOCUS)));
        }
        <EventView as TuiNode<Msg>>::take_pending_focus_request(&mut self.view)
    }
    fn take_pending_clipboard_request(&mut self) -> Option<String> {
        <EventView as TuiNode<Msg>>::take_pending_clipboard_request(&mut self.view)
    }
    fn focus(&mut self, target: Option<&FocusId>, focused: bool, ctx: &mut FocusCtx<Msg>) {
        self.view.focus(target, focused, ctx);
    }
    fn dispatch_focus(&mut self, target: &FocusTarget, focused: bool, ctx: &mut FocusCtx<Msg>) {
        self.view.dispatch_focus(target, focused, ctx);
    }
    fn focus_reveal_area(&self, target: &FocusTarget) -> Option<Rect> {
        <EventView as TuiNode<Msg>>::focus_reveal_area(&self.view, target)
    }
    fn focus_reveal_centered(&self, target: &FocusTarget) -> bool {
        <EventView as TuiNode<Msg>>::focus_reveal_centered(&self.view, target)
    }
    fn init(&mut self, ctx: &mut LifecycleCtx<Msg>) {
        self.view.init(ctx);
    }
    fn mount(&mut self, ctx: &mut LifecycleCtx<Msg>) {
        self.view.mount(ctx);
    }
    fn unmount(&mut self, ctx: &mut LifecycleCtx<Msg>) {
        self.view.unmount(ctx);
    }
    fn destroy(&mut self, ctx: &mut LifecycleCtx<Msg>) {
        self.view.destroy(ctx);
    }
}

impl super::App {
    pub(super) fn sync_overview_tab(&mut self, ctx: &mut EventCtx<Msg>) {
        let index = self.tabs_mut().selected_index();
        if index == 1 {
            self.events_active = true;
            self.providers_active = false;
            self.pages_mut().select_events();
        } else if index == 2 {
            if !self.providers_active {
                ctx.focus(tuicore::FocusRequest::Target(FocusId::new(
                    super::providers::FOCUS,
                )));
            }
            self.providers_active = true;
            self.events_active = false;
            self.pages_mut().select_providers();
        } else {
            let sessions = self.service.opencode_enabled() && index == 0;
            if self.events_active
                || self.providers_active
                || sessions != self.attached_sessions_only
            {
                self.handle_message(Msg::SetAttachedSessionsOnly(sessions), ctx);
            }
        }
    }

    pub(super) fn open_event(&mut self, row: &Record, ctx: &mut EventCtx<Msg>) {
        let content = serde_json::to_string_pretty(row).unwrap_or_else(|error| error.to_string());
        let content: String = content
            .chars()
            .filter(
                |character| !matches!(character, '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}'),
            )
            .collect();
        let modal = super::dialogs::dialog("Event details").host(
            tuicore::Flex::column().child(
                "event-json",
                tuicore::SyntaxHighlighter::new(
                    content,
                    tuicore::Language::guess(Some("event.json"), ""),
                )
                .wrap(true),
                tuicore::FlexItem::fill(1),
            ),
        );
        self.intent = None;
        self.open(Box::new(modal), ctx);
        self.details_open = true;
        self.resize_details_dialog();
    }

    pub(super) fn poll_event_action(&mut self) -> bool {
        let result = match self.event_action.as_mut() {
            Some(reply) => match reply.try_recv() {
                Ok(result) => result,
                Err(tokio::sync::oneshot::error::TryRecvError::Empty) => return false,
                Err(tokio::sync::oneshot::error::TryRecvError::Closed) => Err(
                    crate::store::events::Error::Storage("event worker stopped".into()),
                ),
            },
            None => return false,
        };
        self.event_action = None;
        match result {
            Ok(_) => self.service.poll_events(),
            Err(error) => self.notify(tuicore::Notification::error(
                "Event action failed",
                error.to_string(),
            )),
        }
        true
    }
}
