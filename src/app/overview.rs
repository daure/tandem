use std::time::Duration;

use ratatui::{Frame, layout::Rect};
use tuicore::{
    AnimationSettings, ChildSlot, EventCtx, EventOutcome, EventRoute, Flex, FlexItem, FocusCtx,
    FocusId, FocusTarget, KeySpec, LayoutCtx, LayoutProposal, LayoutResult, LayoutSizeHint,
    LifecycleCtx, RenderCtx, TickResult, TuiEvent, TuiNode,
};

use super::{
    Msg,
    instances::{Instances, SharedState},
    toolbar,
};

pub(super) struct Pages {
    pages: [ChildSlot<Flex<Msg>, Msg>; 2],
    states: [SharedState; 2],
    active: usize,
}

impl Pages {
    pub(super) fn new(
        keys: [KeySpec; 10],
        toolbar: toolbar::SharedState,
        states: [SharedState; 2],
        sessions: bool,
    ) -> Self {
        let pages = std::array::from_fn(|index| {
            ChildSlot::new(
                if index == 0 {
                    "inventory-page"
                } else {
                    "sessions-page"
                },
                Flex::column()
                    .child(
                        "template-actions",
                        toolbar::Toolbar::new(keys[2], keys[4], keys[8], keys[9], toolbar.clone())
                            .align_resources_with(states[index].clone()),
                        FlexItem::fit_content(),
                    )
                    .child(
                        "instances",
                        Instances::new(states[index].clone()),
                        FlexItem::fill(1),
                    ),
            )
        });
        Self {
            pages,
            states,
            active: usize::from(sessions),
        }
    }

    pub(super) fn select(&mut self, sessions: bool) -> SharedState {
        self.active = usize::from(sessions);
        self.states[self.active].clone()
    }

    pub(super) fn states(&self) -> [SharedState; 2] {
        self.states.clone()
    }

    pub(super) fn retain_control_focus(&self, route: &EventRoute, ctx: &mut EventCtx<Msg>) {
        let path = tuicore::TreePath::from_keys(route.path.keys().iter().map(|key| {
            if self.pages.iter().any(|page| page.key() == key) {
                self.pages[self.active].key().clone()
            } else {
                key.clone()
            }
        }));
        if path != route.path {
            ctx.focus(tuicore::FocusRequest::Path(path));
        }
    }

    pub(super) fn focus_overview(&mut self, event: &TuiEvent, ctx: &mut EventCtx<Msg>) {
        let route = EventRoute::new(tuicore::TreePath::from_keys(["instances".into()]));
        self.pages[self.active]
            .child_mut()
            .dispatch_event(&route, event, ctx);
    }
}

impl TuiNode<Msg> for Pages {
    fn measure(&self, proposal: LayoutProposal) -> LayoutSizeHint {
        self.pages[self.active].measure(proposal)
    }

    fn layout(&mut self, area: Rect, ctx: &mut LayoutCtx) -> LayoutResult {
        self.pages[self.active].layout(area, ctx)
    }

    fn render<'a>(&'a self, frame: &mut Frame, area: Rect, ctx: &mut RenderCtx<'a>) {
        self.pages[self.active].render(frame, area, ctx);
    }

    fn event(&mut self, event: &TuiEvent, ctx: &mut EventCtx<Msg>) -> EventOutcome {
        self.pages[self.active].child_mut().event(event, ctx)
    }

    fn dispatch_event(
        &mut self,
        route: &EventRoute,
        event: &TuiEvent,
        ctx: &mut EventCtx<Msg>,
    ) -> EventOutcome {
        self.pages[self.active].dispatch_event(route, event, ctx)
    }

    fn tick(&mut self, dt: Duration, settings: AnimationSettings) -> TickResult {
        self.pages
            .iter_mut()
            .fold(TickResult::IDLE, |result, page| {
                result.merge(page.tick(dt, settings))
            })
    }

    fn take_pending_focus_request(&mut self) -> Option<tuicore::FocusRequest> {
        self.pages[self.active]
            .child_mut()
            .take_pending_focus_request()
    }

    fn take_pending_clipboard_request(&mut self) -> Option<String> {
        self.pages[self.active]
            .child_mut()
            .take_pending_clipboard_request()
    }

    fn focus(&mut self, target: Option<&FocusId>, focused: bool, ctx: &mut FocusCtx<Msg>) {
        self.pages[self.active].focus(target, focused, ctx);
    }

    fn dispatch_focus(&mut self, target: &FocusTarget, focused: bool, ctx: &mut FocusCtx<Msg>) {
        for page in &mut self.pages {
            page.dispatch_focus(target, focused, ctx);
        }
    }

    fn focus_reveal_area(&self, target: &FocusTarget) -> Option<Rect> {
        self.pages[self.active].focus_reveal_area(target)
    }

    fn focus_reveal_centered(&self, target: &FocusTarget) -> bool {
        self.pages[self.active].focus_reveal_centered(target)
    }

    fn init(&mut self, ctx: &mut LifecycleCtx<Msg>) {
        for page in &mut self.pages {
            page.init(ctx);
        }
    }

    fn mount(&mut self, ctx: &mut LifecycleCtx<Msg>) {
        for page in &mut self.pages {
            page.mount(ctx);
        }
    }

    fn unmount(&mut self, ctx: &mut LifecycleCtx<Msg>) {
        for page in &mut self.pages {
            page.unmount(ctx);
        }
    }

    fn destroy(&mut self, ctx: &mut LifecycleCtx<Msg>) {
        for page in &mut self.pages {
            page.destroy(ctx);
        }
    }
}
