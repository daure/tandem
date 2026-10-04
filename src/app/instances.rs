use std::{
    cell::RefCell,
    collections::{HashMap, HashSet},
    rc::Rc,
    time::{Duration, Instant},
};

use ratatui::{
    Frame,
    layout::{Alignment, Constraint, Rect},
    style::Style,
    text::Span,
};
use tuicore::{
    Animated, AnimationSettings, Column, DataView, EventCtx, EventOutcome, EventRoute, FocusCtx,
    FocusId, FocusTarget, LayoutCtx, LayoutProposal, LayoutResult, LayoutSizeHint, LifecycleCtx,
    SelectionMode, Spinner, TickResult, TreeAdapter, TuiEvent, TuiNode,
};

use super::{
    Msg, TREE_FOCUS,
    rows::{self, Row},
};

const OPENCODE_WORKSPACES_ID: &str = "opencode-workspaces";

mod completion;
mod selection;

#[derive(Default)]
pub(super) struct State {
    rows: Vec<Row>,
    highlighted: Option<String>,
    searching: bool,
    rows_changed: bool,
    reset_view: bool,
    expand_view: bool,
    center_highlighted: bool,
    select_first: bool,
    select_created: Option<(bool, String)>,
    select_pane: Option<(crate::store::opencode::Pane, Instant)>,
    attached_sessions_only: bool,
    mode_changed: bool,
    pub(super) resource_right_gutter: u16,
    cpu_column_min_width: usize,
    completion_fade: Duration,
}

pub(super) type SharedState = Rc<RefCell<State>>;

pub(super) fn state(rows: Vec<Row>) -> SharedState {
    Rc::new(RefCell::new(State {
        rows,
        completion_fade: Duration::from_secs(crate::store::completion::DEFAULT_FADE_SECONDS),
        ..Default::default()
    }))
}

pub(super) fn selected(state: &SharedState) -> Option<Row> {
    let state = state.borrow();
    let highlighted = state.highlighted.as_ref()?;
    state
        .rows
        .iter()
        .find(|row| &row.id == highlighted)
        .cloned()
}

pub(super) fn selected_instance(state: &SharedState) -> Option<Row> {
    let state = state.borrow();
    let mut id = state.highlighted.as_ref()?;
    loop {
        let row = state.rows.iter().find(|row| &row.id == id)?;
        if row.instance.is_some() {
            return Some(row.clone());
        }
        id = row.parent.as_ref()?;
    }
}

pub(super) fn set_completion_fade(state: &SharedState, seconds: u64) {
    state.borrow_mut().completion_fade = Duration::from_secs(seconds);
}

pub(super) fn set_cpu_column_min_width(state: &SharedState, width: usize) {
    let mut state = state.borrow_mut();
    if state.cpu_column_min_width != width {
        state.cpu_column_min_width = width;
        state.rows_changed = true;
    }
}

pub(super) fn is_searching(state: &SharedState) -> bool {
    state.borrow().searching
}

pub(super) fn request_view_reset(state: &SharedState) {
    state.borrow_mut().reset_view = true;
}

pub(super) fn request_expanded_overview(state: &SharedState) {
    let mut state = state.borrow_mut();
    state.reset_view = true;
    state.expand_view = true;
}

pub(super) fn request_center_highlighted(state: &SharedState) {
    state.borrow_mut().center_highlighted = true;
}

pub(super) fn set_attached_sessions_only(state: &SharedState, enabled: bool) {
    let mut state = state.borrow_mut();
    if state.attached_sessions_only != enabled {
        state.attached_sessions_only = enabled;
        state.mode_changed = true;
        state.rows_changed = true;
    }
}

pub(super) fn replace_rows(state: &SharedState, rows: Vec<Row>) -> bool {
    let mut state = state.borrow_mut();
    if state.rows == rows {
        return false;
    }
    state.rows = rows;
    state.rows_changed = true;
    true
}

pub(super) fn replace_rows_and_select_first(state: &SharedState, rows: Vec<Row>) {
    let mut state = state.borrow_mut();
    state.rows = rows;
    state.rows_changed = true;
    state.select_first = true;
}

