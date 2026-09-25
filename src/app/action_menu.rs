use std::{cell::RefCell, rc::Rc, time::Duration};

use ratatui::{
    Frame,
    layout::Rect,
    style::Style,
    text::{Line, Span, Text},
};
use tuicore::{
    AnimationSettings, Dropdown, DropdownCommitMode, DropdownLabelPosition, DropdownSearchMode,
    DropdownVariant, EventCtx, EventOutcome, EventRoute, FocusCtx, FocusId, FocusTarget, KeySpec,
    LayoutCtx, LayoutProposal, LayoutResult, LayoutSizeHint, LifecycleCtx, RenderCtx, TickResult,
    TuiEvent, TuiNode, line_width,
};

use super::{Msg, open_route_key};

const MENU_FIELD_WIDTH: u16 = 42;
const MENU_CONTENT_WIDTH: u16 = MENU_FIELD_WIDTH - 2;
const MENU_HEIGHT: u16 = 10;

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub(super) enum Action {
    CopyTemplateName,
    CopyInstanceName,
    CopyServiceName,
    CopyCheckoutPath,
    Yank,
    UpdateDescription,
    OpenBrowser,
    OpenCommand,
    Details,
    NewInstance,
    Start,
    Stop,
    StartService,
    StopService,
    RestartInstance,
    RestartService,
    StopTemplate,
    Purge,
    RetryCleanup,
    DeleteTemplate,
    RemoveTemplate,
}

impl Action {
    pub(super) fn index(self) -> usize {
        match self {
            Self::CopyTemplateName
            | Self::CopyInstanceName
            | Self::CopyServiceName
            | Self::CopyCheckoutPath => unreachable!("copy actions have fixed hotkeys"),
            Self::Yank => unreachable!("yank opens its own menu"),
            Self::UpdateDescription => unreachable!("description editing opens its own dialog"),
            Self::OpenBrowser | Self::OpenCommand => 10,
            Self::RestartInstance | Self::RestartService => 7,
            Self::Details => 0,
            Self::NewInstance => 1,
            Self::Start
            | Self::Stop
            | Self::StartService
            | Self::StopService
            | Self::StopTemplate => 3,
            Self::RemoveTemplate => 5,
            Self::Purge | Self::RetryCleanup | Self::DeleteTemplate => 6,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::CopyTemplateName => "Copy template name",
            Self::CopyInstanceName => "Copy instance name",
            Self::CopyServiceName => "Copy service name",
            Self::CopyCheckoutPath => "Copy checkout directory",
            Self::Yank => "Yank",
            Self::UpdateDescription => "Update description",
            Self::OpenBrowser => "Open in browser",
            Self::OpenCommand => "Run open command",
            Self::Details => "View details",
            Self::NewInstance => "New instance",
            Self::Start => "Start instance",
            Self::Stop => "Stop instance",
            Self::StartService => "Start service",
            Self::StopService => "Stop service",
            Self::RestartInstance => "Restart instance",
            Self::RestartService => "Restart service",
            Self::StopTemplate => "Stop all instances",
            Self::Purge => "Purge instance",
            Self::RetryCleanup => "Retry cleanup",
            Self::DeleteTemplate => "Purge all instances",
            Self::RemoveTemplate => "Delete template",
        }
    }
}

pub(super) struct ActionMenu {
    dropdown: Dropdown<Action, Action>,
    selected: Rc<RefCell<Option<Action>>>,
    actions: Vec<Action>,
    enabled: Rc<RefCell<Vec<Action>>>,
    field_area: Rect,
}

pub(super) struct Target {
    pub template: bool,
    pub instance: bool,
    pub service: bool,
    pub capabilities: (bool, bool, bool),
    pub gateway: bool,
    pub template_available: bool,
    pub repository: bool,
    pub cleanup: bool,
}

impl ActionMenu {
    pub(super) fn new(keys: [KeySpec; 11]) -> Self {
        let selected = Rc::new(RefCell::new(None));
        let selection = Rc::clone(&selected);
        let enabled = Rc::new(RefCell::new(Vec::new()));
        let enabled_for_renderer = Rc::clone(&enabled);
        let labels = keys;
        let dropdown = Dropdown::single_rich(
            [Action::Yank],
            |action| *action,
            |action| action.label().to_owned(),
            move |action, _, _| {
                action_text(
                    *action,
                    &labels,
                    enabled_for_renderer.borrow().contains(action),
                )
            },
        )
        .variant(DropdownVariant::Filled)
        .label("Actions")
        .label_position(DropdownLabelPosition::Inline)
        .search_mode(DropdownSearchMode::Fuzzy)
        .commit_mode(DropdownCommitMode::Explicit)
        .centered(true)
        .show_field_when_open(false)
        .tab_stop(false)
        .max_popup_height(MENU_HEIGHT)
        .on_select(move |actions| *selection.borrow_mut() = actions.first().copied());
        Self {
            dropdown,
            selected,
            actions: Vec::new(),
            enabled,
            field_area: Rect::default(),
        }
    }

