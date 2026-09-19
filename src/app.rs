use std::{
    cell::RefCell,
    rc::Rc,
    time::{Duration, Instant},
};

use ratatui::{
    Frame,
    layout::{Constraint, Rect},
};
use tuicore::{
    AnimationSettings, Dialog, DialogBackdrop, DialogHost, DialogLayer, DialogLayerPlacement,
    DockChrome, DockSpec, EventCtx, EventOutcome, EventRoute, Flex, FlexItem, FocusCtx, FocusId,
    FocusTarget, KeySpec, LayoutCtx, LayoutProposal, LayoutResult, LayoutSizeHint, LifecycleCtx,
    Notification, RenderCtx, Split, StatusBar, StatusBarMenuItem, Tab, Tabs, TabsVariant,
    TickResult, ToastRack, TuiEvent, TuiNode,
};

use crate::{service::AppService, store::environments::EnvironmentSnapshot};

mod action_menu;
mod bulk;
mod details;
mod dialogs;
mod instances;
mod operations;
mod properties;
mod refresh;
mod rows;
mod toolbar;
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
    CloseCommandChanged(String),
    NewTemplate,
    Refresh,
    StopAll,
    PurgeAll,
    SetBranchInstances(bool),
    CopyName,
    CopyGatewayUrl,
    Submit,
}

enum Intent {
    CreateInstance(String),
    Resume {
        name: String,
        template: String,
    },
    NewTemplate,
    Stop(String),
    ServiceState {
        name: String,
        service: String,
        running: bool,
    },
    Restart {
        name: String,
        service: Option<String>,
    },
    Purge(String),
    StopTemplate(String),
    DeleteTemplate(String),
    RemoveTemplate(String),
    StopAll(Vec<String>),
    PurgeAll(Vec<String>),
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
    toolbar_state: toolbar::SharedState,
    keys: [KeySpec; 11],
    refresh_schedule: refresh::RefreshSchedule,
    manual_refresh: Option<tokio::sync::oneshot::Receiver<Result<(), String>>>,
    notifications: ToastRack,
    deletions: Vec<operations::Deletion>,
    container_operations: Vec<crate::store::environments::Operation>,
    intent: Option<Intent>,
    name: String,
    open_command: String,
    settings_save: Option<tokio::sync::oneshot::Receiver<Result<String, String>>>,
    area: Rect,
    details_open: bool,
}

pub(crate) fn root(service: AppService) -> App {
    let keys = service.environment_keys();
    let snapshot = service.environment_snapshot();
    let instances = instances::state(rows::from_snapshot(&snapshot));
    let toolbar_state = Rc::new(RefCell::new(toolbar::State::from_snapshot(&snapshot)));
    let mut content = Tabs::new(vec![Tab::new(
        "Instances",
        Flex::column()
            .child(
                "template-actions",
                toolbar::Toolbar::new(keys[2], keys[4], keys[8], keys[9], toolbar_state.clone()),
                FlexItem::fit_content(),
            )
            .child(
                "instances",
                Instances::new(instances.clone()),
                FlexItem::fill(1),
            ),
    )])
    .variant(TabsVariant::OneRow)
    .action_hotkey("yy", |_| Msg::CopyName)
    .action_hotkey("yu", |_| Msg::CopyGatewayUrl);
    content.set_action_hotkey_visible("yy", false);
    content.set_action_hotkey_visible("yu", false);
    let menu = DialogLayer::new(content, ActionMenu::new(keys))
        .active(false)
        .fit_content()
        .fit_content_max(42, 9);
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
    service.refresh_environments();
    App {
        service,
        snapshot,
        view,
        instances,
        toolbar_state,
        keys,
        refresh_schedule: refresh::RefreshSchedule::default(),
        manual_refresh: None,
        notifications: ToastRack::new(),
        deletions: Vec::new(),
        container_operations: Vec::new(),
        intent: None,
        name: String::new(),
        open_command: String::new(),
        settings_save: None,
        area: Rect::default(),
        details_open: false,
    }
}