pub(super) fn select_created(
    state: &SharedState,
    operation: &crate::store::environments::Operation,
) {
    select_item(
        state,
        operation.action == "create_instance",
        &operation.name,
    );
}

pub(super) fn select_instance(state: &SharedState, name: &str) {
    select_item(state, true, name);
}

pub(super) fn select_opencode_pane(state: &SharedState, pane: crate::store::opencode::Pane) {
    let mut state = state.borrow_mut();
    state.select_pane = Some((pane, Instant::now() + Duration::from_secs(30)));
    state.select_created = None;
    state.rows_changed = true;
}

fn select_item(state: &SharedState, instance: bool, name: &str) {
    let mut state = state.borrow_mut();
    state.select_created = Some((instance, name.into()));
    state.select_pane = None;
    state.rows_changed = true;
}

#[cfg(test)]
pub(super) fn set_highlighted(state: &SharedState, highlighted: Option<String>) {
    state.borrow_mut().highlighted = highlighted;
}

#[cfg(test)]
pub(super) fn set_searching(state: &SharedState, searching: bool) {
    state.borrow_mut().searching = searching;
}

pub(super) struct Instances {
    tree: DataView<Row, String>,
    state: SharedState,
    spinner: Rc<RefCell<Spinner>>,
    stripe_query: String,
    session_timers: HashMap<String, SessionTimer>,
    timer_display_phase: Duration,
    completion: Rc<RefCell<completion::Markers>>,
    scroll_preset: tuicore::ScrollPreset,
    initial_expansion_pending: bool,
}

struct SessionTimer {
    sample: (u64, u64),
    elapsed: Duration,
    displayed_seconds: u64,
}

impl Instances {
    pub(super) fn new(state: SharedState) -> Self {
        let (rows, agent_view) = {
            let state = state.borrow();
            (state.rows.clone(), state.attached_sessions_only)
        };
        let initial_expansion_pending = !rows.iter().any(|row| row.instance.is_some());
        let expanded = Self::fully_expanded_ids(&rows, agent_view);
        let spinner = Rc::new(RefCell::new(Spinner::new()));
        let cell_spinner = Rc::clone(&spinner);
        let memory_spinner = Rc::clone(&spinner);
        let cpu_spinner = Rc::clone(&spinner);
        let cpu_state = Rc::clone(&state);
        let completion = Rc::new(RefCell::new(completion::Markers::new(
            state.borrow().completion_fade,
        )));
        let row_completion = Rc::clone(&completion);
        let row_pulse = Rc::clone(&completion);
        let tree = DataView::new(rows, |row: &Row| row.id.clone())
            .focus_id(TREE_FOCUS)
            .columns(vec![
                Column::multiline(
                    "name",
                    "Templates / instances",
                    Constraint::Fill(1),
                    move |row: &Row, context| {
                        row.text(cell_spinner.borrow().glyph(), context.available_width)
                    },
                )
                .search_key(Row::search_text)
                .constrained(),
                Column::multiline(
                    "memory",
                    "Memory",
                    Constraint::Min(0),
                    move |row: &Row, _| {
                        let mut memory =
                            row.memory_text_with_spinner(memory_spinner.borrow().glyph());
                        memory.spans.insert(0, Span::raw(" "));
                        memory.alignment = Some(Alignment::Right);
                        memory
                    },
                )
                .search_key(|row| row.resource_text().to_string())
                .fit_content(),
                Column::multiline("cpu", "CPU", Constraint::Min(0), move |row: &Row, _| {
                    let mut cpu = row.cpu_text_with_spinner(cpu_spinner.borrow().glyph());
                    let padding = cpu_state
                        .borrow()
                        .cpu_column_min_width
                        .saturating_sub(cpu.width());
                    if padding > 0 {
                        cpu.spans.insert(0, Span::raw(" ".repeat(padding)));
                    }
                    cpu.spans.push(Span::raw(" "));
                    cpu.alignment = Some(Alignment::Right);
                    cpu
                })
                .search_key(|row| row.resource_text().to_string())
                .fit_content(),
            ])
            .headers(false)
            .action_bar(true)
            .filter_controls(false)
            .selection_mode(SelectionMode::Single)
            .tree(TreeAdapter::parent_id(|row: &Row| row.parent.clone()))
            .row_height_by(Row::height)
            .hotkey("shift+h")
            .row_style_by(|row| {
                row.alternate_background
                    .then(|| Style::default().bg(tuicore::theme().surface_bg()))
            })
            .row_style_transform(move |row, style| row_pulse.borrow().style(row, style))
            .left_gutter_overlay_by(move |row, style| row_completion.borrow().marker(row, style))
            .expanded(expanded);
        let mut instances = Self {
            tree,
            state,
            spinner,
            stripe_query: String::new(),
            session_timers: HashMap::new(),
            timer_display_phase: Duration::ZERO,
            completion,
            scroll_preset: tuicore::preset().scroll(),
            initial_expansion_pending,
        };
        instances.tick_session_timers(Duration::ZERO);
        instances.record_highlighted();
        instances
    }

