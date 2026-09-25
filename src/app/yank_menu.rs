use std::{cell::RefCell, rc::Rc, time::Duration};

use ratatui::{
    Frame,
    layout::Rect,
    style::Style,
    text::{Line, Text},
};
use tuicore::{
    AnimationSettings, Dropdown, DropdownCommitMode, DropdownLabelPosition, DropdownSearchMode,
    DropdownVariant, EventCtx, EventOutcome, EventRoute, FocusCtx, FocusId, FocusTarget,
    HotkeyEvent, HotkeyLabelMode, Key, KeyModifiers, LayoutCtx, LayoutProposal, LayoutResult,
    LayoutSizeHint, LifecycleCtx, RenderCtx, TickResult, TuiEvent, TuiNode, hotkey_label_spans,
    hotkey_underline_style,
};

use super::Msg;

const MENU_ANCHOR_WIDTH: u16 = 1;
const MENU_HEIGHT: u16 = 4;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(super) enum YankAction {
    Instance,
    Description,
    Workspace,
    Url,
}

impl YankAction {
    fn hotkey(self) -> char {
        match self {
            Self::Instance => 'i',
            Self::Description => 'd',
            Self::Workspace => 'w',
            Self::Url => 'u',
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Instance => "Instance",
            Self::Description => "Description",
            Self::Workspace => "Workspace",
            Self::Url => "URL",
        }
    }

    pub(super) fn text(self, target: &YankTarget) -> Option<String> {
        match (self, target) {
            (Self::Instance, YankTarget::Instance { name, .. }) => Some(name.clone()),
            (Self::Description, YankTarget::Instance { description, .. }) => {
                Some(description.clone())
            }
            (Self::Workspace, YankTarget::Instance { workspace, .. }) => Some(workspace.clone()),
            (Self::Url, YankTarget::Service { url }) => Some(url.clone()),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum YankTarget {
    Instance {
        name: String,
        description: String,
        workspace: String,
    },
    Service {
        url: String,
    },
}

impl YankTarget {
    fn actions(&self) -> Vec<YankAction> {
        match self {
            Self::Instance { .. } => vec![
                YankAction::Instance,
                YankAction::Description,
                YankAction::Workspace,
            ],
            Self::Service { .. } => vec![YankAction::Url],
        }
    }

    fn label(&self) -> &'static str {
        match self {
            Self::Instance { .. } => "Yank instance",
            Self::Service { .. } => "Yank service",
        }
    }
}

pub(super) struct YankMenu {
    dropdown: Dropdown<YankAction, YankAction>,
    selected: Rc<RefCell<Option<YankAction>>>,
    target: Option<YankTarget>,
    field_area: Rect,
}

impl YankMenu {
    pub(super) fn new() -> Self {
        let selected = Rc::new(RefCell::new(None));
        let selection = Rc::clone(&selected);
        let dropdown = Dropdown::single_rich(
            Vec::<YankAction>::new(),
            |action| *action,
            |action| action.label().to_owned(),
            |action, _, _| action_text(*action),
        )
        .variant(DropdownVariant::Filled)
        .label("Yank")
        .label_position(DropdownLabelPosition::Inline)
        .search_mode(DropdownSearchMode::None)
        .commit_mode(DropdownCommitMode::Explicit)
        .centered(true)
        .show_field_when_open(false)
        .backdrop_amount(0.55)
        .tab_stop(false)
        .max_popup_height(MENU_HEIGHT)
        .max_popup_width(u16::MAX)
        .on_select(move |actions| *selection.borrow_mut() = actions.first().copied());
        Self {
            dropdown,
            selected,
            target: None,
            field_area: Rect::default(),
        }
    }

    pub(super) fn open(&mut self, target: YankTarget, ctx: &mut EventCtx<Msg>) {
        self.selected.borrow_mut().take();
        self.dropdown.set_label(target.label());
        self.dropdown.set_rows(target.actions());
        self.target = Some(target);
        self.dropdown.clear_selection();
        self.dropdown.open_with_context(ctx);
    }

    pub(super) fn is_open(&self) -> bool {
        self.dropdown.is_open()
    }

    pub(super) fn take_selection(&mut self) -> Option<(YankAction, YankTarget)> {
        let action = self.selected.borrow_mut().take()?;
        Some((action, self.target.clone()?))
    }

    fn handle_shortcut(&mut self, event: &TuiEvent, ctx: &mut EventCtx<Msg>) -> bool {
        let hotkey = match event {
            TuiEvent::Key(key) if key.modifiers == KeyModifiers::NONE => match key.code {
                Key::Char(character) => Some(character),
                _ => None,
            },
            TuiEvent::Hotkey(HotkeyEvent::Commit(sequence)) => sequence
                .strip_prefix('y')
                .filter(|suffix| suffix.chars().count() == 1)
                .and_then(|suffix| suffix.chars().next()),
            _ => None,
        };
        let Some(action) = hotkey.and_then(|hotkey| {
            self.target
                .as_ref()?
                .actions()
                .into_iter()
                .find(|action| action.hotkey() == hotkey)
        }) else {
            return false;
        };
        *self.selected.borrow_mut() = Some(action);
        self.dropdown.close();
        ctx.stop_propagation();
        ctx.request_layout();
        ctx.request_redraw();
        true
    }
}

fn action_text(action: YankAction) -> Text<'static> {
    let base = Style::default().fg(tuicore::theme().text_fg());
    Text::from(Line::from(hotkey_label_spans(
        action.label(),
        Some(&action.hotkey().to_string()),
        HotkeyLabelMode::PreferMnemonic,
        None,
        base,
        hotkey_underline_style(base),
    )))
}

impl TuiNode<Msg> for YankMenu {
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
        if self.handle_shortcut(event, ctx) {
            EventOutcome::Handled
        } else {
            self.dropdown.event(event, ctx)
        }
    }

    fn dispatch_event(
        &mut self,
        route: &EventRoute,
        event: &TuiEvent,
        ctx: &mut EventCtx<Msg>,
    ) -> EventOutcome {
        if self.handle_shortcut(event, ctx) {
            EventOutcome::Handled
        } else {
            self.dropdown.dispatch_event(route, event, ctx)
        }
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