impl App {
    fn update_snapshot(&mut self, snapshot: EnvironmentSnapshot) -> bool {
        let operations = self.service.operations();
        let snapshot_changed = snapshot != self.snapshot;
        let rows_changed = instances::replace_rows(
            &self.instances,
            rows::from_snapshot_with_operations(&snapshot, &operations),
        );
        if !snapshot_changed {
            return rows_changed;
        }
        *self.toolbar_state.borrow_mut() = toolbar::State::from_snapshot(&snapshot);
        self.snapshot = snapshot;
        true
    }

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
            Msg::CloseCommandChanged(command) => match self.service.set_close_command(command) {
                Ok(reply) => self.settings_save = Some(reply),
                Err(error) => ctx.notify(Notification::error("Cannot save close command", error)),
            },
            Msg::NewTemplate => self.action(2, ctx),
            Msg::Refresh => self.action(4, ctx),
            Msg::StopAll => self.confirm_stop_all(ctx),
            Msg::PurgeAll => self.confirm_purge_all(ctx),
            Msg::SetBranchInstances(enabled) => {
                if let Err(error) = self.service.set_branch_instances(enabled) {
                    ctx.notify(Notification::error("Cannot save settings", error));
                }
            }
            Msg::CopyName => {
                self.copy_selected_name(ctx);
            }
            Msg::CopyGatewayUrl => {
                self.copy_gateway_url(ctx);
            }
            Msg::Submit => {
                if self.target_instance_has_operation() {
                    self.block_operation(ctx);
                    return;
                }
                let result = match &self.intent {
                    Some(Intent::StopAll(names)) => {
                        self.submit_instance_batch("stop_instance", names.clone(), ctx);
                        return;
                    }
                    Some(Intent::PurgeAll(names)) => {
                        self.submit_instance_batch("delete_instance", names.clone(), ctx);
                        return;
                    }
                    Some(Intent::CreateInstance(template)) => {
                        match self
                            .service
                            .submit_new_instance(&self.name, template.clone())
                        {
                            Ok(crate::service::CreateInstanceOutcome::Existing) => {
                                self.sync_environment();
                                instances::select_instance(&self.instances, &self.name);
                                self.notify(Notification::info(
                                    "Instance already exists",
                                    format!("{} already exists.", self.name),
                                ));
                                self.handle_message(Msg::Close, ctx);
                                ctx.request_layout();
                                return;
                            }
                            Ok(crate::service::CreateInstanceOutcome::Started(operation)) => {
                                Ok(*operation)
                            }
                            Err(error) => Err(error),
                        }
                    }
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
                    Some(Intent::Restart { name, service }) => {
                        self.service.submit_restart(name, service.clone(), true)
                    }
                    Some(Intent::ServiceState {
                        name,
                        service,
                        running,
                    }) => self
                        .service
                        .submit_service_state(name, service.clone(), *running, true),
                    Some(Intent::Purge(name)) => {
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
                    Ok(operation) => {
                        self.operation_accepted(operation);
                        ctx.request_layout();
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
        if self.target_instance_has_operation() {
            self.block_operation(ctx);
            return;
        }
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

    fn target_instance_has_operation(&self) -> bool {
        let Some(name) = self.intent_instance_target() else {
            return false;
        };
        self.snapshot
            .activities
            .iter()
            .any(|activity| activity.active() && activity.targets_instance_name(name))
            || self.service.operations().iter().any(|operation| {
                operation.state == crate::store::environments::OperationState::Running
                    && operation.targets_instance_name(name)
            })
    }

    fn intent_instance_target(&self) -> Option<&str> {
        match self.intent.as_ref()? {
            Intent::CreateInstance(_) => Some(&self.name),
            Intent::Resume { name, .. }
            | Intent::Stop(name)
            | Intent::Purge(name)
            | Intent::ServiceState { name, .. }
            | Intent::Restart { name, .. } => Some(name),
            Intent::NewTemplate
            | Intent::StopTemplate(_)
            | Intent::DeleteTemplate(_)
            | Intent::RemoveTemplate(_)
            | Intent::StopAll(_)
            | Intent::PurgeAll(_) => None,
        }
    }

    fn block_operation(&mut self, ctx: &mut EventCtx<Msg>) {
        self.intent = None;
        self.view.first_mut().set_active_with_context(false, ctx);
        ctx.focus(initial_focus());
        self.notify(Notification::warning(
            "Operation in progress",
            "Wait for this instance's operation to finish before starting another.",
        ));
        ctx.request_redraw();
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
            dialogs::settings(
                self.service.branch_instances(),
                &self.open_command,
                &self.service.close_command(),
            ),
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
        if row.informational {
            return false;
        }
        let menu = self.menu_layer_mut();
        menu.layer_mut().open(
            action_menu::Target {
                template: row.parent.is_none(),
                instance: row.instance.is_some(),
                service: row.service_name.is_some(),
                capabilities: (row.can_start, row.can_stop, row.can_restart),
                gateway: row.gateway_url.is_some(),
                template_available: row.template_available,
                repository: row.checkout_path.is_some(),
                cleanup: row.cleanup_target.is_some(),
            },
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
            match action {
                action_menu::Action::CopyTemplateName
                | action_menu::Action::CopyInstanceName
                | action_menu::Action::CopyServiceName
                | action_menu::Action::CopyCheckoutPath => {
                    self.copy_selected_name(ctx);
                    return;
                }
                action_menu::Action::CopyGatewayUrl => {
                    self.copy_gateway_url(ctx);
                    return;
                }
                _ => {}
            }
            if matches!(
                action,
                action_menu::Action::Start | action_menu::Action::StartService
            ) {
                if let Some(row) = self.selected() {
                    if let Some((name, service)) = row.service {
                        self.intent = Some(Intent::ServiceState {
                            name: name.clone(),
                            service: service.clone(),
                            running: true,
                        });
                        self.open(
                            dialogs::confirm_service_state(&name, &service, true, self.keys[3]),
                            ctx,
                        );
                    } else if let Some(name) = row.instance {
                        self.intent = Some(Intent::Resume {
                            name: name.clone(),
                            template: row.template,
                        });
                        self.open(dialogs::confirm_start(&name), ctx);
                    }
                }
                return;
            }
            self.action(action.index(), ctx);
        }
    }

    fn action(&mut self, index: usize, ctx: &mut EventCtx<Msg>) {
        let row = self.selected();
        self.name.clear();
        match index {
            0 => {
                if let Some(row) = row.filter(|row| !row.informational) {
                    self.intent = None;
                    self.open_details(&row, ctx);
                }
            }
            1 => {
                if let Some(row) = row.filter(|row| row.template_available) {
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
                } else if let Some(row) = row
                    .as_ref()
                    .filter(|row| row.service.is_some() && (row.can_start || row.can_stop))
                {
                    let (name, service) = row.service.clone().expect("service row has a target");
                    let running = !row.can_stop;
                    let modal =
                        dialogs::confirm_service_state(&name, &service, running, self.keys[3]);
                    self.intent = Some(Intent::ServiceState {
                        name,
                        service,
                        running,
                    });
                    self.open(modal, ctx);
                } else if let Some(row) =
                    row.filter(|row| row.instance.is_some() && (row.can_start || row.can_stop))
                {
                    let name = row.instance.expect("instance rows have a name");
                    if row.can_stop {
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
            4 => {
                if self.manual_refresh.is_none() {
                    match self.service.manual_refresh() {
                        Ok(reply) => self.manual_refresh = Some(reply),
                        Err(error) => self.notify(Notification::error("Refresh failed", error)),
                    }
                }
            }
            5 => {
                if let Some(row) = row
                    .as_ref()
                    .filter(|row| row.parent.is_none() && row.template_available)
                {
                    self.intent = Some(Intent::RemoveTemplate(row.template.clone()));
                    self.open(
                        dialogs::confirm_remove_template(&row.template, &row.directory),
                        ctx,
                    );
                }
            }
            7 => {
                if let Some(row) = row.filter(|row| row.can_restart) {
                    let target = row
                        .service
                        .map(|(name, service)| (name, Some(service)))
                        .or_else(|| row.instance.map(|name| (name, None)));
                    if let Some((name, service)) = target {
                        let modal =
                            dialogs::confirm_restart(&name, service.as_deref(), self.keys[7]);
                        self.intent = Some(Intent::Restart { name, service });
                        self.open(modal, ctx);
                    }
                }
            }
            8 => self.confirm_stop_all(ctx),
            9 => self.confirm_purge_all(ctx),
            10 => {
                if !self.open_gateway(ctx) {
                    self.open_workspace(ctx);
                }
            }
            6 => {
                if let Some(row) = row.as_ref().filter(|row| row.parent.is_none()) {
                    self.intent = Some(Intent::DeleteTemplate(row.template.clone()));
                    self.open(dialogs::confirm_delete_template(&row.template), ctx);
                } else if let Some(name) = row.and_then(|row| row.cleanup_target.or(row.instance)) {
                    self.intent = Some(Intent::Purge(name.clone()));
                    self.open(dialogs::confirm_purge(&name), ctx);
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

    fn copy_selected_name(&self, ctx: &mut EventCtx<Msg>) -> bool {
        let Some(value) = self.selected().and_then(|row| row.name_value()) else {
            return false;
        };
        ctx.copy_to_clipboard(value);
        true
    }

    fn copy_gateway_url(&self, ctx: &mut EventCtx<Msg>) -> bool {
        let Some(value) = self.selected().and_then(|row| row.gateway_url) else {
            return false;
        };
        ctx.copy_to_clipboard(value);
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

    fn returns_to_data_view(event: &TuiEvent) -> bool {
        let TuiEvent::Key(key) = event else {
            return false;
        };
        KeySpec::key(tuicore::Key::Esc).matches(*key)
            || KeySpec::key_with_modifiers(tuicore::Key::Char('['), tuicore::KeyModifiers::CONTROL)
                .matches(*key)
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
        if matches!(event, TuiEvent::Yank) && self.copy_selected_name(ctx) {
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
        self.notifications.render(frame, area);
    }
    fn event(&mut self, event: &TuiEvent, ctx: &mut EventCtx<Msg>) -> EventOutcome {
        if self.refresh_schedule.event(event, Instant::now()) {
            self.service.poll_environments();
        }
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
        if self.refresh_schedule.event(event, Instant::now()) {
            self.service.poll_environments();
        }
        // Toolbar and status-bar controls own their input, including instance action keys.
        let toolbar_route = route
            .path
            .without_first_if(&tuicore::ChildKey::second())
            .is_some()
            || route
                .path
                .keys()
                .iter()
                .any(|key| key.as_str() == "template-actions");
        let data_view_route = route
            .path
            .keys()
            .iter()
            .any(|key| key.as_str() == "instances");
        if Self::returns_to_data_view(event) && (toolbar_route || data_view_route) {
            ctx.focus(initial_focus());
            ctx.stop_propagation();
            return EventOutcome::Handled;
        }
        if toolbar_route {
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
        if self.refresh_schedule.tick(Instant::now()) {
            self.service.poll_environments();
        }
        let mut changed = self.sync_environment();
        if let Some(reply) = &mut self.settings_save {
            let message = match reply.try_recv() {
                Ok(Ok(_)) => None,
                Ok(Err(error)) => Some(format!("Cannot save workspace command: {error}")),
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
        let mut result = self.view.tick(dt, settings);
        self.poll_manual_refresh();
        result = result.merge(self.notifications.tick(dt, settings));
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
