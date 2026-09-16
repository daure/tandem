use std::time::Duration;

use ratatui::{
    Frame,
    layout::{Constraint, Rect},
    style::Style,
    text::Text,
};
use tuicore::{
    AnimationSettings, Column, DataView, EventCtx, EventOutcome, EventRoute, FocusCtx, FocusId,
    FocusTarget, LayoutCtx, LayoutProposal, LayoutResult, LayoutSizeHint, LifecycleCtx, RenderCtx,
    TickResult, TuiEvent, TuiNode,
};

use super::{Msg, rows::Tone};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Property {
    pub name: String,
    pub value: String,
    pub tone: Tone,
}

impl Property {
    pub fn new(name: impl Into<String>, value: impl ToString) -> Self {
        Self {
            name: name.into(),
            value: value.to_string(),
            tone: Tone::Normal,
        }
    }

    pub fn tone(mut self, tone: Tone) -> Self {
        self.tone = tone;
        self
    }
}

pub(super) struct Properties {
    table: DataView<Property, String>,
}

impl Properties {
    pub fn new(rows: Vec<Property>) -> Self {
        Self {
            table: DataView::new(rows, |row: &Property| row.name.clone())
                .columns([
                    Column::text("property", "", Constraint::Length(25), |row: &Property| {
                        row.name.clone()
                    }),
                    Column::multiline("value", "", Constraint::Min(20), |row: &Property, _| {
                        Text::styled(row.value.clone(), Style::default().fg(row.tone.color()))
                    })
                    .search_key(|row| row.value.clone()),
                ])
                .headers(false)
                .action_bar(true)
                .filter_controls(false)
                .row_height_by(|row| row.value.lines().count().clamp(1, u16::MAX as usize) as u16),
        }
    }
}

impl TuiNode<Msg> for Properties {
    fn measure(&self, proposal: LayoutProposal) -> LayoutSizeHint {
        <DataView<Property, String> as TuiNode<Msg>>::measure(&self.table, proposal)
    }
    fn layout(&mut self, area: Rect, ctx: &mut LayoutCtx) -> LayoutResult {
        <DataView<Property, String> as TuiNode<Msg>>::layout(&mut self.table, area, ctx)
    }
    fn render<'a>(&'a self, frame: &mut Frame, area: Rect, ctx: &mut RenderCtx<'a>) {
        <DataView<Property, String> as TuiNode<Msg>>::render(&self.table, frame, area, ctx);
    }
    fn event(&mut self, event: &TuiEvent, ctx: &mut EventCtx<Msg>) -> EventOutcome {
        let outcome = self.table.event(event, ctx);
        let _ = self.table.take_events();
        outcome
    }
    fn dispatch_event(
        &mut self,
        route: &EventRoute,
        event: &TuiEvent,
        ctx: &mut EventCtx<Msg>,
    ) -> EventOutcome {
        let outcome = self.table.dispatch_event(route, event, ctx);
        let _ = self.table.take_events();
        outcome
    }
    fn tick(&mut self, dt: Duration, settings: AnimationSettings) -> TickResult {
        <DataView<Property, String> as TuiNode<Msg>>::tick(&mut self.table, dt, settings)
    }
    fn take_pending_focus_request(&mut self) -> Option<tuicore::FocusRequest> {
        <DataView<Property, String> as TuiNode<Msg>>::take_pending_focus_request(&mut self.table)
    }
    fn take_pending_clipboard_request(&mut self) -> Option<String> {
        <DataView<Property, String> as TuiNode<Msg>>::take_pending_clipboard_request(
            &mut self.table,
        )
    }
    fn focus(&mut self, target: Option<&FocusId>, focused: bool, ctx: &mut FocusCtx<Msg>) {
        self.table.focus(target, focused, ctx);
    }
    fn dispatch_focus(&mut self, target: &FocusTarget, focused: bool, ctx: &mut FocusCtx<Msg>) {
        self.table.dispatch_focus(target, focused, ctx);
    }
    fn focus_reveal_area(&self, target: &FocusTarget) -> Option<Rect> {
        <DataView<Property, String> as TuiNode<Msg>>::focus_reveal_area(&self.table, target)
    }
    fn focus_reveal_centered(&self, target: &FocusTarget) -> bool {
        <DataView<Property, String> as TuiNode<Msg>>::focus_reveal_centered(&self.table, target)
    }
    fn init(&mut self, ctx: &mut LifecycleCtx<Msg>) {
        self.table.init(ctx);
    }
    fn mount(&mut self, ctx: &mut LifecycleCtx<Msg>) {
        self.table.mount(ctx);
    }
    fn unmount(&mut self, ctx: &mut LifecycleCtx<Msg>) {
        self.table.unmount(ctx);
    }
    fn destroy(&mut self, ctx: &mut LifecycleCtx<Msg>) {
        self.table.destroy(ctx);
    }
}
