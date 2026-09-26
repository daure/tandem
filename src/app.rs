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

use crate::{
    service::AppService,
    store::environments::{EnvironmentSnapshot, Operation},
};

mod action_menu;
mod bulk;
mod details;
mod dialogs;
mod instances;
mod opencode;
mod operations;
mod properties;
mod refresh;
mod route_menu;
mod rows;
mod toolbar;
mod yank_menu;
use action_menu::ActionMenu;
use instances::{Instances, SharedState};
use route_menu::{RouteChoice, RouteMenu};
use rows::Row;
use yank_menu::{YankMenu, YankTarget};

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

fn visible_rows(
    snapshot: &EnvironmentSnapshot,
    operations: &[Operation],
    running_only: bool,
) -> Vec<Row> {
    if !running_only {
        return rows::from_snapshot_with_operations(snapshot, operations);
    }
    let mut visible = snapshot.clone();
    visible
        .instances
        .retain(|instance| instance.is_running() || instance.workspace_only);
    let names = visible
        .instances
        .iter()
        .map(|instance| instance.name.clone())
        .collect::<Vec<_>>();
    visible
        .activities
        .retain(|activity| names.contains(&activity.name));
    rows::from_filtered_snapshot_with_operations(&visible, operations, snapshot)
}

pub(crate) fn initial_focus() -> tuicore::FocusRequest {
    tuicore::FocusRequest::Target(FocusId::new(TREE_FOCUS))
}

pub(super) fn open_route_key() -> KeySpec {
    KeySpec::key_with_modifiers(tuicore::Key::Enter, tuicore::KeyModifiers::CONTROL)
}

pub(super) fn open_panel_key() -> KeySpec {
    KeySpec::key_with_modifiers(tuicore::Key::Char(';'), tuicore::KeyModifiers::CONTROL)
}

