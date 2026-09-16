use std::time::Duration;

use ratatui::{
    Frame,
    layout::{Constraint, Rect},
};
use tuicore::{
    AnimationSettings, Button, Dialog, DialogBackdrop, DialogHost, DialogLayer,
    DialogLayerPlacement, DockChrome, DockSpec, EventCtx, EventOutcome, EventRoute, Flex, FlexItem,
    FocusCtx, FocusId, FocusTarget, HotkeyLabelMode, KeySpec, LayoutCtx, LayoutProposal,
    LayoutResult, LayoutSizeHint, LifecycleCtx, Notification, RenderCtx, Split, StatusBar,
    StatusBarMenuItem, Tab, Tabs, TabsVariant, TickResult, TuiEvent, TuiNode,
};

use crate::{service::AppService, store::environments::EnvironmentSnapshot};

mod action_menu;
mod details;
mod dialogs;
mod instances;
mod properties;
mod rows;
use action_menu::ActionMenu;
use instances::{Instances, SharedState};
use rows::Row;

const TREE_FOCUS: &str = "environments";
const MOBILE_TABS_WIDTH: u16 = 100;
const SETTINGS_MENU_ID: &str = "settings";
const STATUS_BAR_MENU_ITEMS: [StatusBarMenuItem; 2] = [
    StatusBarMenuItem::Custom {
        id: SETTINGS_MENU_ID,
        label: " Settings",
    },
    StatusBarMenuItem::Theme,
];

pub(crate) fn initial_focus() -> tuicore::FocusRequest {
    tuicore::FocusRequest::Target(FocusId::new(TREE_FOCUS))
}

#[derive(Debug)]
pub(crate) enum Msg {
    Close,
    NameChanged(String),
    OpenSettings,
    OpenCommandChanged(String),
    NewTemplate,
    SetBranchInstances(bool),
    Submit,
}

enum Intent {
    CreateInstance(String),
    Resume { name: String, template: String },
    NewTemplate,
    Stop(String),
    Delete(String),
    StopTemplate(String),
    DeleteTemplate(String),
    RemoveTemplate(String),
}

type Content = Tabs<Msg>;
trait ModalNode: TuiNode<Msg> + DockChrome {
    fn set_bottom_left(&mut self, _title: String) {}
}

impl ModalNode for DialogHost<Flex<Msg>, Msg> {
    fn set_bottom_left(&mut self, title: String) {
        self.dialog_mut().set_bottom_left(title);
    }
}

impl ModalNode for Tabs<Msg> {}

type Modal = Box<dyn ModalNode>;
type MenuLayer = DialogLayer<Content, ActionMenu>;
type MainView = DialogLayer<MenuLayer, Modal>;
type View = Split<MainView, StatusBar<Msg>>;

pub(crate) struct App {
    service: AppService,
    snapshot: EnvironmentSnapshot,
    view: View,
    instances: SharedState,
    keys: [KeySpec; 7],
    poll_elapsed: Duration,
    intent: Option<Intent>,
    name: String,
    open_command: String,
    settings_save: Option<tokio::sync::oneshot::Receiver<Result<String, String>>>,
    area: Rect,
    details_open: bool,
}

