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
    pages: [ChildSlot<Box<dyn TuiNode<Msg>>, Msg>; 5],
    states: [SharedState; 2],
    active: usize,
    events: super::events::SharedState,
    providers: super::providers::SharedState,
    event_filter: super::events::FilterState,
    rules: super::rules::SharedState,
    event_focus: super::events::FocusState,
    rule_focus: super::rules::FocusState,
    acceptance_context: super::acceptances::SharedContext,
}

impl Pages {
    pub(super) fn new(
        keys: [KeySpec; 10],
        toolbar: toolbar::SharedState,
        states: [SharedState; 2],
        sessions: bool,
    ) -> Self {
        let events = std::rc::Rc::new(std::cell::RefCell::new(
            crate::store::events::Snapshot::default(),
        ));
        let providers =
            std::rc::Rc::new(std::cell::RefCell::new(super::providers::State::default()));
        let event_filter = std::rc::Rc::new(std::cell::RefCell::new(None));
        let rules = std::rc::Rc::new(std::cell::RefCell::new(
            crate::store::rules::Snapshot::default(),
        ));
        let event_focus = std::rc::Rc::new(std::cell::RefCell::new(
            super::events::FocusRequest::default(),
        ));
        let rule_focus = std::rc::Rc::new(std::cell::RefCell::new(None));
        let acceptance_context = std::rc::Rc::new(std::cell::RefCell::new(
            super::acceptances::Context::default(),
        ));
        let pages = std::array::from_fn(|index| {
            let (key, page): (&str, Box<dyn TuiNode<Msg>>) = if index == 4 {
                (
                    "rules-page",
                    Box::new(
                        super::rules::Rules::new(rules.clone(), keys)
                            .with_focus_request(rule_focus.clone()),
                    ),
                )
            } else if index == 3 {
                (
                    "providers-page",
                    Box::new(super::providers::Providers::new(
                        providers.clone(),
                        events.clone(),
                    )),
                )
            } else if index == 2 {
                (
                    "events-page",
                    Box::new(super::events::Events::new(
                        events.clone(),
                        event_filter.clone(),
                        providers.clone(),
                        event_focus.clone(),
                        toolbar.clone(),
                        rules.clone(),
                        acceptance_context.clone(),
                    )),
                )
            } else {
                (
                    if index == 0 {
                        "inventory-page"
                    } else {
                        "sessions-page"
                    },
                    Box::new(
                        Flex::column()
                            .child(
                                "template-actions",
                                toolbar::Toolbar::new(
                                    keys[4],
                                    keys[8],
                                    keys[9],
                                    toolbar.clone(),
                                    index == 1,
                                )
                                .align_resources_with(states[index].clone()),
                                FlexItem::fit_content(),
                            )
                            .child(
                                "instances",
                                Instances::new(states[index].clone()),
                                FlexItem::fill(1),
                            ),
                    ),
                )
            };
            ChildSlot::new(key, page)
        });
        Self {
            pages,
            states,
            active: usize::from(sessions),
            events,
            providers,
            event_filter,
            rules,
            event_focus,
            rule_focus,
            acceptance_context,
        }
    }

    pub(super) fn select(&mut self, sessions: bool) -> SharedState {
        self.active = usize::from(sessions);
        self.states[self.active].clone()
    }

    pub(super) fn states(&self) -> [SharedState; 2] {
        self.states.clone()
    }

    pub(super) fn select_events(&mut self) {
        self.active = 2;
    }

    pub(super) fn select_providers(&mut self) {
        self.active = 3;
    }

    pub(super) fn select_rules(&mut self) {
        self.active = 4;
    }
    pub(super) fn rules_state(&self) -> super::rules::SharedState {
        self.rules.clone()
    }
    pub(super) fn tab_counts(&self) -> (usize, usize) {
        let active_rules = self
            .rules
            .borrow()
            .rules
            .iter()
            .filter(|rule| rule.definition.enabled)
            .count();
        let running_providers = self
            .providers
            .borrow()
            .snapshot
            .providers
            .iter()
            .filter(|provider| provider.status == crate::store::providers::Status::Running)
            .count();
        (active_rules, running_providers)
    }
    pub(super) fn acceptance_context(&self) -> super::acceptances::SharedContext {
        self.acceptance_context.clone()
    }
    pub(super) fn update_acceptance_context(
        &mut self,
        inventory: &crate::store::environments::EnvironmentSnapshot,
        opencode: &crate::store::opencode::Snapshot,
    ) {
        let mut context = self.acceptance_context.borrow_mut();
        if context.inventory != *inventory {
            context.inventory = inventory.clone();
        }
        if context.opencode != *opencode {
            context.opencode = opencode.clone();
        }
    }
    pub(super) fn update_rules(&mut self, snapshot: crate::store::rules::Snapshot) -> bool {
        if *self.rules.borrow() == snapshot {
            return false;
        }
        *self.rules.borrow_mut() = snapshot;
        true
    }
    pub(super) fn focus_event(&mut self, record: crate::store::events::Record) {
        self.event_focus.borrow_mut().record = Some(record);
    }
    pub(super) fn forget_event(&mut self, sequence: Option<i64>) {
        let mut request = self.event_focus.borrow_mut();
        request.deletion = Some(sequence);
        if request
            .record
            .as_ref()
            .is_some_and(|row| sequence.is_none_or(|id| id == row.sequence))
        {
            request.record = None;
        }
    }
    pub(super) fn highlight_rule(&mut self, name: &str) -> bool {
        if !self
            .rules
            .borrow()
            .rules
            .iter()
            .any(|rule| rule.definition.name == name)
        {
            return false;
        }
        *self.rule_focus.borrow_mut() = Some(name.into());
        true
    }