#[derive(Debug)]
pub(crate) enum Msg {
    Close,
    NameChanged(String),
    DescriptionChanged(String),
    OpenSettings,
    OpenCommandChanged(String),
    CloseCommandChanged(String),
    NewTemplate,
    Refresh,
    StopAll,
    PurgeAll,
    SetRunningOnly(bool),
    SetBranchInstances(bool),
    SetOpencodeIntegration(bool),
    OpenOpencode(String, Option<crate::store::opencode::Pane>),
    SetOpencodeHistory(bool),
    SetAttachedSessionsOnly(bool),
    CopyName,
    CopyDescription,
    CopyWorkspace,
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
    CloseOpencodeSessions(String),
    UpdateDescription(String),
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
type MainContent = Split<Content, StatusBar<Msg>>;
type MenuLayer = DialogLayer<MainContent, ActionMenu>;
type YankLayer = DialogLayer<MenuLayer, YankMenu>;
type RouteLayer = DialogLayer<YankLayer, RouteMenu>;
type View = DialogLayer<RouteLayer, Modal>;

pub(crate) struct App {
    service: AppService,
    snapshot: EnvironmentSnapshot,
    view: View,
    instances: SharedState,
    toolbar_state: toolbar::SharedState,
    running_only: bool,
    keys: [KeySpec; 11],
    refresh_schedule: refresh::RefreshSchedule,
    manual_refresh: Option<tokio::sync::oneshot::Receiver<Result<(), String>>>,
    notifications: ToastRack,
    deletions: Vec<operations::Deletion>,
    container_operations: Vec<crate::store::environments::Operation>,
    intent: Option<Intent>,
    name: String,
    description: String,
    open_command: String,
    settings_save: Option<tokio::sync::oneshot::Receiver<Result<String, String>>>,
    description_save: Option<tokio::sync::oneshot::Receiver<Result<(), String>>>,
    area: Rect,
    details_open: bool,
    opencode_snapshot: crate::store::opencode::Snapshot,
    closing_opencode_panes: Vec<opencode::ClosingPane>,
    opencode_history: bool,
    attached_sessions_only: bool,
    opencode_action: Option<opencode::PendingAction>,
}

pub(crate) fn root(service: AppService) -> App {
    let keys = service.environment_keys();
    let snapshot = service.environment_snapshot();
    let opencode_enabled = service.opencode_enabled();
    let opencode_snapshot = service.opencode_snapshot();
    let mut rows = visible_rows(&snapshot, &[], false);
    if opencode_enabled {
        let owners = snapshot
            .instances
            .iter()
            .map(|instance| (instance.name.clone(), instance.workspace.clone()))
            .collect::<Vec<_>>();
        rows = opencode::attached_rows_for_owners(rows, &opencode_snapshot, &owners, false);
    }
    let instances = instances::state(rows);
    instances::set_attached_sessions_only(&instances, opencode_enabled);
    let toolbar_state = Rc::new(RefCell::new(toolbar::State::from_snapshot(&snapshot)));
    {
        let mut toolbar_state = toolbar_state.borrow_mut();
        toolbar_state.opencode_enabled = opencode_enabled;
        toolbar_state.attached_sessions_only = opencode_enabled;
    }
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
    .action_hotkey("yi", |_| Msg::CopyName)
    .action_hotkey("yd", |_| Msg::CopyDescription)
    .action_hotkey("yw", |_| Msg::CopyWorkspace)
    .action_hotkey("yu", |_| Msg::CopyGatewayUrl);
    for sequence in ["yy", "yi", "yd", "yw", "yu"] {
        content.set_action_hotkey_visible(sequence, false);
    }
    let main = Split::vertical(
        content,
        StatusBar::new()
            .ai_enabled(false)
            .menu_items(STATUS_BAR_MENU_ITEMS)
            .on_custom_menu_item(|id| match id {
                SETTINGS_MENU_ID => Msg::OpenSettings,
                _ => Msg::Close,
            }),
    )
    .constraints(Constraint::Fill(1), Constraint::Length(1));
    let menu = DialogLayer::new(main, ActionMenu::new(keys))
        .active(false)
        .fit_content()
        .fit_content_max(42, 10)
        .child_overlays_use_base_bounds(true);
    let yank = DialogLayer::new(menu, YankMenu::new())
        .active(false)
        .fit_content()
        .fit_content_max(42, 4)
        .child_overlays_use_base_bounds(true);
    let routes = DialogLayer::new(yank, RouteMenu::new())
        .active(false)
        .fit_content()
        .fit_content_max(110, 12)
        .child_overlays_use_base_bounds(true);
    let modal: Modal = Box::new(
        Dialog::<Msg>::new()
            .on_close(|_| Msg::Close)
            .host(Flex::column()),
    );
    let view = DialogLayer::new(routes, modal)
        .active(false)
        .fit_content()
        .fit_content_max(110, 34)
        .backdrop(DialogBackdrop::dim().amount(0.55));
    service.refresh_environments();
    App {
        service,
        snapshot,
        view,
        instances,
        toolbar_state,
        running_only: false,
        keys,
        refresh_schedule: refresh::RefreshSchedule::default(),
        manual_refresh: None,
        notifications: ToastRack::new(),
        deletions: Vec::new(),
        container_operations: Vec::new(),
        intent: None,
        name: String::new(),
        description: String::new(),
        open_command: String::new(),
        settings_save: None,
        description_save: None,
        area: Rect::default(),
        details_open: false,
        opencode_snapshot,
        closing_opencode_panes: Vec::new(),
        opencode_history: false,
        attached_sessions_only: opencode_enabled,
        opencode_action: None,
    }
}

impl App {
    fn update_snapshot(&mut self, snapshot: EnvironmentSnapshot) -> bool {
        if snapshot.loading {
            self.snapshot = snapshot;
            return false;
        }
        let initial_load_completed = self.snapshot.loading;
        let operations = self.service.operations();
        let snapshot_changed = snapshot != self.snapshot;
        let mut opencode_snapshot = self.service.opencode_snapshot();
        opencode::hide_closing_panes(&mut opencode_snapshot, &mut self.closing_opencode_panes);
        self.opencode_snapshot = opencode_snapshot;
        if !self.service.opencode_enabled() {
            self.attached_sessions_only = false;
        }
        instances::set_attached_sessions_only(&self.instances, self.attached_sessions_only);
        let rows = self.project_rows(&snapshot, &operations);
        let rows_changed = if initial_load_completed {
            instances::replace_rows_and_select_first(&self.instances, rows);
            true
        } else {
            instances::replace_rows(&self.instances, rows)
        };
        self.toolbar_state.borrow_mut().opencode_enabled = self.service.opencode_enabled();
        self.toolbar_state.borrow_mut().attached_sessions_only = self.attached_sessions_only;
        if !snapshot_changed {
            return rows_changed;
        }
        let mut toolbar_state = toolbar::State::from_snapshot(&snapshot);
        toolbar_state.running_only = self.running_only;
        toolbar_state.opencode_enabled = self.service.opencode_enabled();
        toolbar_state.show_saved = self.opencode_history;
        toolbar_state.attached_sessions_only = self.attached_sessions_only;
        *self.toolbar_state.borrow_mut() = toolbar_state;
        self.snapshot = snapshot;
        true
    }

