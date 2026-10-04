use std::{cell::RefCell, collections::BTreeSet, rc::Rc, time::Duration};

use ratatui::{
    Frame,
    layout::{Alignment, Constraint, Rect},
    style::Style,
    text::Line,
};
use tuicore::{
    Animated, AnimationSettings, Button, Column, DataView, DataViewTypedEvent, Dropdown,
    DropdownLabelPosition, DropdownVariant, EventCtx, EventOutcome, EventRoute, FocusCtx, FocusId,
    FocusTarget, HotkeyLabelMode, KeySpec, LayoutCtx, LayoutProposal, LayoutResult, LayoutSizeHint,
    LifecycleCtx, Paragraph, RenderCtx, Spinner, Split, TickResult, Toggle, TreeAdapter, TuiEvent,
    TuiNode, line_width,
};

use super::Msg;
use crate::store::events::{Record, Snapshot};

mod filters;
mod rows;
mod tree;
pub(super) use filters::StreamKey;
pub(super) use rows::{profile_icon, row_text};
use tree::Entry;

pub(super) const FOCUS: &str = "event-feed";
pub(super) type SharedState = Rc<RefCell<Snapshot>>;
pub(super) type FilterState = Rc<RefCell<Option<Vec<StreamKey>>>>;
#[derive(Default)]
pub(super) struct FocusRequest {
    pub(super) record: Option<Record>,
    pub(super) deletion: Option<Option<i64>>,
    pub(super) reset_filters: bool,
}
pub(super) type FocusState = Rc<RefCell<FocusRequest>>;
type EventToggles = Split<Toggle<Msg>, Split<Toggle<Msg>, Split<Toggle<Msg>, Toggle<Msg>>>>;
type EventStatus = Split<Split<Paragraph, Button<Msg>>, Paragraph>;
type EventControls = Split<Dropdown<StreamKey, StreamKey>, EventStatus>;
type HeaderRow = Split<EventToggles, EventControls>;
type EventView = Split<Split<HeaderRow, Paragraph>, DataView<Entry, String>>;

pub(super) struct Events {
    view: EventView,
    shared: SharedState,
    snapshot: Snapshot,
    filter: FilterState,
    sources: Vec<StreamKey>,
    providers: super::providers::SharedState,
    stream_sources: Vec<StreamKey>,
    handovers_only: bool,
    following: bool,
    newest_visible: Option<String>,
    focus_feed: bool,
    status: String,
    requested: FocusState,
    pinned: Option<Record>,
    toolbar: super::toolbar::SharedState,
    rules: super::rules::SharedState,
    context: super::acceptances::SharedContext,
    projected: Vec<Entry>,
    spinner: Rc<RefCell<Spinner>>,
}