    pub(super) fn open(&mut self, target: Target, ctx: &mut EventCtx<Msg>) {
        self.actions = if target.cleanup {
            vec![
                Action::CopyInstanceName,
                Action::Details,
                Action::RetryCleanup,
            ]
        } else if target.repository {
            vec![
                Action::CopyCheckoutPath,
                Action::Details,
                Action::NewInstance,
            ]
        } else if target.gateway {
            vec![
                Action::Yank,
                Action::OpenBrowser,
                Action::Details,
                Action::NewInstance,
                Action::StartService,
                Action::StopService,
                Action::RestartService,
            ]
        } else if target.template {
            vec![
                Action::CopyTemplateName,
                Action::Details,
                Action::NewInstance,
                Action::StopTemplate,
                Action::DeleteTemplate,
                Action::RemoveTemplate,
            ]
        } else if target.instance {
            vec![
                Action::Yank,
                Action::UpdateDescription,
                Action::Details,
                Action::OpenCommand,
                Action::NewInstance,
                Action::Start,
                Action::Stop,
                Action::RestartInstance,
                Action::Purge,
            ]
        } else {
            let mut actions = vec![
                Action::Details,
                Action::NewInstance,
                Action::StartService,
                Action::StopService,
                Action::RestartService,
            ];
            if target.service {
                actions.insert(0, Action::CopyServiceName);
            }
            actions
        };
        *self.enabled.borrow_mut() = self
            .actions
            .iter()
            .copied()
            .filter(|action| match action {
                Action::Start | Action::StartService => target.capabilities.0,
                Action::Stop | Action::StopService => target.capabilities.1,
                Action::RestartInstance | Action::RestartService => target.capabilities.2,
                _ => true,
            })
            .collect();
        if !target.template_available {
            self.enabled
                .borrow_mut()
                .retain(|action| !matches!(action, Action::RemoveTemplate | Action::NewInstance));
        }
        self.selected.borrow_mut().take();
        self.dropdown.clear_selection();
        self.dropdown.set_rows(self.actions.clone());
        self.dropdown.set_search_query("");
        self.dropdown.open_with_context(ctx);
    }

    pub(super) fn is_open(&self) -> bool {
        self.dropdown.is_open()
    }

    pub(super) fn take_action(&mut self) -> Option<Action> {
        let action = self.selected.borrow_mut().take()?;
        self.enabled.borrow().contains(&action).then_some(action)
    }
}

fn action_text(action: Action, keys: &[KeySpec; 11], enabled: bool) -> Text<'static> {
    let label = action.label();
    let hotkey = match action {
        Action::CopyTemplateName | Action::CopyCheckoutPath => "yy".into(),
        Action::CopyInstanceName => "yi".into(),
        Action::CopyServiceName => String::new(),
        Action::Yank => "y".into(),
        Action::UpdateDescription => "d".into(),
        Action::OpenBrowser => open_route_key().label(),
        _ => keys
            .get(action.index())
            .copied()
            .map(KeySpec::label)
            .unwrap_or_default(),
    };
    let spacing = usize::from(MENU_CONTENT_WIDTH)
        .saturating_sub(line_width(&Line::from(label)))
        .saturating_sub(line_width(&Line::from(hotkey.as_str())));
    Text::from(Line::from(vec![
        Span::styled(
            label,
            if enabled {
                Style::default()
            } else {
                Style::default().fg(tuicore::theme().muted_fg())
            },
        ),
        Span::raw(" ".repeat(spacing)),
        Span::styled(hotkey, Style::default().fg(tuicore::theme().muted_fg())),
    ]))
}

impl TuiNode<Msg> for ActionMenu {
    fn measure(&self, proposal: LayoutProposal) -> LayoutSizeHint {
        LayoutSizeHint::content(MENU_FIELD_WIDTH, MENU_HEIGHT).normalized(proposal)
    }

    fn layout(&mut self, area: Rect, ctx: &mut LayoutCtx) -> LayoutResult {
        let width = MENU_FIELD_WIDTH.min(area.width);
        let height = <Dropdown<Action, Action> as TuiNode<Msg>>::measure(
            &self.dropdown,
            LayoutProposal::at_most(width, area.height),
        )
        .preferred
        .height
        .min(area.height);
        self.field_area = Rect::new(
            area.x.saturating_add(area.width.saturating_sub(width) / 2),
            area.y
                .saturating_add(area.height.saturating_sub(height) / 2),
            width,
            height,
        );
        <Dropdown<Action, Action> as TuiNode<Msg>>::layout(
            &mut self.dropdown,
            self.field_area,
            ctx,
        );
        LayoutResult::new(area)
    }

    fn render<'a>(&'a self, frame: &mut Frame, _area: Rect, ctx: &mut RenderCtx<'a>) {
        <Dropdown<Action, Action> as TuiNode<Msg>>::render(
            &self.dropdown,
            frame,
            self.field_area,
            ctx,
        );
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
        <Dropdown<Action, Action> as TuiNode<Msg>>::tick(&mut self.dropdown, dt, settings)
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