    fn selected(&self) -> Option<Row> {
        instances::selected(&self.instances)
    }

    fn menu_layer(&self) -> &MenuLayer {
        self.view.base().base().base()
    }

    fn menu_layer_mut(&mut self) -> &mut MenuLayer {
        self.view.base_mut().base_mut().base_mut()
    }

    fn yank_layer(&self) -> &YankLayer {
        self.view.base().base()
    }

    fn yank_layer_mut(&mut self) -> &mut YankLayer {
        self.view.base_mut().base_mut()
    }

    fn route_layer(&self) -> &RouteLayer {
        self.view.base()
    }

    fn route_layer_mut(&mut self) -> &mut RouteLayer {
        self.view.base_mut()
    }

    fn transient_menu_active(&self) -> bool {
        self.route_layer().is_active()
            || self.yank_layer().is_active()
            || self.menu_layer().is_active()
    }

    fn transient_menu_event(&mut self, event: &TuiEvent, ctx: &mut EventCtx<Msg>) -> EventOutcome {
        if self.route_layer().is_active() {
            self.route_layer_mut().event(event, ctx)
        } else if self.yank_layer().is_active() {
            self.yank_layer_mut().event(event, ctx)
        } else {
            self.menu_layer_mut().event(event, ctx)
        }
    }

    #[cfg(test)]
    fn set_rows_for_tests(&mut self, rows: Vec<Row>) {
        let highlighted = rows.first().map(|row| row.id.clone());
        instances::replace_rows(&self.instances, rows);
        instances::set_highlighted(&self.instances, highlighted);
    }

    #[cfg(test)]
    fn status_bar_mut(&mut self) -> &mut StatusBar<Msg> {
        self.view
            .base_mut()
            .base_mut()
            .base_mut()
            .base_mut()
            .second_mut()
    }

    fn set_running_only(&mut self, running_only: bool, ctx: &mut EventCtx<Msg>) {
        if self.running_only == running_only {
            return;
        }
        self.running_only = running_only;
        self.toolbar_state.borrow_mut().running_only = running_only;
        let operations = self.service.operations();
        let rows = self.project_rows(&self.snapshot, &operations);
        instances::replace_rows(&self.instances, rows);
        instances::request_center_highlighted(&self.instances);
        ctx.request_layout();
        ctx.request_redraw();
    }