impl Events {
    pub(super) fn new(
        shared: SharedState,
        filter: FilterState,
        providers: super::providers::SharedState,
        requested: FocusState,
        toolbar: super::toolbar::SharedState,
        rules: super::rules::SharedState,
        context: super::acceptances::SharedContext,
    ) -> Self {
        let selection = filter.clone();
        let stream_filter = Dropdown::multi(
            Vec::new(),
            |source: &StreamKey| source.clone(),
            StreamKey::label,
        )
        .variant(DropdownVariant::Filled)
        .label("Streams")
        .label_position(DropdownLabelPosition::Inline)
        .placeholder("All streams")
        .show_multi_labels(true)
        .max_popup_width(u16::MAX)
        .hotkey("shift+p")
        .on_select(move |names| *selection.borrow_mut() = Some(names));
        let spinner = Rc::new(RefCell::new(Spinner::new()));
        let cell_spinner = spinner.clone();
        let memory_spinner = spinner.clone();
        let cpu_spinner = spinner.clone();
        let data = DataView::new(Vec::new(), Entry::id)
            .focus_id(FOCUS)
            .hotkey("shift+h")
            .columns(vec![
                Column::multiline(
                    "event",
                    "Events",
                    Constraint::Fill(1),
                    move |row: &Entry, ctx| {
                        row.text(cell_spinner.borrow().glyph(), ctx.available_width)
                    },
                )
                .search_key(Entry::search)
                .constrained(),
                Column::multiline(
                    "memory",
                    "Memory",
                    Constraint::Min(0),
                    move |row: &Entry, _| {
                        let mut line = row.memory(memory_spinner.borrow().glyph());
                        line.alignment = Some(Alignment::Right);
                        line
                    },
                )
                .fit_content(),
                Column::multiline("cpu", "CPU", Constraint::Min(0), move |row: &Entry, _| {
                    let mut line = row.cpu(cpu_spinner.borrow().glyph());
                    line.alignment = Some(Alignment::Right);
                    line
                })
                .fit_content(),
            ])
            .headers(false)
            .action_bar(true)
            .filter_controls(false)
            .row_height_by(Entry::height)
            .tree(TreeAdapter::parent_id(Entry::parent))
            .empty_state(tuicore::SeasonalEmptyState::new(
                "No events handed over to Tandem instances.",
            ));
        let header = Split::horizontal(
            Split::horizontal(
                Toggle::new("󰈈")
                    .hotkey("shift+a")
                    .preserve_focus_on_hotkey(true)
                    .on_change(|show_all| Msg::SetRunningOnly(!show_all)),
                Split::horizontal(
                    Toggle::new("󰋚")
                        .hotkey("shift+o")
                        .preserve_focus_on_hotkey(true)
                        .on_change(Msg::SetOpencodeHistory),
                    Split::horizontal(
                        Toggle::new("󰕾")
                            .hotkey("shift+n")
                            .preserve_focus_on_hotkey(true)
                            .on_change(Msg::SetCompletionSound),
                        Toggle::new("󰞖")
                            .checked(true)
                            .hotkey("gg")
                            .preserve_focus_on_hotkey(true),
                    )
                    .gap(1),
                )
                .gap(1),
            )
            .gap(1),
            Split::horizontal(
                stream_filter,
                Split::horizontal(
                    Split::horizontal(
                        Paragraph::new("").wrap(false),
                        Button::new("Delete all events")
                            .hotkey("shift+x")
                            .hotkey_focus_enabled(false)
                            .hotkey_label_mode(HotkeyLabelMode::Inline)
                            .on_press(|| Msg::DeleteEvents(None)),
                    ),
                    Paragraph::new("Waiting for event providers").wrap(false),
                )
                .gap(1),
            )
            .gap(1),
        )
        .constraints(Constraint::Length(1), Constraint::Fill(1))
        .gap(1);
        Self {
            view: Split::vertical(
                Split::vertical(header, Paragraph::new("").wrap(false))
                    .constraints(Constraint::Length(1), Constraint::Length(0)),
                data,
            )
            .constraints(Constraint::Length(1), Constraint::Fill(1)),
            shared,
            snapshot: Snapshot::default(),
            filter,
            sources: Vec::new(),
            providers,
            stream_sources: Vec::new(),
            handovers_only: true,
            following: true,
            newest_visible: None,
            focus_feed: false,
            status: "Waiting for event providers".into(),
            requested,
            pinned: None,
            toolbar,
            rules,
            context,
            projected: Vec::new(),
            spinner,
        }
    }

    fn header(&self) -> &HeaderRow {
        self.view.first().first()
    }

    fn header_mut(&mut self) -> &mut HeaderRow {
        self.view.first_mut().first_mut()
    }

    fn streams(&self) -> &Dropdown<StreamKey, StreamKey> {
        self.header().second().first()
    }

    fn streams_mut(&mut self) -> &mut Dropdown<StreamKey, StreamKey> {
        self.header_mut().second_mut().first_mut()
    }

    fn toggles(&self) -> &EventToggles {
        self.header().first()
    }

    fn toggles_mut(&mut self) -> &mut EventToggles {
        self.header_mut().first_mut()
    }