pub(crate) fn root(service: AppService) -> App {
    let key_chars = service.environment_keys();
    let keys = key_chars.map(|key| {
        if key.is_ascii_uppercase() {
            KeySpec::shifted(key.to_ascii_lowercase())
        } else {
            KeySpec::plain(key)
        }
    });
    let template_hotkey = if key_chars[2].is_ascii_uppercase() {
        format!("shift+{}", key_chars[2].to_ascii_lowercase())
    } else {
        key_chars[2].to_string()
    };
    let snapshot = service.environment_snapshot();
    let instances = instances::state(rows::from_snapshot(&snapshot));
    let content = Tabs::new(vec![Tab::new(
        "Instances",
        Flex::column()
            .child(
                "template-actions",
                Flex::row().child(
                    "new-template",
                    Button::new("Template")
                        .hotkey(template_hotkey)
                        .hotkey_label_mode(HotkeyLabelMode::Inline)
                        .on_press(|| Msg::NewTemplate),
                    FlexItem::fit_content(),
                ),
                FlexItem::fit_content(),
            )
            .child(
                "instances",
                Instances::new(instances.clone()),
                FlexItem::fill(1),
            ),
    )])
    .variant(TabsVariant::OneRow);
    let menu = DialogLayer::new(content, ActionMenu::new(keys))
        .active(false)
        .fit_content()
        .fit_content_max(42, 8);
    let modal: Modal = Box::new(
        Dialog::<Msg>::new()
            .on_close(|_| Msg::Close)
            .host(Flex::column()),
    );
    let main = DialogLayer::new(menu, modal)
        .active(false)
        .fit_content()
        .fit_content_max(110, 34)
        .backdrop(DialogBackdrop::dim().amount(0.55));
    let view = Split::vertical(
        main,
        StatusBar::new()
            .ai_enabled(false)
            .menu_items(STATUS_BAR_MENU_ITEMS)
            .on_custom_menu_item(|id| match id {
                SETTINGS_MENU_ID => Msg::OpenSettings,
                _ => Msg::Close,
            }),
    )
    .constraints(Constraint::Fill(1), Constraint::Length(1));
    service.poll_environments();
    App {
        service,
        snapshot,
        view,
        instances,
        keys,
        poll_elapsed: Duration::ZERO,
        intent: None,
        name: String::new(),
        open_command: String::new(),
        settings_save: None,
        area: Rect::default(),
        details_open: false,
    }
}

impl App {
    fn selected(&self) -> Option<Row> {
        instances::selected(&self.instances)
    }

    fn menu_layer(&self) -> &MenuLayer {
        self.view.first().base()
    }

    fn menu_layer_mut(&mut self) -> &mut MenuLayer {
        self.view.first_mut().base_mut()
    }

    #[cfg(test)]
    fn set_rows_for_tests(&mut self, rows: Vec<Row>) {
        let highlighted = rows.first().map(|row| row.id.clone());
        instances::replace_rows(&self.instances, rows);
        instances::set_highlighted(&self.instances, highlighted);
    }

    pub(crate) fn handle_message(&mut self, message: Msg, ctx: &mut EventCtx<Msg>) {
        match message {
            Msg::Close => {
                self.settings_save = None;
                self.view.first_mut().set_active_with_context(false, ctx);
                ctx.focus(initial_focus());
                self.intent = None;
                self.details_open = false;
            }
            Msg::NameChanged(name) => self.name = name,
            Msg::OpenSettings => self.open_settings(ctx),
            Msg::OpenCommandChanged(command) => {
                self.open_command = command;
                match self.service.set_open_command(self.open_command.clone()) {
                    Ok(reply) => {
                        self.settings_save = Some(reply);
                    }
                    Err(error) => {
                        ctx.notify(Notification::error("Cannot save open command", error))
                    }
                }
            }
            Msg::NewTemplate => self.action(2, ctx),
            Msg::SetBranchInstances(enabled) => {
                if let Err(error) = self.service.set_branch_instances(enabled) {
                    ctx.notify(Notification::error("Cannot save settings", error));
                }
            }
            Msg::Submit => {
                let result = match &self.intent {
                    Some(Intent::CreateInstance(template)) => self.service.submit_operation(
                        "create_instance",
                        &self.name,
                        Some(template.clone()),
                        600,
                        true,
                    ),
                    Some(Intent::Resume { name, template }) => self.service.submit_operation(
                        "create_instance",
                        name,
                        Some(template.clone()),
                        600,
                        true,
                    ),
                    Some(Intent::NewTemplate) => {
                        self.service
                            .submit_operation("create_template", &self.name, None, 60, true)
                    }
                    Some(Intent::Stop(name)) => {
                        self.service
                            .submit_operation("stop_instance", name, None, 60, true)
                    }
                    Some(Intent::Delete(name)) => {
                        self.service
                            .submit_operation("delete_instance", name, None, 60, true)
                    }
                    Some(Intent::StopTemplate(name)) => {
                        self.service
                            .submit_operation("stop_template", name, None, 60, true)
                    }
                    Some(Intent::DeleteTemplate(name)) => {
                        self.service
                            .submit_operation("delete_template", name, None, 60, true)
                    }
                    Some(Intent::RemoveTemplate(name)) => {
                        self.service
                            .submit_operation("remove_template", name, None, 600, true)
                    }
                    None => return,
                };
                match result {
                    Ok(_) => {
                        self.view.first_mut().set_active_with_context(false, ctx);
                        ctx.focus(initial_focus());
                        self.intent = None;
                        self.details_open = false;
                    }
                    Err(error) => {
                        self.view.first_mut().layer_mut().set_bottom_left(error);
                    }
                }
            }
        }
        ctx.request_redraw();
    }