    pub(crate) fn handle_message(&mut self, message: Msg, ctx: &mut EventCtx<Msg>) {
        match message {
            Msg::SetOpencodeIntegration(enabled) => {
                match self.service.set_opencode_enabled(enabled) {
                    Ok(reply) => self.settings_save = Some(reply),
                    Err(error) => ctx.notify(Notification::error("Cannot save settings", error)),
                }
            }
            Msg::OpenOpencode(id, pane) => self.submit_opencode(&id, pane, ctx),
            Msg::SetOpencodeHistory(show_saved) => {
                if self.service.opencode_enabled() {
                    self.opencode_history = show_saved;
                    self.toolbar_state.borrow_mut().show_saved = show_saved;
                    self.update_snapshot(self.snapshot.clone());
                    instances::request_center_highlighted(&self.instances);
                    ctx.request_layout();
                }
            }
            Msg::SetAttachedSessionsOnly(enabled) => {
                if self.service.opencode_enabled() {
                    self.attached_sessions_only = enabled;
                    self.update_snapshot(self.snapshot.clone());
                    instances::request_center_highlighted(&self.instances);
                    ctx.request_layout();
                    ctx.request_redraw();
                }
            }
            Msg::Close => {
                self.service.cancel_opencode_conversation();
                self.settings_save = None;
                self.view.set_active_with_context(false, ctx);
                ctx.focus(initial_focus());
                self.intent = None;
                self.details_open = false;
            }
            Msg::NameChanged(name) => self.name = name,
            Msg::DescriptionChanged(description) => self.description = description,
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
            Msg::SetRunningOnly(running_only) => self.set_running_only(running_only, ctx),
            Msg::SetBranchInstances(enabled) => {
                if let Err(error) = self.service.set_branch_instances(enabled) {
                    ctx.notify(Notification::error("Cannot save settings", error));
                }
            }
            Msg::CopyName => {
                self.copy_selected_name(ctx);
            }
            Msg::CopyDescription => {
                self.copy_selected_description(ctx);
            }
            Msg::CopyWorkspace => {
                self.copy_selected_workspace(ctx);
            }
            Msg::CopyGatewayUrl => {
                self.copy_gateway_url(ctx);
            }
            Msg::Submit => {
                if self.target_instance_has_operation() {
                    self.block_operation(ctx);
                    return;
                }
                if let Some(Intent::CloseOpencodeSessions(name)) = &self.intent {
                    self.submit_close_instance_opencode(name.clone(), ctx);
                    return;
                }
                if let Some(Intent::UpdateDescription(name)) = &self.intent {
                    self.description_save = Some(
                        self.service
                            .update_instance_description(name.clone(), self.description.clone()),
                    );
                    self.view.set_active_with_context(false, ctx);
                    ctx.focus(initial_focus());
                    self.intent = None;
                    self.details_open = false;
                    ctx.request_redraw();
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
                        match self.service.submit_new_instance(
                            &self.name,
                            template.clone(),
                            self.description.clone(),
                        ) {
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
                    Some(Intent::CloseOpencodeSessions(_)) => {
                        unreachable!("OpenCode sessions are handled before operations")
                    }
                    Some(Intent::UpdateDescription(_)) => {
                        unreachable!("description updates are handled before operations")
                    }
                    None => return,
                };
                match result {
                    Ok(operation) => {
                        self.operation_accepted(operation);
                        ctx.request_layout();
                        self.view.set_active_with_context(false, ctx);
                        ctx.focus(initial_focus());
                        self.intent = None;
                        self.details_open = false;
                    }
                    Err(error) => {
                        self.view.layer_mut().set_bottom_left(error);
                    }
                }
            }
        }
        ctx.request_redraw();
    }

    fn open(&mut self, modal: Modal, ctx: &mut EventCtx<Msg>) {
        if !matches!(self.intent, Some(Intent::CreateInstance(_)))
            && self.target_instance_has_operation()
        {
            self.block_operation(ctx);
            return;
        }
        self.details_open = false;
        self.view.replace_layer(modal, ctx);
        self.view.set_fit_content(true);
        self.view.set_fit_content_max(110, 34);
        self.view.set_placement(DialogLayerPlacement::Center);
        self.view.set_active_with_context(true, ctx);
        if matches!(
            self.intent,
            Some(Intent::CreateInstance(_) | Intent::NewTemplate)
        ) {
            ctx.focus(tuicore::FocusRequest::Path(tuicore::TreePath::from_keys([
                tuicore::ChildKey::second(),
                tuicore::ChildKey::body(),
                tuicore::ChildKey::new("name"),
            ])));
        } else if matches!(self.intent, Some(Intent::UpdateDescription(_))) {
            ctx.focus(tuicore::FocusRequest::Path(tuicore::TreePath::from_keys([
                tuicore::ChildKey::second(),
                tuicore::ChildKey::body(),
                tuicore::ChildKey::new("description"),
            ])));
        }
    }

    fn target_instance_has_operation(&self) -> bool {
        let Some(name) = self.intent_instance_target() else {
            return false;
        };
        self.instance_has_operation(name)
    }

    fn instance_has_operation(&self, name: &str) -> bool {
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
            Intent::UpdateDescription(_)
            | Intent::CloseOpencodeSessions(_)
            | Intent::NewTemplate
            | Intent::StopTemplate(_)
            | Intent::DeleteTemplate(_)
            | Intent::RemoveTemplate(_)
            | Intent::StopAll(_)
            | Intent::PurgeAll(_) => None,
        }
    }