    fn sync_rows(&mut self) -> bool {
        let reset = std::mem::take(&mut self.state.borrow_mut().reset_view);
        if reset {
            self.reset_search_and_selection();
        }
        let changed = self.sync_updated_rows();
        if std::mem::take(&mut self.state.borrow_mut().expand_view) {
            self.expand_overview();
            self.after_event();
        }
        changed || reset
    }

    fn sync_updated_rows(&mut self) -> bool {
        let (mut rows, select_first, agent_view, mode_changed) = {
            let mut state = self.state.borrow_mut();
            if !state.rows_changed {
                return false;
            }
            state.rows_changed = false;
            let mode_changed = std::mem::take(&mut state.mode_changed);
            if mode_changed {
                self.tree.set_empty_state(tuicore::SeasonalEmptyState::new(
                    if state.attached_sessions_only {
                        "No attached OpenCode sessions"
                    } else {
                        "No results found."
                    },
                ));
            }
            let select_first = std::mem::take(&mut state.select_first);
            (
                state.rows.clone(),
                select_first,
                state.attached_sessions_only,
                mode_changed,
            )
        };
        let select_pane = {
            let mut state = self.state.borrow_mut();
            if state
                .select_pane
                .as_ref()
                .is_some_and(|(_, deadline)| Instant::now() >= *deadline)
            {
                state.select_pane = None;
            }
            state
                .select_pane
                .as_ref()
                .and_then(|(pane, _)| selection::pane_row(&rows, pane))
                .map(|row| (row.id.clone(), row.parent.clone()))
        };
        let select_created = select_pane.or_else(|| {
            self.state
                .borrow()
                .select_created
                .as_ref()
                .and_then(|(instance, name)| {
                    rows.iter()
                        .find(|row| {
                            if *instance {
                                row.instance.as_ref() == Some(name)
                            } else {
                                row.parent.is_none() && row.template == *name
                            }
                        })
                        .map(|row| (row.id.clone(), row.parent.clone()))
                })
        });
        let query = if select_created.is_some() {
            String::new()
        } else {
            self.tree.transform_state().search.clone()
        };
        rows::assign_alternating_backgrounds(&mut rows, &query);
        let templates_with_new_children = rows
            .iter()
            .filter(|row| {
                row.parent.is_none()
                    && (agent_view || !Self::external_workspace(row))
                    && !Self::external_workspaces_group(row)
                    && !self
                        .tree
                        .rows()
                        .iter()
                        .any(|current| current.parent.as_ref() == Some(&row.id))
            })
            .map(|row| row.id.clone())
            .collect::<Vec<_>>();
        let instances_with_new_children = rows
            .iter()
            .filter(|row| {
                row.instance.is_some()
                    && !self
                        .tree
                        .rows()
                        .iter()
                        .any(|current| current.parent.as_ref() == Some(&row.id))
            })
            .map(|row| row.id.clone())
            .collect::<Vec<_>>();
        let deleted_instance_replacement = self.tree.highlighted_id().and_then(|highlighted| {
            let current = self.tree.rows().iter().find(|row| row.id == highlighted)?;
            if current.instance.is_none() || rows.iter().any(|row| row.id == highlighted) {
                return None;
            }
            let siblings = self
                .tree
                .rows()
                .iter()
                .filter(|row| row.parent == current.parent && row.instance.is_some())
                .collect::<Vec<_>>();
            let position = siblings.iter().position(|row| row.id == highlighted)?;
            siblings[position + 1..]
                .iter()
                .chain(siblings[..position].iter().rev())
                .find(|candidate| rows.iter().any(|row| row.id == candidate.id))
                .map(|row| row.id.clone())
                .or_else(|| {
                    current
                        .parent
                        .as_ref()
                        .filter(|parent| rows.iter().any(|row| &row.id == *parent))
                        .cloned()
                })
        });
        let retained_highlight = self
            .tree
            .highlighted_id()
            .filter(|_| self.state.borrow().center_highlighted)
            .filter(|highlighted| rows.iter().any(|row| &row.id == highlighted))
            .or_else(|| {
                selection::pane_replacement(
                    self.tree.rows(),
                    &rows,
                    self.tree.highlighted_id().as_deref(),
                )
            })
            .filter(|highlighted| {
                !(mode_changed
                    && !agent_view
                    && Self::external_workspace_descendant(&rows, highlighted))
            });
        self.completion.borrow_mut().observe(
            if mode_changed {
                &rows
            } else {
                self.tree.rows()
            },
            &rows,
            select_first,
        );
        self.tree.set_rows(rows);
        self.tick_session_timers(Duration::ZERO);
        self.stripe_query = query;
        if mode_changed && !agent_view {
            self.tree.collapse_all();
            for id in Self::overview_expanded_ids(self.tree.rows()) {
                self.tree.expand(&id);
            }
        }
        for id in templates_with_new_children {
            self.tree.expand(&id);
        }
        for id in instances_with_new_children {
            self.tree.expand(&id);
        }
        if self.initial_expansion_pending
            && self.tree.rows().iter().any(|row| row.instance.is_some())
        {
            self.expand_overview();
            self.initial_expansion_pending = false;
        }
        if let Some(id) = retained_highlight {
            let mut current = id.clone();
            while let Some(parent) = self
                .tree
                .rows()
                .iter()
                .find(|row| row.id == current)
                .and_then(|row| row.parent.clone())
            {
                self.tree.expand(&parent);
                current = parent;
            }
            self.tree.highlight_id(&id);
        }
        if let Some(id) = deleted_instance_replacement {
            self.tree.highlight_id(&id);
        }
        if select_first {
            self.highlight_first_template();
            self.tree.reveal_highlighted();
        }
        if let Some((id, parent)) = select_created {
            self.tree.set_search_query("");
            self.stripe_query.clear();
            let mut ancestor = parent;
            while let Some(parent) = ancestor {
                self.tree.expand(&parent);
                ancestor = self
                    .tree
                    .rows()
                    .iter()
                    .find(|row| row.id == parent)
                    .and_then(|row| row.parent.clone());
            }
            self.tree.highlight_id(&id);
            self.tree.reveal_highlighted();
            self.state.borrow_mut().select_created = None;
            self.state.borrow_mut().select_pane = None;
        }
        self.record_highlighted();
        true
    }

