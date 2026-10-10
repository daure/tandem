use std::{
    cell::RefCell,
    ops::{Deref, DerefMut},
    rc::Rc,
    time::Duration,
};

use ratatui::{Frame, layout::Rect};
use tuicore::{
    AnimationSettings, ChildKey, DialogLayer, Dropdown, DropdownCommitMode, DropdownSearchMode,
    DropdownVariant, EventCtx, EventOutcome, EventRoute, Flex, FocusCtx, FocusId, FocusRequest,
    FocusTarget, HotkeyEvent, KeySpec, LayoutCtx, LayoutProposal, LayoutResult, LayoutSizeHint,
    LifecycleCtx, RenderCtx, TickResult, TreePath, TuiEvent, TuiNode,
};

use super::{App, Msg, View};

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub(super) enum Sound {
    Completion,
    EventAcceptance,
    InstancePing,
}

impl Sound {
    const ALL: [Self; 3] = [Self::Completion, Self::EventAcceptance, Self::InstancePing];

    fn label(self) -> String {
        match self {
            Self::Completion => "Session completion",
            Self::EventAcceptance => "Rule acceptance",
            Self::InstancePing => "Instance ping",
        }
        .into()
    }
}

type Picker = Dropdown<Sound, Sound>;
const MENU_SLOT: &str = "notification-sounds";

pub(super) struct Overlay {
    base: View,
    menu: DialogLayer<Flex<Msg>, Picker>,
    selected: Rc<RefCell<Option<Vec<Sound>>>>,
}

impl Overlay {
    pub(super) fn new(view: View) -> Self {
        let selected = Rc::new(RefCell::new(None));
        let selection = selected.clone();
        let picker = Dropdown::multi(Sound::ALL, |sound| *sound, |sound| sound.label())
            .variant(DropdownVariant::Filled)
            .search_mode(DropdownSearchMode::Fuzzy)
            .search_placeholder("Notification sounds…")
            .commit_mode(DropdownCommitMode::Explicit)
            .centered(true)
            .show_field_when_open(false)
            .tab_stop(false)
            .max_popup_height(4)
            .on_select(move |sounds| *selection.borrow_mut() = Some(sounds));
        Self {
            base: view,
            menu: DialogLayer::new(Flex::column(), picker)
                .active(false)
                .fit_content()
                .fit_content_max(32, 1)
                .child_overlays_use_base_bounds(true),
            selected,
        }
    }

    fn open(&mut self, enabled: [bool; 3], ctx: &mut EventCtx<Msg>) {
        self.selected.borrow_mut().take();
        self.menu.layer_mut().set_search_query("");
        self.menu.layer_mut().set_selected(
            Sound::ALL
                .into_iter()
                .zip(enabled)
                .filter_map(|(sound, enabled)| enabled.then_some(sound)),
        );
        self.menu.layer_mut().open_with_context(ctx);
        self.menu.set_active_with_context(true, ctx);
        ctx.focus(FocusRequest::Path(TreePath::from_keys([
            ChildKey::new(MENU_SLOT),
            ChildKey::second(),
        ])));
    }
}

// Base paths stay stable so closing the menu can restore the exact prior focus.
impl Deref for Overlay {
    type Target = View;

    fn deref(&self) -> &View {
        &self.base
    }
}

impl DerefMut for Overlay {
    fn deref_mut(&mut self) -> &mut View {
        &mut self.base
    }
}

impl App {
    pub(super) fn open_sound_menu(&mut self, ctx: &mut EventCtx<Msg>) {
        let enabled = [
            self.completion_sound,
            self.event_acceptance_sound,
            self.service.instance_ping_enabled(),
        ];
        self.view.open(enabled, ctx);
    }

    pub(super) fn sound_menu_event(&mut self, event: &TuiEvent, ctx: &mut EventCtx<Msg>) -> bool {
        let requested = matches!(event, TuiEvent::Key(key) if KeySpec::shifted('n').matches(*key))
            || matches!(event, TuiEvent::Hotkey(HotkeyEvent::Commit(sequence)) if sequence == "shift+n");
        if !self.view.menu.is_active() {
            if !requested {
                return false;
            }
            self.open_sound_menu(ctx);
        } else {
            self.view.menu.layer_mut().event(event, ctx);
            let selected = self.view.selected.borrow_mut().take();
            if let Some(selected) = selected {
                let completion = selected.contains(&Sound::Completion);
                let acceptance = selected.contains(&Sound::EventAcceptance);
                let ping = selected.contains(&Sound::InstancePing);
                if completion != self.completion_sound {
                    self.handle_message(Msg::SetCompletionSound(completion), ctx);
                }
                if acceptance != self.event_acceptance_sound {
                    self.handle_message(Msg::SetEventAcceptanceSound(acceptance), ctx);
                }
                if ping != self.service.instance_ping_enabled() {
                    self.handle_message(Msg::SetInstancePingSound(ping), ctx);
                }
            }
            if !self.view.menu.layer().is_open() {
                self.view.menu.set_active_with_context(false, ctx);
            }
        }
        ctx.stop_propagation();
        ctx.request_redraw();
        true
    }
}

