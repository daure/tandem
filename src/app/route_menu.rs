use std::{cell::RefCell, rc::Rc, time::Duration};

use ratatui::{Frame, layout::Rect, text::Text};
use tuicore::{
    AnimationSettings, Dropdown, DropdownCommitMode, DropdownLabelPosition, DropdownSearchMode,
    DropdownVariant, EventCtx, EventOutcome, EventRoute, FocusCtx, FocusId, FocusTarget, LayoutCtx,
    LayoutProposal, LayoutResult, LayoutSizeHint, LifecycleCtx, RenderCtx, TickResult, TuiEvent,
    TuiNode,
};

use super::Msg;

const MENU_ANCHOR_WIDTH: u16 = 1;
const MENU_HEIGHT: u16 = 12;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(super) struct RouteChoice {
    pub service: String,
    pub url: String,
}

impl RouteChoice {
    fn label(&self) -> String {
        format!("{} - {}", self.service, self.url)
    }
}

pub(super) struct RouteMenu {
    dropdown: Dropdown<RouteChoice, RouteChoice>,
    selected: Rc<RefCell<Option<RouteChoice>>>,
    field_area: Rect,
}

impl RouteMenu {
    pub(super) fn new() -> Self {
        let selected = Rc::new(RefCell::new(None));
        let selection = Rc::clone(&selected);
        let dropdown = Dropdown::single_rich(
            Vec::<RouteChoice>::new(),
            Clone::clone,
            RouteChoice::label,
            |route, _, _| Text::raw(route.label()),
        )
        .variant(DropdownVariant::Filled)
        .label("Open route")
        .label_position(DropdownLabelPosition::Inline)
        .search_mode(DropdownSearchMode::Fuzzy)
        .commit_mode(DropdownCommitMode::Explicit)
        .centered(true)
        .show_field_when_open(false)
        .backdrop_amount(0.55)
        .tab_stop(false)
        .max_popup_height(MENU_HEIGHT)
        .max_popup_width(u16::MAX)
        .on_select(move |routes| *selection.borrow_mut() = routes.first().cloned());
        Self {
            dropdown,
            selected,
            field_area: Rect::default(),
        }
    }

    pub(super) fn open(&mut self, routes: Vec<RouteChoice>, ctx: &mut EventCtx<Msg>) {
        self.selected.borrow_mut().take();
        self.dropdown.clear_selection();
        self.dropdown.set_search_query("");
        // Dropdown preserves its highlight across row replacements; emptying it resets to row one.
        self.dropdown.set_rows(Vec::<RouteChoice>::new());
        self.dropdown.set_rows(routes);
        self.dropdown.open_with_context(ctx);
    }

    pub(super) fn is_open(&self) -> bool {
        self.dropdown.is_open()
    }

    pub(super) fn take_selection(&mut self) -> Option<RouteChoice> {
        self.selected.borrow_mut().take()
    }
}

impl TuiNode<Msg> for RouteMenu {
    fn measure(&self, proposal: LayoutProposal) -> LayoutSizeHint {
        LayoutSizeHint::content(MENU_ANCHOR_WIDTH, MENU_HEIGHT).normalized(proposal)
    }

    fn layout(&mut self, area: Rect, ctx: &mut LayoutCtx) -> LayoutResult {
        let width = MENU_ANCHOR_WIDTH.min(area.width);
        self.field_area = Rect::new(
            area.x.saturating_add(area.width.saturating_sub(width) / 2),
            area.y.saturating_add(area.height / 2),
            width,
            u16::from(!area.is_empty()),
        );
        <Dropdown<_, _> as TuiNode<Msg>>::layout(&mut self.dropdown, self.field_area, ctx);
        LayoutResult::new(area)
    }

    fn render<'a>(&'a self, frame: &mut Frame, _area: Rect, ctx: &mut RenderCtx<'a>) {
        self.dropdown.render(frame, self.field_area, ctx);
    }

    fn event(&mut self, event: &TuiEvent, ctx: &mut EventCtx<Msg>) -> EventOutcome {
        self.dropdown.event(event, ctx)
    }

    fn dispatch_event(
        &mut self,
        route: &EventRoute,
        event: &TuiEvent,
        ctx: &mut EventCtx<Msg>,
    ) -> EventOutcome {
        self.dropdown.dispatch_event(route, event, ctx)
    }

    fn tick(&mut self, dt: Duration, settings: AnimationSettings) -> TickResult {
        <Dropdown<_, _> as TuiNode<Msg>>::tick(&mut self.dropdown, dt, settings)
    }

    fn focus(&mut self, target: Option<&FocusId>, focused: bool, ctx: &mut FocusCtx<Msg>) {
        self.dropdown.focus(target, focused, ctx);
    }

    fn dispatch_focus(&mut self, target: &FocusTarget, focused: bool, ctx: &mut FocusCtx<Msg>) {
        self.dropdown.dispatch_focus(target, focused, ctx);
    }

    fn init(&mut self, ctx: &mut LifecycleCtx<Msg>) {
        self.dropdown.init(ctx);
    }

    fn mount(&mut self, ctx: &mut LifecycleCtx<Msg>) {
        self.dropdown.mount(ctx);
    }

    fn unmount(&mut self, ctx: &mut LifecycleCtx<Msg>) {
        self.dropdown.unmount(ctx);
    }

    fn destroy(&mut self, ctx: &mut LifecycleCtx<Msg>) {
        self.dropdown.destroy(ctx);
    }
}