    fn tick_session_timers(&mut self, dt: Duration) -> bool {
        self.timer_display_phase = self.timer_display_phase.saturating_add(dt);
        let display_tick = self.timer_display_phase >= Duration::from_secs(1);
        self.timer_display_phase =
            Duration::from_nanos(u64::from(self.timer_display_phase.subsec_nanos()));
        self.session_timers.retain(|id, _| {
            self.tree
                .rows()
                .iter()
                .any(|row| &row.id == id && row.activity_timer.is_some())
        });
        let mut updates = Vec::new();
        for row in self.tree.rows() {
            let Some(sample) = row.activity_timer else {
                continue;
            };
            let timer = self
                .session_timers
                .entry(row.id.clone())
                .or_insert(SessionTimer {
                    sample,
                    elapsed: Duration::from_millis(sample.1),
                    displayed_seconds: sample.1 / 1_000,
                });
            if sample.0 != timer.sample.0 {
                timer.elapsed = Duration::from_millis(sample.1);
                timer.displayed_seconds = timer.elapsed.as_secs();
            } else if sample != timer.sample {
                timer.elapsed = timer.elapsed.max(Duration::from_millis(sample.1));
            } else {
                timer.elapsed = timer.elapsed.saturating_add(dt);
            }
            timer.sample = sample;
            if display_tick {
                timer.displayed_seconds = timer.elapsed.as_secs();
            }
            let timer_detail =
                rows::compact_duration(timer.displayed_seconds.saturating_mul(1_000));
            let detail = row
                .status_detail
                .as_deref()
                .and_then(|detail| detail.split_once(" · "))
                .map_or_else(
                    || timer_detail.clone(),
                    |(_, suffix)| format!("{timer_detail} · {suffix}"),
                );
            if row.status_detail.as_deref() != Some(&detail) {
                updates.push((row.id.clone(), detail));
            }
        }
        if updates.is_empty() {
            return false;
        }
        let mut rows = self.tree.rows().to_vec();
        for (id, detail) in updates {
            if let Some(row) = rows.iter_mut().find(|row| row.id == id) {
                row.status_detail = Some(detail);
            }
        }
        self.tree.set_rows(rows);
        true
    }