impl TuiNode<Msg> for Overlay {
    fn measure(&self, proposal: LayoutProposal) -> LayoutSizeHint {
        self.base.measure(proposal)
    }
    fn layout(&mut self, area: Rect, ctx: &mut LayoutCtx) -> LayoutResult {
        let focus_disabled = ctx.focus_disabled();
        ctx.set_focus_disabled(focus_disabled || self.menu.is_active());
        self.base.layout(area, ctx);
        ctx.set_focus_disabled(focus_disabled);
        if self.menu.is_active() {
            ctx.push_slot(ChildKey::new(MENU_SLOT), area, |ctx| {
                self.menu.layout(area, ctx)
            });
        }
        LayoutResult::new(area)
    }
    fn render<'a>(&'a self, frame: &mut Frame, area: Rect, ctx: &mut RenderCtx<'a>) {
        self.base.render(frame, area, ctx);
        if self.menu.is_active() {
            self.menu.render(frame, area, ctx);
        }
    }
    fn event(&mut self, event: &TuiEvent, ctx: &mut EventCtx<Msg>) -> EventOutcome {
        if self.menu.is_active() {
            self.menu.event(event, ctx)
        } else {
            self.base.event(event, ctx)
        }
    }
    fn dispatch_event(
        &mut self,
        route: &EventRoute,
        event: &TuiEvent,
        ctx: &mut EventCtx<Msg>,
    ) -> EventOutcome {
        if let Some(path) = route.path.without_first_if(&ChildKey::new(MENU_SLOT)) {
            self.menu.dispatch_event(&EventRoute::new(path), event, ctx)
        } else if !self.menu.is_active() {
            self.base.dispatch_event(route, event, ctx)
        } else {
            EventOutcome::Handled
        }
    }
    fn tick(&mut self, dt: Duration, settings: AnimationSettings) -> TickResult {
        self.base
            .tick(dt, settings)
            .merge(self.menu.tick(dt, settings))
    }
    fn take_pending_focus_request(&mut self) -> Option<FocusRequest> {
        if self.menu.is_active() {
            self.menu.take_pending_focus_request()
        } else {
            self.base.take_pending_focus_request()
        }
    }
    fn take_pending_clipboard_request(&mut self) -> Option<String> {
        self.base.take_pending_clipboard_request()
    }
    fn focus(&mut self, target: Option<&FocusId>, focused: bool, ctx: &mut FocusCtx<Msg>) {
        if self.menu.is_active() {
            self.menu.focus(target, focused, ctx);
        } else {
            self.base.focus(target, focused, ctx);
        }
    }
    fn dispatch_focus(&mut self, target: &FocusTarget, focused: bool, ctx: &mut FocusCtx<Msg>) {
        if let Some(target) = target.for_child(&ChildKey::new(MENU_SLOT)) {
            self.menu.dispatch_focus(&target, focused, ctx);
        } else {
            self.base.dispatch_focus(target, focused, ctx);
        }
    }
    fn focus_reveal_area(&self, target: &FocusTarget) -> Option<Rect> {
        if let Some(target) = target.for_child(&ChildKey::new(MENU_SLOT)) {
            self.menu.focus_reveal_area(&target)
        } else {
            self.base.focus_reveal_area(target)
        }
    }
    fn focus_reveal_centered(&self, target: &FocusTarget) -> bool {
        if let Some(target) = target.for_child(&ChildKey::new(MENU_SLOT)) {
            self.menu.focus_reveal_centered(&target)
        } else {
            self.base.focus_reveal_centered(target)
        }
    }
    fn init(&mut self, ctx: &mut LifecycleCtx<Msg>) {
        self.base.init(ctx);
        self.menu.init(ctx);
    }
    fn mount(&mut self, ctx: &mut LifecycleCtx<Msg>) {
        self.base.mount(ctx);
        self.menu.mount(ctx);
    }
    fn unmount(&mut self, ctx: &mut LifecycleCtx<Msg>) {
        self.menu.unmount(ctx);
        self.base.unmount(ctx);
    }
    fn destroy(&mut self, ctx: &mut LifecycleCtx<Msg>) {
        self.menu.destroy(ctx);
        self.base.destroy(ctx);
    }
}