    fn sync(&mut self) -> bool {
        let sound = self.toolbar.borrow().completion_sound;
        let show_saved = self.toolbar.borrow().show_saved;
        let handovers_only = self.toolbar.borrow().running_only;
        self.toggles_mut().first_mut().set_value(!handovers_only);
        let history = self.toggles_mut().second_mut().first_mut();
        let history_changed = history.is_checked() != show_saved;
        history.set_value(show_saved);
        let sound_toggle = self.toggles_mut().second_mut().second_mut().first_mut();
        let sound_changed = sound_toggle.is_checked() != sound;
        sound_toggle.set_value(sound);
        let deletion = self.requested.borrow_mut().deletion.take();
        if let Some(sequence) = deletion
            && self
                .pinned
                .as_ref()
                .is_some_and(|row| sequence.is_none_or(|id| id == row.sequence))
        {
            self.pinned = None;
        }
        let requested = self.requested.borrow_mut().record.take();
        let reset_filters = std::mem::take(&mut self.requested.borrow_mut().reset_filters);
        if reset_filters {
            self.view.second_mut().clear_search();
            self.view.second_mut().clear_filters();
            self.streams_mut().cancel();
        }
        let selected = requested.as_ref().map(|record| record.sequence);
        if let Some(record) = requested {
            self.pinned = Some(record);
            *self.filter.borrow_mut() = Some(Vec::new());
            self.view.second_mut().clear_search();
            self.view.second_mut().clear_filters();
            self.streams_mut().cancel();
            self.set_following(false);
        }
        let filter = self.filter.borrow_mut().take();
        let filtered = filter.is_some();
        if let Some(names) = filter {
            self.sources = names;
        }
        let snapshot = self.shared.borrow().clone();
        let stream_sources = filters::inventory(
            &self.providers.borrow().snapshot,
            &snapshot.records,
            &self.sources,
            self.pinned.as_ref(),
        );
        let streams_changed = stream_sources != self.stream_sources;
        let handovers_changed = handovers_only != self.handovers_only;
        if streams_changed {
            self.streams_mut().set_rows(stream_sources.clone());
            self.stream_sources = stream_sources;
        }
        if filtered || streams_changed {
            let sources = self.sources.clone();
            self.streams_mut().set_selected(sources);
        }
        let mut records: Vec<_> = snapshot
            .records
            .iter()
            .filter(|row| {
                self.sources.is_empty() || self.sources.iter().any(|source| source.matches(row))
            })
            .filter(|row| {
                !handovers_only
                    || row
                        .acceptances
                        .iter()
                        .any(|acceptance| acceptance.operation_id.is_some())
            })
            .cloned()
            .collect();
        if let Some(pinned) = &self.pinned
            && !records
                .iter()
                .any(|record| record.sequence == pinned.sequence)
            && (self.sources.is_empty() || self.sources.iter().any(|source| source.matches(pinned)))
            && (!handovers_only
                || pinned
                    .acceptances
                    .iter()
                    .any(|acceptance| acceptance.operation_id.is_some()))
        {
            records.push(pinned.clone());
        }
        records.sort_by_key(|row| std::cmp::Reverse(row.sequence));
        let projected = tree::project(
            &records,
            &self.rules.borrow(),
            &self.context.borrow(),
            self.toolbar.borrow().show_saved,
        );
        if snapshot == self.snapshot
            && projected == self.projected
            && !filtered
            && !streams_changed
            && !handovers_changed
            && !reset_filters
            && deletion.is_none()
        {
            return sound_changed || history_changed;
        }
        let status = snapshot
            .error
            .as_deref()
            .map(clean)
            .unwrap_or_else(|| format!("{} of {} events", records.len(), snapshot.total));
        self.view.second_mut().set_rows(projected.clone());
        self.projected = projected;
        let highlighted = self.view.second().highlighted_id();
        let reset = filtered || handovers_changed;
        self.highlight_top();
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
        if let Some(id) = selected {
            self.view.second_mut().highlight_id(&format!("event:{id}"));
            self.view.second_mut().reveal_highlighted();
            self.set_following(false);
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
        self.snapshot
            .records
            .iter()
            .find(|row| row.sequence == id)
            .or_else(|| self.pinned.as_ref().filter(|row| row.sequence == id))
    }

    fn set_following(&mut self, following: bool) {
        self.following = following;
        self.toggles_mut()
            .second_mut()
            .second_mut()
            .second_mut()
            .set_value(following);
    }

    fn highlight_top(&mut self) {
        let ids: Vec<_> = self
            .view
            .second()
            .rows()
            .iter()
            .filter_map(|row| {
                if let Entry::Event(_) = row {
                    Some(row.id())
                } else {
                    None
                }
            })
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
        self.toolbar.borrow_mut().completion_sound = false;
        ctx.emit(Msg::SetCompletionSound(false));
        self.pinned = None;
        self.streams_mut().cancel();
        *self.filter.borrow_mut() = Some(Vec::new());
        self.toolbar.borrow_mut().running_only = true;
        ctx.emit(Msg::SetRunningOnly(true));
        self.view.second_mut().clear_search();
        self.view.second_mut().clear_filters();
        self.sync();
        self.follow_top(ctx);
    }

    fn follow_top(&mut self, ctx: &mut EventCtx<Msg>) {
        self.set_following(true);
        self.highlight_top();
        self.view.second_mut().reveal_highlighted();
        self.view.second_mut().take_events();
        self.return_to_feed(ctx);
    }

    fn action(&mut self, event: &TuiEvent, ctx: &mut EventCtx<Msg>) -> bool {
        if matches!(event, TuiEvent::Key(key) if KeySpec::key_with_modifiers(tuicore::Key::Enter, tuicore::KeyModifiers::SHIFT).matches(*key))
        {
            self.streams_mut().cancel();
            *self.filter.borrow_mut() = Some(Vec::new());
            self.sync();
            self.view.second_mut().clear_selection();
            ctx.request_layout();
            ctx.request_redraw();
            ctx.stop_propagation();
            return true;
        }
        if super::App::overview_requested(event) {
            self.reset(ctx);
            ctx.stop_propagation();
            return true;
        }
        if super::App::returns_to_data_view(event)
            && (!self.view.second().is_focused()
                || self.view.second().is_searching()
                || self.streams().is_open())
        {
            self.streams_mut().cancel();
            self.return_to_feed(ctx);
            ctx.stop_propagation();
            return true;
        }
        if self.view.second().is_focused()
            && !self.view.second().is_searching()
            && matches!(event, TuiEvent::Hotkey(tuicore::HotkeyEvent::Commit(sequence)) if sequence == "gg")
        {
            self.follow_top(ctx);
            ctx.stop_propagation();
            return true;
        }
        if self.view.second().is_searching() || self.streams().is_open() {
            return false;
        }
        let TuiEvent::Key(key) = event else {
            return false;
        };
        if KeySpec::shifted('p').matches(*key) {
            self.streams_mut().open_with_context(ctx);
            ctx.stop_propagation();
            return true;
        }
        if KeySpec::shifted('a').matches(*key) {
            self.toggles_mut().first_mut().event(event, ctx);
            self.drain(ctx);
            ctx.stop_propagation();
            return true;
        }
        if KeySpec::shifted('n').matches(*key) {
            self.toggles_mut()
                .second_mut()
                .second_mut()
                .first_mut()
                .event(
                    &TuiEvent::Hotkey(tuicore::HotkeyEvent::Commit("shift+n".into())),
                    ctx,
                );
            self.drain(ctx);
            ctx.stop_propagation();
            return true;
        }
        if KeySpec::shifted('o').matches(*key) {
            self.toggles_mut().second_mut().first_mut().event(
                &TuiEvent::Hotkey(tuicore::HotkeyEvent::Commit("shift+o".into())),
                ctx,
            );
            ctx.stop_propagation();
            return true;
        }
        if tuicore::keybindings().home_matches(*key) {
            self.follow_top(ctx);
            ctx.stop_propagation();
            return true;
        }
        if KeySpec::shifted('x').matches(*key) {
            ctx.emit(Msg::DeleteEvents(None));
            ctx.stop_propagation();
            return true;
        }
        let Some(id) = self.view.second().highlighted_id() else {
            return false;
        };
        let Some(entry) = self.projected.iter().find(|row| row.id() == id) else {
            return false;
        };
        if let Some(target) = entry.target() {
            return target.action(event, ctx);
        }
        let Entry::Event(record) = entry else {
            return false;
        };
        let id = record.sequence;
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
        } else if KeySpec::plain('x').matches(*key) {
            ctx.emit(Msg::DeleteEvents(Some(id)));
        } else {
            return false;
        }
        ctx.stop_propagation();
        true
    }