    fn record_highlighted(&mut self) {
        let mut state = self.state.borrow_mut();
        state.highlighted = self.tree.highlighted_id();
        state.searching = self.tree.is_searching();
    }

    fn after_event(&mut self) {
        let _ = self.tree.take_events();
        let query = self.tree.transform_state().search.clone();
        if query != self.stripe_query {
            let mut rows = self.tree.rows().to_vec();
            rows::assign_alternating_backgrounds(&mut rows, &query);
            self.tree.set_rows(rows);
            self.stripe_query = query;
        }
        self.record_highlighted();
    }

    fn overview_expanded_ids(rows: &[Row]) -> Vec<String> {
        rows.iter()
            .filter(|row| {
                (row.parent.is_none()
                    && !Self::external_workspace(row)
                    && !Self::external_workspaces_group(row))
                    || row.instance.is_some()
            })
            .map(|row| row.id.clone())
            .collect()
    }

    fn external_workspace(row: &Row) -> bool {
        row.id.starts_with("opencode-workspace:")
    }

    fn external_workspaces_group(row: &Row) -> bool {
        row.id == OPENCODE_WORKSPACES_ID
    }

    fn external_workspace_descendant(rows: &[Row], id: &str) -> bool {
        let mut current = rows.iter().find(|row| row.id == id);
        while let Some(parent) = current.and_then(|row| row.parent.as_deref()) {
            if parent == OPENCODE_WORKSPACES_ID {
                return true;
            }
            current = rows.iter().find(|row| row.id == parent);
        }
        false
    }

    fn fully_expanded_ids(rows: &[Row], include_external: bool) -> Vec<String> {
        let parent_ids = rows
            .iter()
            .filter_map(|row| row.parent.clone())
            .collect::<HashSet<_>>();
        rows.iter()
            .filter(|row| {
                parent_ids.contains(&row.id)
                    && !Self::external_workspaces_group(row)
                    && (include_external || !Self::external_workspace(row))
            })
            .map(|row| row.id.clone())
            .collect()
    }

    fn highlight_first_template(&mut self) {
        let first = self
            .tree
            .rows()
            .iter()
            .find(|row| row.is_template())
            .map(|row| row.id.clone());
        if let Some(id) = first {
            self.tree.highlight_id(&id);
        }
    }

    fn reset_search_and_selection(&mut self) {
        self.tree.clear_search();
        self.tree.clear_filters();
        if self.tree.is_searching() {
            self.tree.on_key(tuicore::Key::Enter, Rect::default());
        }
        if let Some(id) = self.tree.rows().first().map(|row| row.id.clone()) {
            self.tree.highlight_id(&id);
        }
        self.tree.reveal_highlighted();
        self.after_event();
    }