    fn block_operation(&mut self, ctx: &mut EventCtx<Msg>) {
        self.intent = None;
        self.view.set_active_with_context(false, ctx);
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
                dialogs::instance_entry(
                    "New instance",
                    &self.name,
                    &self.description,
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

    fn open_description_editor(&mut self, ctx: &mut EventCtx<Msg>) -> bool {
        if self.description_save.is_some() {
            self.notify(Notification::info(
                "Description update in progress",
                "Wait for the current description update to finish.",
            ));
            return true;
        }
        let Some(row) = self
            .selected()
            .filter(|row| row.instance.is_some() && row.service.is_none())
        else {
            return false;
        };
        let name = row.instance.expect("instance rows have a name");
        self.description = row.description;
        self.intent = Some(Intent::UpdateDescription(name));
        self.open(dialogs::description_entry(&self.description), ctx);
        true
    }

    fn open_settings(&mut self, ctx: &mut EventCtx<Msg>) {
        self.intent = None;
        self.settings_save = None;
        self.open_command = self.service.open_command();
        self.open(
            dialogs::settings(
                self.service.branch_instances(),
                self.service.opencode_enabled(),
                &self.open_command,
                &self.service.close_command(),
            ),
            ctx,
        );
    }

    fn open_details(&mut self, row: &Row, ctx: &mut EventCtx<Msg>) {
        self.view.replace_layer(dialogs::details(row), ctx);
        self.details_open = true;
        self.resize_details_dialog();
        self.view.set_active_with_context(true, ctx);
    }

    fn resize_details_dialog(&mut self) {
        let dock = DockSpec::bottom(80).cross_percent(details_width_percent(self.area.width));
        let layer = &mut self.view;
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
                template: row.is_template(),
                instance: row.instance.is_some(),
                service: row.service_name.is_some(),
                capabilities: (row.can_start, row.can_stop, row.can_restart),
                gateway: row.gateway_url.is_some(),
                template_available: row.template_available,
                repository: row.checkout_path.is_some(),
                cleanup: row.cleanup_target.is_some(),
                opencode_session: row
                    .opencode
                    .as_ref()
                    .and_then(opencode::Target::session_action),
                close_opencode: row
                    .opencode
                    .as_ref()
                    .is_some_and(opencode::Target::closeable),
                external_opencode: row
                    .opencode
                    .as_ref()
                    .is_some_and(opencode::Target::external_observation),
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
                action_menu::Action::Yank => {
                    self.open_yank_menu(ctx);
                    return;
                }
                action_menu::Action::OpenBrowser => {
                    self.open_gateway(ctx);
                    return;
                }
                action_menu::Action::OpenCommand => {
                    self.open_workspace(ctx);
                    return;
                }
                action_menu::Action::OpenPanel | action_menu::Action::GotoPanel => {
                    if let Some(row) = self.selected() {
                        self.activate_opencode(&row, ctx);
                    }
                    return;
                }
                action_menu::Action::CloseSession => {
                    if let Some(row) = self.selected() {
                        self.close_opencode(&row, ctx);
                    }
                    return;
                }
                action_menu::Action::UpdateDescription => {
                    self.open_description_editor(ctx);
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

    fn yank_requested(event: &TuiEvent) -> bool {
        matches!(event, TuiEvent::Yank)
            || matches!(event, TuiEvent::Key(key) if key.code == tuicore::Key::Char('y') && key.modifiers == tuicore::KeyModifiers::NONE)
            || matches!(event, TuiEvent::Hotkey(tuicore::HotkeyEvent::Pending(sequence)) if sequence == "y")
    }

    fn open_yank_menu(&mut self, ctx: &mut EventCtx<Msg>) -> bool {
        let Some(row) = self.selected() else {
            return false;
        };
        let target = if let Some(url) = row.gateway_url {
            YankTarget::Service { url }
        } else if row.service.is_none() {
            let (Some(name), Some(workspace)) = (row.instance, row.workspace) else {
                return false;
            };
            YankTarget::Instance {
                name,
                description: row.description,
                workspace,
            }
        } else {
            return false;
        };
        let menu = self.yank_layer_mut();
        menu.layer_mut().open(target, ctx);
        menu.set_active_with_context(true, ctx);
        true
    }

    fn drain_yank_menu(&mut self, ctx: &mut EventCtx<Msg>) {
        if !self.yank_layer().is_active() {
            return;
        }
        let selection = self.yank_layer_mut().layer_mut().take_selection();
        if selection.is_none() && self.yank_layer().layer().is_open() {
            return;
        }
        self.yank_layer_mut().set_active_with_context(false, ctx);
        if let Some((action, target)) = selection
            && let Some(value) = action.text(&target)
        {
            ctx.copy_to_clipboard(value);
        }
        ctx.focus(initial_focus());
    }

    fn selected_instance_routes(&self) -> Vec<RouteChoice> {
        let Some(name) = self
            .selected()
            .filter(|row| row.service.is_none())
            .and_then(|row| row.instance)
        else {
            return Vec::new();
        };
        self.snapshot
            .instances
            .iter()
            .find(|instance| instance.name == name)
            .into_iter()
            .flat_map(|instance| &instance.services)
            .filter_map(|service| {
                service
                    .url
                    .as_ref()
                    .filter(|url| !url.trim().is_empty())
                    .map(|url| RouteChoice {
                        service: service.name.clone(),
                        url: url.clone(),
                    })
            })
            .collect()
    }

    fn open_selected_route(&mut self, ctx: &mut EventCtx<Msg>) -> bool {
        if self.open_gateway(ctx) {
            return true;
        }
        let routes = self.selected_instance_routes();
        if routes.is_empty() {
            return false;
        }
        if routes.len() == 1 {
            self.open_gateway_url(&routes[0].url, ctx);
            return true;
        }
        let menu = self.route_layer_mut();
        menu.layer_mut().open(routes, ctx);
        menu.set_active_with_context(true, ctx);
        true
    }

    fn drain_route_menu(&mut self, ctx: &mut EventCtx<Msg>) {
        if !self.route_layer().is_active() {
            return;
        }
        let route = self.route_layer_mut().layer_mut().take_selection();
        if route.is_none() && self.route_layer().layer().is_open() {
            return;
        }
        self.route_layer_mut().set_active_with_context(false, ctx);
        if let Some(route) = route {
            self.open_gateway_url(&route.url, ctx);
        }
        ctx.focus(initial_focus());
    }

    fn action(&mut self, index: usize, ctx: &mut EventCtx<Msg>) {
        let row = self
            .selected()
            .filter(|row| index == 0 || row.opencode.is_none());
        self.name.clear();
        self.description.clear();
        match index {
            0 => {
                if let Some(row) = row.filter(|row| !row.informational) {
                    if self.open_opencode_dialog(&row, ctx) {
                        return;
                    }
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
                if let Some(name) = row.as_ref().and_then(|row| row.instance.as_ref())
                    && self.instance_has_operation(name)
                {
                    self.intent = Some(Intent::Stop(name.clone()));
                    self.block_operation(ctx);
                    return;
                }
                if let Some(row) = row.as_ref().filter(|row| row.is_template()) {
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
                    .filter(|row| row.is_template() && row.template_available)
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
                self.open_workspace(ctx);
            }
            6 => {
                if let Some(row) = row.as_ref().filter(|row| row.is_template()) {
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
        self.open_gateway_url(&url, ctx);
        true
    }

    fn open_gateway_url(&self, url: &str, ctx: &mut EventCtx<Msg>) {
        if let Err(error) = self.service.open_gateway(url) {
            ctx.notify(Notification::error("Cannot open gateway", error));
        }
    }

    fn copy_selected_name(&self, ctx: &mut EventCtx<Msg>) -> bool {
        let Some(value) = self.selected().and_then(|row| row.name_value()) else {
            return false;
        };
        ctx.copy_to_clipboard(value);
        true
    }

    fn copy_selected_description(&self, ctx: &mut EventCtx<Msg>) -> bool {
        let Some(value) = self
            .selected()
            .filter(|row| row.instance.is_some() && row.service.is_none())
            .map(|row| row.description)
        else {
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

    fn copy_selected_workspace(&self, ctx: &mut EventCtx<Msg>) -> bool {
        let Some(value) = self
            .selected()
            .filter(|row| row.service.is_none())
            .and_then(|row| row.workspace)
        else {
            return false;
        };
        ctx.copy_to_clipboard(value);
        true
    }

    fn open_workspace(&self, ctx: &mut EventCtx<Msg>) -> bool {
        let Some((workspace, instance)) = self
            .selected()
            .filter(|row| row.service.is_none())
            .and_then(|row| Some((row.workspace?, row.instance?)))
        else {
            return false;
        };
        if let Err(error) = self.service.open_workspace(&workspace, &instance) {
            ctx.notify(Notification::error("Cannot open workspace", error));
        }
        true
    }

    fn confirm_close_instance_opencode(&mut self, row: &Row, ctx: &mut EventCtx<Msg>) -> bool {
        if row.opencode.is_some() {
            return false;
        }
        let Some(name) = row.instance.clone().or_else(|| {
            row.id
                .strip_prefix("sessions:")
                .filter(|name| !name.is_empty())
                .map(str::to_owned)
        }) else {
            return false;
        };
        self.intent = Some(Intent::CloseOpencodeSessions(name.clone()));
        self.open(dialogs::confirm_close_opencode_sessions(&name), ctx);
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

    fn show_agent_overview(&mut self, event: &TuiEvent, ctx: &mut EventCtx<Msg>) {
        if !matches!(event, TuiEvent::Hotkey(tuicore::HotkeyEvent::Commit(sequence)) if sequence == "shift+h")
        {
            return;
        }
        self.running_only = false;
        self.opencode_history = false;
        self.attached_sessions_only = true;
        {
            let mut toolbar = self.toolbar_state.borrow_mut();
            toolbar.running_only = false;
            toolbar.show_saved = false;
            toolbar.attached_sessions_only = true;
        }
        instances::set_attached_sessions_only(&self.instances, true);
        let operations = self.service.operations();
        instances::replace_rows(
            &self.instances,
            self.project_rows(&self.snapshot, &operations),
        );
        ctx.request_layout();
        ctx.request_redraw();
    }

    fn handle_key(&mut self, event: &TuiEvent, ctx: &mut EventCtx<Msg>) -> bool {
        if self.view.is_active()
            && matches!(
                self.intent,
                Some(
                    Intent::CreateInstance(_) | Intent::NewTemplate | Intent::UpdateDescription(_)
                )
            )
            && let TuiEvent::Key(key) = event
            && (KeySpec::key_with_modifiers(tuicore::Key::Enter, tuicore::KeyModifiers::CONTROL)
                .matches(*key)
                || matches!(self.intent, Some(Intent::NewTemplate))
                    && KeySpec::key(tuicore::Key::Enter).matches(*key))
        {
            self.handle_message(Msg::Submit, ctx);
            ctx.stop_propagation();
            return true;
        }
        if self.transient_menu_active() || self.view.is_active() {
            return false;
        }
        if instances::is_searching(&self.instances) {
            return false;
        }
        if let TuiEvent::Key(key) = event
            && KeySpec::shifted('a').matches(*key)
            && self.service.opencode_enabled()
        {
            self.handle_message(
                Msg::SetAttachedSessionsOnly(!self.attached_sessions_only),
                ctx,
            );
            ctx.stop_propagation();
            return true;
        }
        if let TuiEvent::Key(key) = event
            && KeySpec::shifted('u').matches(*key)
        {
            self.set_running_only(!self.running_only, ctx);
            ctx.stop_propagation();
            return true;
        }
        if let TuiEvent::Key(key) = event
            && KeySpec::shifted('o').matches(*key)
            && self.service.opencode_enabled()
        {
            self.handle_message(Msg::SetOpencodeHistory(!self.opencode_history), ctx);
            ctx.stop_propagation();
            return true;
        }
        if let TuiEvent::Key(key) = event
            && KeySpec::plain('c').matches(*key)
            && let Some(row) = self.selected()
            && (self.confirm_close_instance_opencode(&row, ctx) || self.close_opencode(&row, ctx))
        {
            ctx.stop_propagation();
            return true;
        }
        if let TuiEvent::Key(key) = event
            && KeySpec::plain('d').matches(*key)
            && self.open_description_editor(ctx)
        {
            ctx.stop_propagation();
            return true;
        }
        if let TuiEvent::Key(key) = event
            && open_route_key().matches(*key)
            && self.open_selected_route(ctx)
        {
            ctx.stop_propagation();
            return true;
        }
        if let TuiEvent::Key(key) = event
            && open_panel_key().matches(*key)
            && let Some(row) = self.selected()
            && self.activate_opencode(&row, ctx)
        {
            ctx.stop_propagation();
            return true;
        }
        if Self::yank_requested(event) && self.open_yank_menu(ctx) {
            ctx.stop_propagation();
            return true;
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
        self.drain_yank_menu(ctx);
        self.drain_route_menu(ctx);
        ctx.request_redraw();
    }

    fn poll_description_save(&mut self) -> bool {
        let result = match self.description_save.as_mut() {
            Some(reply) => match reply.try_recv() {
                Ok(result) => result,
                Err(tokio::sync::oneshot::error::TryRecvError::Closed) => {
                    Err("description worker stopped".into())
                }
                Err(tokio::sync::oneshot::error::TryRecvError::Empty) => return false,
            },
            None => return false,
        };
        self.description_save = None;
        match result {
            Ok(()) => self.sync_environment(),
            Err(error) => {
                self.notify(Notification::error("Cannot update description", error));
                true
            }
        }
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
        self.show_agent_overview(event, ctx);
        if self.handle_key(event, ctx) {
            return EventOutcome::Handled;
        }
        let outcome = if self.transient_menu_active() {
            self.transient_menu_event(event, ctx)
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
        self.show_agent_overview(event, ctx);
        // Toolbar and status-bar controls own their input, including instance action keys.
        let status_bar_route = route.path.keys().starts_with(&[
            tuicore::ChildKey::first(),
            tuicore::ChildKey::first(),
            tuicore::ChildKey::first(),
            tuicore::ChildKey::first(),
            tuicore::ChildKey::second(),
        ]);
        let toolbar_route = status_bar_route
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
        if Self::returns_to_data_view(event) && toolbar_route {
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
        let outcome = if self.transient_menu_active() {
            self.transient_menu_event(event, ctx)
        } else {
            self.view.dispatch_event(route, event, ctx)
        };
        self.after_event(ctx);
        if Self::returns_to_data_view(event) && data_view_route && outcome == EventOutcome::Ignored
        {
            ctx.focus(initial_focus());
            ctx.stop_propagation();
            return EventOutcome::Handled;
        }
        outcome
    }
    fn tick(&mut self, dt: Duration, settings: AnimationSettings) -> TickResult {
        self.service.poll_opencode();
        self.poll_opencode_action();
        if self.refresh_schedule.tick(Instant::now()) {
            self.service.poll_environments();
        }
        let mut changed = self.sync_environment();
        changed |= self.poll_description_save();
        if let Some(reply) = &mut self.settings_save {
            let result = match reply.try_recv() {
                Ok(result) => Some(result),
                Err(tokio::sync::oneshot::error::TryRecvError::Closed) => {
                    Some(Err("Settings worker stopped".to_owned()))
                }
                Err(tokio::sync::oneshot::error::TryRecvError::Empty) => None,
            };
            if let Some(result) = result {
                self.settings_save = None;
                if let Err(error) = result {
                    self.view
                        .layer_mut()
                        .set_bottom_left(format!("Cannot save settings: {error}"));
                }
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
    if width < MOBILE_TABS_WIDTH { 100 } else { 75 }
}

#[cfg(test)]
mod tests;