    fn drain(&mut self, ctx: &mut EventCtx<Msg>) {
        let following = self.toggles().second().second().second().is_checked();
        if following != self.following {
            self.following = following;
            if following {
                self.highlight_top();
                self.view.second_mut().reveal_highlighted();
            }
        }
        let mut transform_changed = false;
        for event in self.view.second_mut().take_events() {
            match event {
                DataViewTypedEvent::Activated { row_id } => {
                    if let Some(row) = self.projected.iter().find(|row| row.id() == row_id) {
                        match row {
                            Entry::Event(row) => ctx.emit(Msg::OpenEvent(row.clone())),
                            Entry::Acceptance(target) => {
                                ctx.emit(Msg::FocusRule(target.acceptance.rule_name.clone()))
                            }
                            Entry::Conversation { .. } => ctx.emit(Msg::AcceptanceAction(
                                Box::new(row.target().unwrap()),
                                super::row_actions::Command::Details,
                            )),
                        }
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
            self.highlight_top();
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
        "#{} {} {} {} {} {}",
        row.sequence,
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
        self.sync();
        let data = self.view.second_mut();
        let alternate: BTreeSet<_> = data
            .rows()
            .iter()
            .filter(|row| {
                tuicore::search_match(
                    &data.transform_state().search,
                    &row.search(),
                    tuicore::SearchMode::Fuzzy,
                )
                .is_some()
            })
            .enumerate()
            .filter(|(index, _)| index % 2 == 0)
            .map(|(_, row)| row.id())
            .collect();
        data.set_row_style_by(move |row| {
            alternate
                .contains(&row.id())
                .then(|| Style::default().bg(tuicore::theme().surface_bg()))
        });
        let stream_width = <Dropdown<StreamKey, StreamKey> as TuiNode<Msg>>::measure(
            self.streams(),
            LayoutProposal::unbounded(),
        )
        .preferred
        .width;
        let toggle_width = <Toggle<Msg> as TuiNode<Msg>>::measure(
            self.toggles().first(),
            LayoutProposal::unbounded(),
        )
        .preferred
        .width;
        let follow_width = <Toggle<Msg> as TuiNode<Msg>>::measure(
            self.toggles().second().second().second(),
            LayoutProposal::unbounded(),
        )
        .preferred
        .width;
        let sound_width = <Toggle<Msg> as TuiNode<Msg>>::measure(
            self.toggles().second().second().first(),
            LayoutProposal::unbounded(),
        )
        .preferred
        .width;
        let history_width = <Toggle<Msg> as TuiNode<Msg>>::measure(
            self.toggles().second().first(),
            LayoutProposal::unbounded(),
        )
        .preferred
        .width;
        let compact = area.width < 60;
        let sound_width = sound_width.saturating_sub(if compact { 4 } else { 0 });
        let history_width = history_width.saturating_sub(if compact { 4 } else { 0 });
        let sound_follow_width = sound_width.saturating_add(follow_width).saturating_add(1);
        let history_sound_width = history_width
            .saturating_add(sound_follow_width)
            .saturating_add(1);
        let controls_width = toggle_width
            .saturating_add(history_sound_width)
            .saturating_add(1);
        let status_width =
            line_width(&Line::from(self.status.as_str())).min(u16::MAX as usize) as u16;
        let delete = self
            .header_mut()
            .second_mut()
            .second_mut()
            .first_mut()
            .second_mut();
        delete.set_label(if compact {
            " X"
        } else {
            "Delete all events"
        });
        delete.set_hotkey_label_mode(if compact {
            HotkeyLabelMode::PreferMnemonic
        } else {
            HotkeyLabelMode::Inline
        });
        let delete_width = self
            .header()
            .second()
            .second()
            .first()
            .second()
            .measure(LayoutProposal::unbounded())
            .preferred
            .width;
        let narrow = area.width
            < controls_width
                .saturating_add(delete_width)
                .saturating_add(status_width)
                .saturating_add(stream_width.min(20))
                .saturating_add(4);
        self.view.set_constraints(
            Constraint::Length(if narrow { 2 } else { 1 }),
            Constraint::Fill(1),
        );
        self.view
            .first_mut()
            .set_constraints(Constraint::Length(1), Constraint::Length(u16::from(narrow)));
        let inline_status_width = if narrow { 0 } else { status_width };
        let stream_width = area
            .width
            .saturating_sub(
                controls_width
                    .saturating_add(delete_width)
                    .saturating_add(inline_status_width)
                    .saturating_add(3),
            )
            .max(1)
            .min(area.width)
            .min(stream_width);
        self.header_mut().set_constraints(
            Constraint::Fill(1),
            Constraint::Length(stream_width + delete_width + inline_status_width + 2),
        );
        self.header_mut()
            .second_mut()
            .set_constraints(Constraint::Length(stream_width), Constraint::Fill(1));
        self.toggles_mut().set_constraints(
            Constraint::Length(toggle_width),
            Constraint::Length(history_sound_width),
        );
        self.toggles_mut().second_mut().set_constraints(
            Constraint::Length(history_width),
            Constraint::Length(sound_follow_width),
        );
        self.toggles_mut()
            .second_mut()
            .second_mut()
            .set_constraints(
                Constraint::Length(sound_width),
                Constraint::Length(follow_width),
            );
        let status = self.header_mut().second_mut().second_mut();
        status.set_constraints(Constraint::Length(delete_width), Constraint::Fill(1));
        status
            .first_mut()
            .set_constraints(Constraint::Length(0), Constraint::Length(delete_width));
        status.second_mut().set_text("");
        self.view.first_mut().second_mut().set_text("");
        let result = <EventView as TuiNode<Msg>>::layout(&mut self.view, area, ctx);
        if self.following {
            self.view.second_mut().reveal_highlighted();
        }
        let width = if narrow {
            self.view.first().child_areas().1.width
        } else {
            self.header().second().second().child_areas().1.width
        };
        let status = format!(
            "{}{}",
            " ".repeat(usize::from(width.saturating_sub(status_width))),
            self.status,
        );
        if narrow {
            self.view.first_mut().second_mut().set_text(status);
        } else {
            self.header_mut()
                .second_mut()
                .second_mut()
                .second_mut()
                .set_text(status);
        }
        result
    }
    fn render<'a>(&'a self, frame: &mut Frame, area: Rect, ctx: &mut RenderCtx<'a>) {
        <EventView as TuiNode<Msg>>::render(&self.view, frame, area, ctx);
    }
    fn event(&mut self, event: &TuiEvent, ctx: &mut EventCtx<Msg>) -> EventOutcome {
        self.sync();
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
        self.sync();
        let feed_route = route
            .path
            .keys()
            .first()
            .is_some_and(|key| key.as_str() == "second");
        let controls_escape = !feed_route && super::App::returns_to_data_view(event);
        if controls_escape {
            self.streams_mut().cancel();
            self.return_to_feed(ctx);
            ctx.stop_propagation();
            return EventOutcome::Handled;
        }
        let follow_hotkey = self.view.second().is_focused()
            && matches!(event, TuiEvent::Hotkey(tuicore::HotkeyEvent::Commit(sequence)) if sequence == "gg");
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
        let result = <EventView as TuiNode<Msg>>::tick(&mut self.view, dt, settings).merge(
            Animated::tick(&mut *self.spinner.borrow_mut(), dt, settings),
        );
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
    pub(super) fn confirm_delete_events(&mut self, sequence: Option<i64>, ctx: &mut EventCtx<Msg>) {
        let (title, prose) = if sequence.is_some() {
            ("Delete event", "Delete this event and history?")
        } else {
            ("Delete all events", "Delete all events and history?")
        };
        let modal = super::dialogs::dialog(title)
            .actions([
                tuicore::DialogAction::new("Ok")
                    .hotkey(KeySpec::plain('o'))
                    .on_trigger(move || Msg::DeleteEventsConfirmed(sequence)),
                tuicore::DialogAction::new("Cancel")
                    .hotkey(KeySpec::plain('c'))
                    .on_trigger(|| Msg::Close),
            ])
            .host(tuicore::Flex::column().child(
                "warning",
                Paragraph::new(prose),
                tuicore::FlexItem::fit_content(),
            ));
        self.intent = None;
        self.open_compact(Box::new(modal), ctx);
    }

    pub(super) fn update_event_snapshot(&mut self, snapshot: Snapshot) -> bool {
        if snapshot.error.is_none()
            && let Some(count) = snapshot.accepted_attempts
        {
            if self.completion_sound
                && self
                    .event_acceptance_count
                    .is_some_and(|previous| count > previous)
            {
                self.service.play_event_acceptance_sound();
            }
            self.event_acceptance_count = Some(count);
        }
        self.pages_mut().update_events(snapshot)
    }

    pub(super) fn sync_overview_tab(&mut self, ctx: &mut EventCtx<Msg>) {
        let index = self.tabs_mut().selected_index();
        if index == 1 {
            self.rules_active = false;
            self.events_active = true;
            self.providers_active = false;
            self.pages_mut().select_events();
        } else if index == self.providers_tab_index() {
            self.rules_active = false;
            if !self.providers_active {
                ctx.focus(tuicore::FocusRequest::Target(FocusId::new(
                    super::providers::FOCUS,
                )));
            }
            self.providers_active = true;
            self.events_active = false;
            self.pages_mut().select_providers();
        } else if index == self.rules_tab_index() {
            if !self.rules_active {
                ctx.focus(tuicore::FocusRequest::Target(FocusId::new(
                    super::rules::FOCUS,
                )));
            }
            self.rules_active = true;
            self.events_active = false;
            self.providers_active = false;
            self.pages_mut().select_rules();
        } else {
            let sessions = self.service.opencode_enabled() && index == 0;
            if self.events_active
                || self.providers_active
                || self.rules_active
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
        let modal = tuicore::Tabs::dialog(vec![tuicore::Tab::new(
            "Event details",
            tuicore::SyntaxHighlighter::new(
                content,
                tuicore::Language::guess(Some("event.json"), ""),
            )
            .wrap(true),
        )])
        .variant(tuicore::TabsVariant::OneRow)
        .edge_borders(ratatui::widgets::Borders::TOP)
        .on_close(|_| Msg::Close);
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
        let deletion = self.event_deletion.take();
        match result {
            Ok(_) => {
                if let Some(sequence) = deletion {
                    self.pages_mut().forget_event(sequence);
                }
                self.service.poll_events();
                self.service.poll_rules();
            }
            Err(error) => self.notify(tuicore::Notification::error(
                "Event action failed",
                error.to_string(),
            )),
        }
        true
    }
}