    pub(super) fn filter_event_stream(&mut self, provider: String, stream: String) {
        *self.event_filter.borrow_mut() =
            Some(vec![super::events::StreamKey::new(provider, stream)]);
        self.event_focus.borrow_mut().reset_filters = true;
    }

    pub(super) fn provider_stream_action_unavailable(
        &self,
        name: &str,
        stream: &str,
        action: crate::store::providers::Action,
    ) -> Option<&'static str> {
        let state = self.providers.borrow();
        let Some(provider) = state
            .snapshot
            .providers
            .iter()
            .find(|provider| provider.name == name)
        else {
            return Some("Provider is unavailable; refresh the provider list");
        };
        provider
            .streams
            .iter()
            .find(|row| row.name == stream)
            .map_or(
                Some("Stream is unavailable; refresh the provider list"),
                |stream| stream.action_unavailable(provider, action),
            )
    }

    pub(super) fn highlight_provider(&mut self, identity: &str) -> bool {
        let name = self
            .providers
            .borrow()
            .snapshot
            .providers
            .iter()
            .find(|provider| {
                provider
                    .manifest
                    .as_ref()
                    .is_some_and(|manifest| manifest.name == identity)
            })
            .map(|provider| provider.name.clone());
        if let Some(name) = name {
            self.providers.borrow_mut().requested = Some(name);
            true
        } else {
            false
        }
    }

    pub(super) fn update_providers(&mut self, snapshot: crate::store::providers::Snapshot) -> bool {
        if self.providers.borrow().snapshot == snapshot {
            return false;
        }
        self.providers.borrow_mut().snapshot = snapshot;
        true
    }

    pub(super) fn provider_action_unavailable(
        &self,
        name: &str,
        action: crate::store::providers::Action,
    ) -> Option<&'static str> {
        self.providers
            .borrow()
            .snapshot
            .providers
            .iter()
            .find(|provider| provider.name == name)
            .map_or(
                Some("Provider is unavailable; refresh the provider list"),
                |provider| provider.action_unavailable(action),
            )
    }

    pub(super) fn provider_bulk_targets(
        &self,
        action: crate::store::providers::Action,
    ) -> Vec<String> {
        self.providers.borrow().snapshot.action_targets(action)
    }

    pub(super) fn update_events(&mut self, snapshot: crate::store::events::Snapshot) -> bool {
        if *self.events.borrow() == snapshot {
            return false;
        }
        *self.events.borrow_mut() = snapshot;
        true
    }

    pub(super) fn retain_control_focus(&self, route: &EventRoute, ctx: &mut EventCtx<Msg>) {
        if self.active == 4 {
            ctx.focus(tuicore::FocusRequest::Target(FocusId::new(
                super::rules::FOCUS,
            )));
            return;
        }
        if self.active == 3 {
            ctx.focus(tuicore::FocusRequest::Target(FocusId::new(
                super::providers::FOCUS,
            )));
            return;
        }
        if self.active == 2 {
            ctx.focus(tuicore::FocusRequest::Target(FocusId::new(
                super::events::FOCUS,
            )));
            return;
        }
        if route.path.keys().iter().any(|key| {
            matches!(
                key.as_str(),
                "events-page" | "providers-page" | "rules-page"
            )
        }) {
            ctx.focus(super::initial_focus());
            return;
        }
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

    pub(super) fn reset_overviews(&mut self, event: &TuiEvent, ctx: &mut EventCtx<Msg>) {
        let route = EventRoute::new(tuicore::TreePath::from_keys(["instances".into()]));
        for (index, page) in self.pages.iter_mut().enumerate() {
            if index == self.active {
                continue;
            }
            if index < 2 {
                super::instances::request_view_reset(&self.states[index]);
                page.tick(Duration::ZERO, ctx.animation());
            } else {
                page.child_mut()
                    .dispatch_event(&route, event, &mut EventCtx::new(ctx.animation()));
            }
        }
        self.focus_overview(event, ctx);
    }

    pub(super) fn focus_overview(&mut self, event: &TuiEvent, ctx: &mut EventCtx<Msg>) {
        if self.active >= 2 {
            self.pages[self.active].child_mut().event(event, ctx);
            return;
        }
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