    fn open(&mut self, modal: Modal, ctx: &mut EventCtx<Msg>) {
        self.details_open = false;
        self.view.first_mut().replace_layer(modal, ctx);
        self.view.first_mut().set_fit_content(true);
        self.view.first_mut().set_fit_content_max(110, 34);
        self.view
            .first_mut()
            .set_placement(DialogLayerPlacement::Center);
        self.view.first_mut().set_active_with_context(true, ctx);
        if matches!(
            self.intent,
            Some(Intent::CreateInstance(_) | Intent::NewTemplate)
        ) {
            ctx.focus(tuicore::FocusRequest::Path(tuicore::TreePath::from_keys([
                tuicore::ChildKey::first(),
                tuicore::ChildKey::second(),
                tuicore::ChildKey::body(),
                tuicore::ChildKey::new("name"),
            ])));
        }
    }

    fn open_name_entry(&mut self, ctx: &mut EventCtx<Msg>) {
        let dialog = match &self.intent {
            Some(Intent::CreateInstance(_)) => {
                let branch_instances = self.service.branch_instances();
                dialogs::name_entry(
                    "New instance",
                    &self.name,
                    if branch_instances {
                        "branch-name"
                    } else {
                        "Instance name"
                    },
                    branch_instances.then_some(
                        "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789-",
                    ),
                )
            }
            Some(Intent::NewTemplate) => dialogs::name_entry(
                "New template",
                &self.name,
                "Template name",
                Some("abcdefghijklmnopqrstuvwxyz0123456789-"),
            ),
            _ => return,
        };
        self.open(dialog, ctx);
    }

    fn open_settings(&mut self, ctx: &mut EventCtx<Msg>) {
        self.intent = None;
        self.settings_save = None;
        self.open_command = self.service.open_command();
        self.open(
            dialogs::settings(self.service.branch_instances(), &self.open_command),
            ctx,
        );
    }

    fn open_details(&mut self, row: &Row, ctx: &mut EventCtx<Msg>) {
        self.view
            .first_mut()
            .replace_layer(dialogs::details(row), ctx);
        self.details_open = true;
        self.resize_details_dialog();
        self.view.first_mut().set_active_with_context(true, ctx);
    }

    fn resize_details_dialog(&mut self) {
        let dock = DockSpec::bottom(50).cross_percent(details_width_percent(self.area.width));
        let layer = self.view.first_mut();
        layer.set_dock(dock);
        layer.layer_mut().set_dock_edge_borders(dock.edge_borders());
    }

    fn open_action_menu(&mut self, ctx: &mut EventCtx<Msg>) -> bool {
        let Some(row) = self.selected() else {
            return false;
        };
        if row.parent.is_some() && row.instance.is_none() && row.gateway_url.is_none() {
            return false;
        }
        let menu = self.menu_layer_mut();
        menu.layer_mut().open(
            row.parent.is_none(),
            row.running,
            row.gateway_url.is_some(),
            !row.compose_file.is_empty(),
            ctx,
        );
        menu.set_active_with_context(true, ctx);
        true
    }

    fn drain_action_menu(&mut self, ctx: &mut EventCtx<Msg>) {
        if !self.menu_layer().is_active() {
            return;
        }
        let action = self.menu_layer_mut().layer_mut().take_action();
        if action.is_none() && self.menu_layer().layer().is_open() {
            return;
        }
        self.menu_layer_mut().set_active_with_context(false, ctx);
        if let Some(action) = action {
            self.action(action.index(), ctx);
        }
    }