    fn focus_expanded_overview(&mut self, ctx: &mut EventCtx<Msg>) {
        self.reset_search_and_selection();
        self.expand_overview();
        self.tree.reveal_highlighted();
        self.after_event();
        ctx.focus(super::initial_focus());
    }

    fn expand_overview(&mut self) {
        let highlighted = self.tree.highlighted_id();
        self.tree.collapse_all();
        let expanded_ids =
            Self::fully_expanded_ids(self.tree.rows(), self.state.borrow().attached_sessions_only);
        for id in expanded_ids {
            self.tree.expand(&id);
        }
        if let Some(id) = highlighted {
            self.tree.highlight_id(&id);
        }
    }

    fn toggles_overview_expansion(&self, event: &TuiEvent) -> bool {
        if self.tree.is_searching() {
            return false;
        }
        let TuiEvent::Key(key) = event else {
            return false;
        };
        tuicore::keybindings()
            .data_view()
            .toggle_all_expansion_matches(*key)
    }

    fn toggle_overview_expansion(&mut self, ctx: &mut EventCtx<Msg>) -> EventOutcome {
        let parent_ids = self
            .tree
            .rows()
            .iter()
            .filter_map(|row| row.parent.clone())
            .collect::<HashSet<_>>();
        let mut expandable_ids = Self::overview_expanded_ids(self.tree.rows())
            .into_iter()
            .filter(|id| parent_ids.contains(id))
            .collect::<Vec<_>>();
        if self.state.borrow().attached_sessions_only {
            expandable_ids.extend(
                self.tree
                    .rows()
                    .iter()
                    .filter(|row| parent_ids.contains(&row.id) && Self::external_workspace(row))
                    .map(|row| row.id.clone()),
            );
        }
        let mut all_expanded = true;
        for id in &expandable_ids {
            all_expanded &= !self.tree.expand(id).changed;
        }
        self.tree.collapse_all();
        if !all_expanded {
            for id in expandable_ids {
                self.tree.expand(&id);
            }
        }
        self.after_event();
        ctx.request_layout();
        ctx.request_redraw();
        ctx.stop_propagation();
        EventOutcome::Handled
    }

    #[cfg(test)]
    pub(super) fn search_query(&self) -> &str {
        &self.tree.transform_state().search
    }

    #[cfg(test)]
    pub(super) fn expand_for_tests(&mut self, id: &str) {
        self.tree.expand(&id.to_owned());
        self.after_event();
    }

    #[cfg(test)]
    pub(super) fn highlight_for_tests(&mut self, id: &str) {
        self.tree.highlight_id(&id.to_owned());
        self.after_event();
    }
}

impl TuiNode<Msg> for Instances {
    fn measure(&self, proposal: LayoutProposal) -> LayoutSizeHint {
        <DataView<Row, String> as TuiNode<Msg>>::measure(&self.tree, proposal)
    }

    fn layout(&mut self, area: Rect, ctx: &mut LayoutCtx) -> LayoutResult {
        self.sync_rows();
        let result = <DataView<Row, String> as TuiNode<Msg>>::layout(&mut self.tree, area, ctx);
        let content_height = self
            .measure(LayoutProposal::at_most(area.width, u16::MAX))
            .preferred
            .height;
        let scrollbar_visible = match self.scroll_preset.vertical_scrollbar {
            tuicore::ScrollbarVisibility::Auto => content_height > area.height,
            tuicore::ScrollbarVisibility::Always => true,
            tuicore::ScrollbarVisibility::Never => false,
        };
        self.state.borrow_mut().resource_right_gutter = u16::from(
            !area.is_empty()
                && scrollbar_visible
                && self.scroll_preset.gutter == tuicore::ScrollbarGutter::Reserve,
        );
        if std::mem::take(&mut self.state.borrow_mut().center_highlighted) {
            self.tree.reveal_highlighted_centered();
        }
        result
    }

