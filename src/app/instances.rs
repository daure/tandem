use std::{cell::RefCell, rc::Rc, time::Duration};

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

#[derive(Default)]
pub(super) struct State {
    rows: Vec<Row>,
    highlighted: Option<String>,
    searching: bool,
    rows_changed: bool,
    select_created: Option<(bool, String)>,
}

pub(super) type SharedState = Rc<RefCell<State>>;

pub(super) fn state(rows: Vec<Row>) -> SharedState {
    Rc::new(RefCell::new(State {
        rows,
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

pub(super) fn is_searching(state: &SharedState) -> bool {
    state.borrow().searching
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

fn select_item(state: &SharedState, instance: bool, name: &str) {
    let mut state = state.borrow_mut();
    state.select_created = Some((instance, name.into()));
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
}

impl Instances {
    pub(super) fn new(state: SharedState) -> Self {
        let rows = state.borrow().rows.clone();
        let expanded = rows
            .iter()
            .filter(|row| row.parent.is_none())
            .map(|row| row.id.clone())
            .collect::<Vec<_>>();
        let spinner = Rc::new(RefCell::new(Spinner::new()));
        let cell_spinner = Rc::clone(&spinner);
        let tree = DataView::new(rows, |row: &Row| row.id.clone())
            .focus_id(TREE_FOCUS)
            .columns(vec![
                Column::multiline(
                    "name",
                    "Templates / instances",
                    Constraint::Fill(1),
                    move |row: &Row, _| row.text(cell_spinner.borrow().glyph()),
                )
                .search_key(Row::search_text)
                .constrained(),
                Column::multiline(
                    "resources",
                    "Memory / CPU",
                    Constraint::Min(0),
                    |row: &Row, _| {
                        let mut resources = row.resource_text();
                        for line in &mut resources.lines {
                            line.spans.push(Span::raw(" "));
                            line.alignment = Some(Alignment::Right);
                        }
                        resources
                    },
                )
                .search_key(|row| row.resource_text().to_string())
                .fit_content(),
            ])
            .headers(false)
            .action_bar(true)
            .filter_controls(false)
            .selection_mode(SelectionMode::Single)
            .tree(TreeAdapter::parent_id(|row: &Row| row.parent.clone()))
            .row_height_by(Row::height)
            .row_style_by(|row| {
                row.alternate_background
                    .then(|| Style::default().bg(tuicore::theme().surface_bg()))
            })
            .expanded(expanded);
        let mut instances = Self {
            tree,
            state,
            spinner,
            stripe_query: String::new(),
        };
        instances.record_highlighted();
        instances
    }

    fn sync_rows(&mut self) -> bool {
        let mut rows = {
            let mut state = self.state.borrow_mut();
            if !state.rows_changed {
                return false;
            }
            state.rows_changed = false;
            state.rows.clone()
        };
        let select_created =
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
                    && !self
                        .tree
                        .rows()
                        .iter()
                        .any(|current| current.parent.as_ref() == Some(&row.id))
            })
            .map(|row| row.id.clone())
            .collect::<Vec<_>>();
        self.tree.set_rows(rows);
        self.stripe_query = query;
        for id in templates_with_new_children {
            self.tree.expand(&id);
        }
        if let Some((id, parent)) = select_created {
            self.tree.set_search_query("");
            self.stripe_query.clear();
            if let Some(parent) = parent {
                self.tree.expand(&parent);
            }
            self.tree.highlight_id(&id);
            self.tree.reveal_highlighted();
            self.state.borrow_mut().select_created = None;
        }
        self.record_highlighted();
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
}

impl TuiNode<Msg> for Instances {
    fn measure(&self, proposal: LayoutProposal) -> LayoutSizeHint {
        <DataView<Row, String> as TuiNode<Msg>>::measure(&self.tree, proposal)
    }

    fn layout(&mut self, area: Rect, ctx: &mut LayoutCtx) -> LayoutResult {
        self.sync_rows();
        <DataView<Row, String> as TuiNode<Msg>>::layout(&mut self.tree, area, ctx)
    }

    fn render<'a>(&'a self, frame: &mut Frame, area: Rect, ctx: &mut tuicore::RenderCtx<'a>) {
        <DataView<Row, String> as TuiNode<Msg>>::render(&self.tree, frame, area, ctx);
    }

    fn event(&mut self, event: &TuiEvent, ctx: &mut EventCtx<Msg>) -> EventOutcome {
        self.sync_rows();
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
        let outcome = self.tree.dispatch_event(route, event, ctx);
        self.after_event();
        outcome
    }

    fn tick(&mut self, dt: Duration, settings: AnimationSettings) -> TickResult {
        let changed = self.sync_rows();
        let mut result =
            <DataView<Row, String> as TuiNode<Msg>>::tick(&mut self.tree, dt, settings);
        if self.state.borrow().rows.iter().any(|row| row.loading) {
            result = result.merge(Animated::tick(
                &mut *self.spinner.borrow_mut(),
                dt,
                settings,
            ));
        }
        if changed {
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