    fn action(&mut self, index: usize, ctx: &mut EventCtx<Msg>) {
        let row = self.selected();
        self.name.clear();
        match index {
            0 => {
                if let Some(row) = row {
                    self.intent = None;
                    self.open_details(&row, ctx);
                }
            }
            1 => {
                if let Some(row) = row.filter(|row| row.parent.is_none()) {
                    self.intent = Some(Intent::CreateInstance(row.template.clone()));
                    self.open_name_entry(ctx);
                }
            }
            2 => {
                self.intent = Some(Intent::NewTemplate);
                self.open_name_entry(ctx);
            }
            3 => {
                if let Some(row) = row.as_ref().filter(|row| row.parent.is_none()) {
                    let template = row.template.clone();
                    self.intent = Some(Intent::StopTemplate(template.clone()));
                    self.open(dialogs::confirm_stop_template(&template), ctx);
                } else if let Some(row) = row.filter(|row| row.instance.is_some()) {
                    let name = row.instance.expect("instance rows have a name");
                    if row.running {
                        self.intent = Some(Intent::Stop(name.clone()));
                        self.open(dialogs::confirm_stop(&name), ctx);
                    } else {
                        self.intent = Some(Intent::Resume {
                            name: name.clone(),
                            template: row.template,
                        });
                        self.open(dialogs::confirm_start(&name), ctx);
                    }
                }
            }
            4 => self.service.poll_environments(),
            5 => {
                if let Some(row) = row
                    .as_ref()
                    .filter(|row| row.parent.is_none() && !row.compose_file.is_empty())
                {
                    self.intent = Some(Intent::RemoveTemplate(row.template.clone()));
                    self.open(
                        dialogs::confirm_remove_template(&row.template, &row.directory),
                        ctx,
                    );
                } else if let Some(name) = row.and_then(|row| row.instance) {
                    self.intent = Some(Intent::Delete(name.clone()));
                    self.open(dialogs::confirm_delete(&name), ctx);
                }
            }
            7 => {
                self.open_gateway(ctx);
            }
            6 => {
                if let Some(row) = row.filter(|row| row.parent.is_none()) {
                    self.intent = Some(Intent::DeleteTemplate(row.template.clone()));
                    self.open(dialogs::confirm_delete_template(&row.template), ctx);
                }
            }
            _ => {}
        }
        ctx.request_redraw();
    }

    fn open_gateway(&self, ctx: &mut EventCtx<Msg>) -> bool {
        let Some(url) = self.selected().and_then(|row| row.gateway_url) else {
            return false;
        };
        if let Err(error) = self.service.open_gateway(&url) {
            ctx.notify(Notification::error("Cannot open gateway", error));
        }
        true
    }

    fn open_workspace(&self, ctx: &mut EventCtx<Msg>) -> bool {
        let Some((workspace, instance)) = self
            .selected()
            .and_then(|row| Some((row.workspace?, row.instance?)))
        else {
            return false;
        };
        if let Err(error) = self.service.open_workspace(&workspace, &instance) {
            ctx.notify(Notification::error("Cannot open workspace", error));
        }
        true
    }

    fn handle_key(&mut self, event: &TuiEvent, ctx: &mut EventCtx<Msg>) -> bool {
        if self.view.first().is_active()
            && matches!(
                self.intent,
                Some(Intent::CreateInstance(_) | Intent::NewTemplate)
            )
            && let TuiEvent::Key(key) = event
            && (KeySpec::key(tuicore::Key::Enter).matches(*key)
                || KeySpec::key_with_modifiers(tuicore::Key::Enter, tuicore::KeyModifiers::CONTROL)
                    .matches(*key))
        {
            self.handle_message(Msg::Submit, ctx);
            ctx.stop_propagation();
            return true;
        }
        if self.menu_layer().is_active() || self.view.first().is_active() {
            return false;
        }
        if instances::is_searching(&self.instances) {
            return false;
        }
        if let TuiEvent::Key(key) = event
            && KeySpec::key(tuicore::Key::Enter).matches(*key)
            && (self.open_workspace(ctx) || self.open_gateway(ctx))
        {
            ctx.stop_propagation();
            return true;
        }
        if let TuiEvent::Key(key) = event
            && KeySpec::plain('.').matches(*key)
            && self.open_action_menu(ctx)
        {
            ctx.stop_propagation();
            return true;
        }
        if let TuiEvent::Key(key) = event
            && let Some(index) = self.keys.iter().position(|spec| spec.matches(*key))
        {
            self.action(index, ctx);
            ctx.stop_propagation();
            return true;
        }
        false
    }

    fn after_event(&mut self, ctx: &mut EventCtx<Msg>) {
        self.drain_action_menu(ctx);
        ctx.request_redraw();
    }
}