    fn render<'a>(&'a self, frame: &mut Frame, area: Rect, ctx: &mut tuicore::RenderCtx<'a>) {
        <DataView<Row, String> as TuiNode<Msg>>::render(&self.tree, frame, area, ctx);
    }

    fn event(&mut self, event: &TuiEvent, ctx: &mut EventCtx<Msg>) -> EventOutcome {
        self.sync_rows();
        if let Some(outcome) = self.navigate_completion(event, ctx) {
            return outcome;
        }
        if matches!(event, TuiEvent::Hotkey(tuicore::HotkeyEvent::Commit(sequence)) if sequence == "shift+h")
        {
            self.focus_expanded_overview(ctx);
            ctx.stop_propagation();
            return EventOutcome::Handled;
        }
        if self.toggles_overview_expansion(event) {
            return self.toggle_overview_expansion(ctx);
        }
        let outcome = self.tree.event(event, ctx);
        self.after_event();
        outcome
    }

    fn dispatch_event(
        &mut self,
        route: &EventRoute,
        event: &TuiEvent,
        ctx: &mut EventCtx<Msg>,
    ) -> EventOutcome {
        self.sync_rows();
        if let Some(outcome) = self.navigate_completion(event, ctx) {
            return outcome;
        }
        if matches!(event, TuiEvent::Hotkey(tuicore::HotkeyEvent::Commit(sequence)) if sequence == "shift+h")
        {
            self.focus_expanded_overview(ctx);
            ctx.stop_propagation();
            return EventOutcome::Handled;
        }
        if self.toggles_overview_expansion(event) {
            return self.toggle_overview_expansion(ctx);
        }
        let outcome = self.tree.dispatch_event(route, event, ctx);
        self.after_event();
        outcome
    }

    fn tick(&mut self, dt: Duration, settings: AnimationSettings) -> TickResult {
        let changed = self.sync_rows();
        let timer_changed = self.tick_session_timers(dt);
        let mut result =
            <DataView<Row, String> as TuiNode<Msg>>::tick(&mut self.tree, dt, settings);
        result = result.merge(self.completion.borrow_mut().tick(
            dt,
            settings,
            self.state.borrow().completion_fade,
        ));
        if self.state.borrow().rows.iter().any(|row| {
            row.loading
                || row.secondary_loading
                || row.metrics.memory_waiting
                || row.metrics.cpu_waiting
        }) {
            result = result.merge(Animated::tick(
                &mut *self.spinner.borrow_mut(),
                dt,
                settings,
            ));
        }
        if !self.session_timers.is_empty() {
            result.active = true;
        }
        if changed || timer_changed {
            result.merge(TickResult::CHANGED)
        } else {
            result
        }
    }

    fn take_pending_focus_request(&mut self) -> Option<tuicore::FocusRequest> {
        <DataView<Row, String> as TuiNode<Msg>>::take_pending_focus_request(&mut self.tree)
    }

    fn take_pending_clipboard_request(&mut self) -> Option<String> {
        <DataView<Row, String> as TuiNode<Msg>>::take_pending_clipboard_request(&mut self.tree)
    }

    fn focus(&mut self, target: Option<&FocusId>, focused: bool, ctx: &mut FocusCtx<Msg>) {
        self.tree.focus(target, focused, ctx);
    }

    fn dispatch_focus(&mut self, target: &FocusTarget, focused: bool, ctx: &mut FocusCtx<Msg>) {
        self.tree.dispatch_focus(target, focused, ctx);
    }

    fn focus_reveal_area(&self, target: &FocusTarget) -> Option<Rect> {
        <DataView<Row, String> as TuiNode<Msg>>::focus_reveal_area(&self.tree, target)
    }

    fn focus_reveal_centered(&self, target: &FocusTarget) -> bool {
        <DataView<Row, String> as TuiNode<Msg>>::focus_reveal_centered(&self.tree, target)
    }

    fn init(&mut self, ctx: &mut LifecycleCtx<Msg>) {
        self.tree.init(ctx);
    }

    fn mount(&mut self, ctx: &mut LifecycleCtx<Msg>) {
        self.tree.mount(ctx);
    }

    fn unmount(&mut self, ctx: &mut LifecycleCtx<Msg>) {
        self.tree.unmount(ctx);
    }

    fn destroy(&mut self, ctx: &mut LifecycleCtx<Msg>) {
        self.tree.destroy(ctx);
    }
}