impl TuiNode<Msg> for App {
    fn measure(&self, proposal: LayoutProposal) -> LayoutSizeHint {
        self.view.measure(proposal)
    }
    fn layout(&mut self, area: Rect, ctx: &mut LayoutCtx) -> LayoutResult {
        self.area = area;
        if self.details_open {
            self.resize_details_dialog();
        }
        self.view.layout(area, ctx)
    }
    fn render<'a>(&'a self, frame: &mut Frame, area: Rect, ctx: &mut RenderCtx<'a>) {
        self.view.render(frame, area, ctx);
    }
    fn event(&mut self, event: &TuiEvent, ctx: &mut EventCtx<Msg>) -> EventOutcome {
        if self.handle_key(event, ctx) {
            return EventOutcome::Handled;
        }
        let outcome = if self.menu_layer().is_active() {
            self.menu_layer_mut().event(event, ctx)
        } else {
            self.view.event(event, ctx)
        };
        self.after_event(ctx);
        outcome
    }
    fn dispatch_event(
        &mut self,
        route: &EventRoute,
        event: &TuiEvent,
        ctx: &mut EventCtx<Msg>,
    ) -> EventOutcome {
        // Toolbar and status-bar controls own their input, including instance action keys.
        if route
            .path
            .without_first_if(&tuicore::ChildKey::second())
            .is_some()
            || route
                .path
                .keys()
                .iter()
                .any(|key| key.as_str() == "template-actions")
        {
            return self.view.dispatch_event(route, event, ctx);
        }
        if self.handle_key(event, ctx) {
            return EventOutcome::Handled;
        }
        let outcome = if self.menu_layer().is_active() {
            self.menu_layer_mut().event(event, ctx)
        } else {
            self.view.dispatch_event(route, event, ctx)
        };
        self.after_event(ctx);
        outcome
    }
    fn tick(&mut self, dt: Duration, settings: AnimationSettings) -> TickResult {
        self.poll_elapsed = self.poll_elapsed.saturating_add(dt);
        if self.poll_elapsed >= Duration::from_secs(2) {
            self.poll_elapsed = Duration::ZERO;
            self.service.poll_environments();
        }
        let snapshot = self.service.environment_snapshot();
        let mut changed = false;
        if let Some(reply) = &mut self.settings_save {
            let message = match reply.try_recv() {
                Ok(Ok(_)) => None,
                Ok(Err(error)) => Some(format!("Cannot save open command: {error}")),
                Err(tokio::sync::oneshot::error::TryRecvError::Closed) => {
                    Some("Settings worker stopped".to_owned())
                }
                Err(tokio::sync::oneshot::error::TryRecvError::Empty) => None,
            };
            if let Some(message) = message {
                self.settings_save = None;
                self.view.first_mut().layer_mut().set_bottom_left(message);
                changed = true;
            }
        }
        if snapshot != self.snapshot {
            instances::replace_rows(&self.instances, rows::from_snapshot(&snapshot));
            self.snapshot = snapshot;
            changed = true;
        }
        let mut result = self.view.tick(dt, settings);
        if changed {
            result = result.merge(TickResult::CHANGED);
        }
        result.merge(TickResult::scheduled_after(Duration::from_millis(250)))
    }
    fn take_pending_focus_request(&mut self) -> Option<tuicore::FocusRequest> {
        self.view.take_pending_focus_request()
    }
    fn take_pending_clipboard_request(&mut self) -> Option<String> {
        self.view.take_pending_clipboard_request()
    }
    fn focus(&mut self, target: Option<&FocusId>, focused: bool, ctx: &mut FocusCtx<Msg>) {
        self.view.focus(target, focused, ctx);
    }
    fn dispatch_focus(&mut self, target: &FocusTarget, focused: bool, ctx: &mut FocusCtx<Msg>) {
        self.view.dispatch_focus(target, focused, ctx);
    }
    fn focus_reveal_area(&self, target: &FocusTarget) -> Option<Rect> {
        self.view.focus_reveal_area(target)
    }
    fn focus_reveal_centered(&self, target: &FocusTarget) -> bool {
        self.view.focus_reveal_centered(target)
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

fn details_width_percent(width: u16) -> u16 {
    if width < MOBILE_TABS_WIDTH { 100 } else { 60 }
}

#[cfg(test)]
mod tests;
