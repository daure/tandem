use std::{
    cell::RefCell,
    collections::HashSet,
    rc::Rc,
    time::{Duration, Instant},
};

use ratatui::{
    Frame,
    layout::{Constraint, Rect},
};
use tuicore::{
    AnimationSettings, Dialog, DialogBackdrop, DialogHost, DialogLayer, DialogLayerPlacement,
    DockChrome, DockSpec, EventCtx, EventOutcome, EventRoute, Flex, FocusCtx, FocusId, FocusTarget,
    KeySpec, LayoutCtx, LayoutProposal, LayoutResult, LayoutSizeHint, LifecycleCtx, Notification,
    RenderCtx, Split, StatusBar, StatusBarMenuItem, Tab, Tabs, TabsVariant, TickResult, ToastRack,
    TuiEvent, TuiNode,
};

use crate::{
    service::AppService,
    store::environments::{EnvironmentSnapshot, Operation},
};

mod action_menu;
mod bulk;
mod creation;
mod details;
mod dialogs;
mod instances;
mod opencode;
mod operations;
mod overview;
mod properties;
mod refresh;
mod route_menu;
mod rows;
mod toolbar;
mod yank_menu;
use action_menu::ActionMenu;
#[cfg(test)]
use instances::Instances;
use instances::SharedState;
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
    opencode_snapshot: &crate::store::opencode::Snapshot,
    running_only: bool,
) -> Vec<Row> {
    if !running_only {
        return rows::from_snapshot_with_operations(snapshot, operations);
    }
    let opencode_instances = opencode_snapshot
        .sessions
        .iter()
        .filter(|session| session.live() && !session.stale)
        .map(|session| session.directory.as_str())
        .chain(
            opencode_snapshot
                .clients
                .iter()
                .filter(|client| !client.stale)
                .map(|client| client.directory.as_str()),
        )
        .filter_map(|directory| {
            crate::store::opencode::workspace_owner(
                directory,
                snapshot
                    .instances
                    .iter()
                    .map(|instance| (instance.name.as_str(), instance.workspace.as_str())),
            )
        })
        .collect::<HashSet<_>>();
    let mut visible = snapshot.clone();
    visible.instances.retain(|instance| {
        instance.is_running()
            || instance.is_starting()
            || instance.status_summary().status == crate::store::environments::Status::Stopping
            || opencode_instances.contains(instance.name.as_str())
    });
    let directories = visible
        .instances
        .iter()
        .map(|instance| instance.template_directory.as_str())
        .collect::<HashSet<_>>();
    visible
        .templates
        .retain(|template| directories.contains(template.directory.as_str()));
    let names = visible
        .instances
        .iter()
        .map(|instance| instance.name.as_str())
        .collect::<HashSet<_>>();
    visible
        .activities
        .retain(|activity| names.contains(activity.name.as_str()));
    rows::from_filtered_snapshot_with_operations(&visible, operations, snapshot)
}

pub(crate) fn initial_focus() -> tuicore::FocusRequest {
    tuicore::FocusRequest::Target(FocusId::new(TREE_FOCUS))
}

pub(super) fn open_route_key() -> KeySpec {
    KeySpec::key_with_modifiers(tuicore::Key::Enter, tuicore::KeyModifiers::CONTROL)
}

pub(super) fn open_panel_key() -> KeySpec {
    KeySpec::plain('o')
}

#[derive(Debug)]
pub(crate) enum Msg {
    Close,
    NameChanged(String),
    DescriptionChanged(String),
    InitialPromptChanged(String),
    OpenSettings,
    CompletionFadeChanged(String),
    CompletionSoundSelected(String),
    NewTemplate,
    Refresh,
    StopAll,
    PurgeAll,
    SetRunningOnly(bool),
    SetBranchInstances(bool),
    SetOpencodeIntegration(bool),
    SetClearOpencodeHistory(bool),
    OpenOpencode(String, Option<crate::store::opencode::Pane>),
    SetOpencodeHistory(bool),
    SetAttachedSessionsOnly(bool),
    SetCompletionSound(bool),
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
    CloseOpencodeSessions(crate::store::opencode::CloseScope),
    UpdateDescription(String),
    Purge(String),
    StopTemplate(String),
    DeleteTemplate(String),
    RemoveTemplate(String),
    StopAll(Vec<String>),
    PurgeAll(Vec<String>),
}

type Content = Split<Tabs<Msg>, overview::Pages>;
trait ModalNode: TuiNode<Msg> + DockChrome {
    fn set_bottom_left(&mut self, _title: String) {}
    fn updated_details(&self, _previous: &Row, _row: &Row) -> Option<Modal> {
        None
    }
}

impl ModalNode for DialogHost<Flex<Msg>, Msg> {
    fn set_bottom_left(&mut self, title: String) {
        self.dialog_mut().set_bottom_left(title);
    }
}

impl ModalNode for Tabs<Msg> {
    fn updated_details(&self, previous: &Row, row: &Row) -> Option<Modal> {
        Some(dialogs::updated_details(
            self.selected_index(),
            previous,
            row,
        ))
    }
}

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
    // Action handlers address the active page; both pages retain their own state.
    instances: SharedState,
    toolbar_state: toolbar::SharedState,
    running_only: bool,
    keys: [KeySpec; 10],
    refresh_schedule: refresh::RefreshSchedule,
    manual_refresh: Option<tokio::sync::oneshot::Receiver<Result<(), String>>>,
    notifications: ToastRack,
    deletions: Vec<operations::Deletion>,
    container_operations: Vec<crate::store::environments::Operation>,
    intent: Option<Intent>,
    name: String,
    description: String,
    creation: creation::Creation,
    settings_sound_choice: Rc<RefCell<Option<String>>>,
    settings_save: Option<tokio::sync::oneshot::Receiver<Result<String, String>>>,
    description_save: Option<tokio::sync::oneshot::Receiver<Result<(), String>>>,
    area: Rect,
    details_open: bool,
    template_details: Option<Row>,
    details_layout_pending: bool,
    opencode_snapshot: crate::store::opencode::Snapshot,
    closing_opencode_panes: Vec<opencode::ClosingPane>,
    opencode_history: bool,
    attached_sessions_only: bool,
    completion_sound: bool,
    opencode_action: Option<opencode::PendingAction>,
}

pub(crate) fn root(service: AppService) -> App {
    #[cfg(test)]
    tests::init_ui();
    tuicore::set_keybindings(tuicore::keybindings().with_tabs_close([KeySpec::plain('x')]));
    let keys = service.environment_keys();
    let snapshot = service.environment_snapshot();
    let opencode_enabled = service.opencode_enabled();
    let opencode_snapshot = service.opencode_snapshot();
    let states = [false, true].map(|sessions| {
        let state = instances::state(opencode::project_rows(
            &snapshot,
            &[],
            &opencode_snapshot,
            true,
            false,
            sessions,
        ));
        instances::set_completion_fade(&state, service.completion_fade_seconds());
        instances::set_attached_sessions_only(&state, sessions);
        state
    });
    let instances = states[usize::from(opencode_enabled)].clone();
    let toolbar_state = Rc::new(RefCell::new(toolbar::State::from_snapshots(
        &snapshot,
        &opencode_snapshot,
    )));
    {
        let mut toolbar_state = toolbar_state.borrow_mut();
        toolbar_state.opencode_enabled = opencode_enabled;
        toolbar_state.completion_sound = false;
    }
    let content = Split::vertical(
        overview_tabs(opencode_enabled, opencode_enabled),
        overview::Pages::new(keys, toolbar_state.clone(), states, opencode_enabled),
    )
    .constraints(Constraint::Length(1), Constraint::Fill(1));
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
        .fit_content_max(42, action_menu::MENU_HEIGHT)
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
        running_only: true,
        keys,
        refresh_schedule: refresh::RefreshSchedule::default(),
        manual_refresh: None,
        notifications: ToastRack::new(),
        deletions: Vec::new(),
        container_operations: Vec::new(),
        intent: None,
        name: String::new(),
        description: String::new(),
        creation: creation::Creation::default(),
        settings_sound_choice: Rc::new(RefCell::new(None)),
        settings_save: None,
        description_save: None,
        area: Rect::default(),
        details_open: false,
        template_details: None,
        details_layout_pending: false,
        opencode_snapshot,
        closing_opencode_panes: Vec::new(),
        opencode_history: false,
        attached_sessions_only: opencode_enabled,
        completion_sound: false,
        opencode_action: None,
    }
}

impl App {
    fn update_snapshot(&mut self, snapshot: EnvironmentSnapshot) -> bool {
        let opencode_enabled = self.service.opencode_enabled();
        let tabs_changed = self.toolbar_state.borrow().opencode_enabled != opencode_enabled;
        if tabs_changed {
            self.select_overview(self.attached_sessions_only && opencode_enabled);
            *self.tabs_mut() = overview_tabs(opencode_enabled, self.attached_sessions_only);
            self.toolbar_state.borrow_mut().opencode_enabled = opencode_enabled;
        }
        if snapshot.loading {
            self.snapshot = snapshot;
            return tabs_changed;
        }
        let initial_load_completed = self.snapshot.loading;
        let operations = self.service.operations();
        let snapshot_changed = snapshot != self.snapshot;
        if snapshot.templates != self.snapshot.templates {
            self.update_template_details(&snapshot);
        }
        let mut opencode_snapshot = self.service.opencode_snapshot();
        for deletion in &self.deletions {
            deletion.hide_opencode(&mut opencode_snapshot);
        }
        opencode::hide_closing_panes(&mut opencode_snapshot, &mut self.closing_opencode_panes);
        if self.completion_sound && opencode_snapshot.completed_since(&self.opencode_snapshot) {
            self.service.play_completion_sound();
        }
        self.opencode_snapshot = opencode_snapshot;
        let rows_changed =
            self.update_overview_rows(&snapshot, &operations, initial_load_completed);
        let mut toolbar_state = toolbar::State::from_snapshots(&snapshot, &self.opencode_snapshot);
        let totals_changed = toolbar_state.totals != self.toolbar_state.borrow().totals;
        toolbar_state.running_only = self.running_only;
        toolbar_state.opencode_enabled = opencode_enabled;
        toolbar_state.show_saved = self.opencode_history;
        toolbar_state.completion_sound = self.completion_sound;
        *self.toolbar_state.borrow_mut() = toolbar_state;
        self.snapshot = snapshot;
        snapshot_changed || rows_changed || totals_changed || tabs_changed
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

    fn tabs_mut(&mut self) -> &mut Tabs<Msg> {
        self.menu_layer_mut().base_mut().first_mut().first_mut()
    }

    fn pages_mut(&mut self) -> &mut overview::Pages {
        self.menu_layer_mut().base_mut().first_mut().second_mut()
    }

    fn select_overview(&mut self, sessions: bool) {
        self.attached_sessions_only = sessions;
        self.instances = self.pages_mut().select(sessions);
    }

    fn update_overview_rows(
        &mut self,
        snapshot: &EnvironmentSnapshot,
        operations: &[Operation],
        select_first: bool,
    ) -> bool {
        let mut changed = false;
        for (index, state) in self.pages_mut().states().iter().enumerate() {
            let rows = opencode::project_rows(
                snapshot,
                operations,
                &self.opencode_snapshot,
                self.running_only,
                self.opencode_history,
                index == 1,
            );
            if select_first {
                instances::replace_rows_and_select_first(state, rows);
                changed = true;
            } else {
                changed |= instances::replace_rows(state, rows);
            }
        }
        changed
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
        self.update_overview_rows(&self.snapshot.clone(), &operations, false);
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
                let opencode_enabled = self.service.opencode_enabled();
                let enabled = enabled && opencode_enabled;
                self.select_overview(enabled);
                self.tabs_mut().select_index_with_settings(
                    usize::from(opencode_enabled && !enabled),
                    ctx.animation(),
                );
                self.update_snapshot(self.snapshot.clone());
                ctx.request_layout();
                ctx.request_redraw();
            }
            Msg::SetCompletionSound(enabled) => {
                self.completion_sound = enabled;
                self.toolbar_state.borrow_mut().completion_sound = enabled;
                ctx.request_redraw();
            }
            Msg::Close => {
                self.service.cancel_opencode_conversation();
                self.settings_save = None;
                self.view.set_active_with_context(false, ctx);
                ctx.focus(initial_focus());
                self.intent = None;
                self.details_open = false;
                self.template_details = None;
            }
            Msg::NameChanged(name) => self.name = name,
            Msg::DescriptionChanged(description) => self.description = description,
            Msg::InitialPromptChanged(prompt) => self.creation.prompt = prompt,
            Msg::OpenSettings => self.open_settings(ctx),
            Msg::CompletionFadeChanged(value) => {
                match self.service.set_completion_fade_seconds(value) {
                    Ok(reply) => self.settings_save = Some(reply),
                    Err(error) => self.view.layer_mut().set_bottom_left(error),
                }
            }
            Msg::CompletionSoundSelected(value) => {
                match self.service.set_completion_sound_choice(value) {
                    Ok(reply) => self.settings_save = Some(reply),
                    Err(error) => ctx.notify(Notification::error("Cannot select sound", error)),
                }
            }
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
            Msg::SetClearOpencodeHistory(enabled) => {
                match self.service.set_clear_opencode_history(enabled) {
                    Ok(reply) => self.settings_save = Some(reply),
                    Err(error) => ctx.notify(Notification::error("Cannot save settings", error)),
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
                    self.submit_close_opencode_scope(name.clone(), ctx);
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
                            self.creation.opencode(),
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
                            Ok(crate::service::CreateInstanceOutcome::Started {
                                operation,
                                opencode,
                            }) => {
                                if let Some(reply) = opencode {
                                    self.creation.launches.push((self.name.clone(), reply));
                                }
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
        self.template_details = None;
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
                    &self.creation,
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
        self.settings_sound_choice.borrow_mut().take();
        self.open(
            dialogs::settings(
                self.service.branch_instances(),
                self.service.opencode_enabled(),
                self.service.clear_opencode_history(),
                self.service.completion_fade_seconds(),
                self.service.completion_sound_choices(),
                &self.service.completion_sound_choice(),
                Rc::clone(&self.settings_sound_choice),
            ),
            ctx,
        );
    }

    fn open_details(&mut self, row: &Row, ctx: &mut EventCtx<Msg>) {
        self.view.replace_layer(dialogs::details(row), ctx);
        self.details_open = true;
        self.template_details = row.is_template().then(|| row.clone());
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
        let new_opencode =
            self.service.opencode_enabled() && opencode::new_session_directory(&row).is_some();
        let close_opencode_group =
            self.service.opencode_enabled() && opencode::close_scope(&row).is_some();
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
                new_opencode,
                close_opencode_group,
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
                action_menu::Action::NewSession => {
                    if let Some(row) = self.selected() {
                        self.create_opencode_session(&row, ctx);
                    }
                    return;
                }
                action_menu::Action::CloseSessions => {
                    if let Some(row) = self.selected() {
                        self.confirm_close_opencode_scope(&row, ctx);
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
                        self.open(dialogs::confirm_service_state(&name, &service, true), ctx);
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
            .map(|row| {
                if index == 6 || matches!(index, 3 | 7) && row.service_name.is_none() {
                    instances::selected_instance(&self.instances).unwrap_or(row)
                } else {
                    row
                }
            })
            .filter(|row| index == 0 || row.opencode.is_none());
        self.name.clear();
        self.description.clear();
        self.creation.reset_form();
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
                    let modal = dialogs::confirm_service_state(&name, &service, running);
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
                        let modal = dialogs::confirm_restart(&name, service.as_deref());
                        self.intent = Some(Intent::Restart { name, service });
                        self.open(modal, ctx);
                    }
                }
            }
            8 => self.confirm_stop_all(ctx),
            9 => self.confirm_purge_all(ctx),
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

    fn returns_to_data_view(event: &TuiEvent) -> bool {
        let TuiEvent::Key(key) = event else {
            return false;
        };
        KeySpec::key(tuicore::Key::Esc).matches(*key)
            || KeySpec::key_with_modifiers(tuicore::Key::Char('['), tuicore::KeyModifiers::CONTROL)
                .matches(*key)
    }

    fn show_agent_overview(&mut self, event: &TuiEvent, ctx: &mut EventCtx<Msg>) -> bool {
        if !matches!(event, TuiEvent::Hotkey(tuicore::HotkeyEvent::Commit(sequence)) if sequence == "shift+h")
        {
            return false;
        }
        self.running_only = true;
        self.opencode_history = false;
        self.select_overview(self.service.opencode_enabled());
        self.tabs_mut()
            .select_index_with_settings(0, ctx.animation());
        {
            let mut toolbar = self.toolbar_state.borrow_mut();
            toolbar.running_only = true;
            toolbar.show_saved = false;
        }
        let operations = self.service.operations();
        self.update_overview_rows(&self.snapshot.clone(), &operations, false);
        self.pages_mut().focus_overview(event, ctx);
        ctx.request_layout();
        ctx.request_redraw();
        true
    }

    fn navigate_tabs(&mut self, event: &TuiEvent, ctx: &mut EventCtx<Msg>) -> bool {
        if self.view.is_active() || self.transient_menu_active() || !self.service.opencode_enabled()
        {
            return false;
        }
        let TuiEvent::Key(key) = event else {
            return false;
        };
        let bindings = tuicore::keybindings();
        if !bindings.tabs().previous_matches(*key) && !bindings.tabs().next_matches(*key) {
            return false;
        }
        // With two tabs, wrapping left or right selects the other view.
        self.handle_message(
            Msg::SetAttachedSessionsOnly(!self.attached_sessions_only),
            ctx,
        );
        ctx.stop_propagation();
        true
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
            && KeySpec::shifted('n').matches(*key)
            && self.service.opencode_enabled()
        {
            self.handle_message(Msg::SetCompletionSound(!self.completion_sound), ctx);
            ctx.stop_propagation();
            return true;
        }
        if let TuiEvent::Key(key) = event
            && KeySpec::shifted('a').matches(*key)
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
            && KeySpec::plain('n').matches(*key)
            && let Some(row) = self.selected()
            && self.create_opencode_session(&row, ctx)
        {
            ctx.stop_propagation();
            return true;
        }
        if let TuiEvent::Key(key) = event
            && KeySpec::plain('c').matches(*key)
            && let Some(row) = self.selected()
            && (self.confirm_close_opencode_scope(&row, ctx) || self.close_opencode(&row, ctx))
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
        let sound = self.settings_sound_choice.borrow_mut().take();
        if let Some(sound) = sound {
            self.handle_message(Msg::CompletionSoundSelected(sound), ctx);
        }
        let sessions = self.service.opencode_enabled() && self.tabs_mut().selected_index() == 0;
        if sessions != self.attached_sessions_only {
            self.handle_message(Msg::SetAttachedSessionsOnly(sessions), ctx);
        }
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
        if self.show_agent_overview(event, ctx)
            || self.navigate_tabs(event, ctx)
            || self.handle_key(event, ctx)
        {
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
        if self.show_agent_overview(event, ctx) {
            return EventOutcome::Handled;
        }
        if self.navigate_tabs(event, ctx) {
            self.pages_mut().retain_control_focus(route, ctx);
            return EventOutcome::Handled;
        }
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
        let textarea_route = self.view.is_active()
            && route
                .path
                .keys()
                .iter()
                .any(|key| matches!(key.as_str(), "description" | "initial-prompt"));
        if !textarea_route && self.handle_key(event, ctx) {
            return EventOutcome::Handled;
        }
        let outcome = if self.transient_menu_active() {
            self.transient_menu_event(event, ctx)
        } else {
            self.view.dispatch_event(route, event, ctx)
        };
        self.after_event(ctx);
        // Textareas consume Ctrl+Enter to leave insert mode before the dialog can submit.
        if textarea_route
            && ctx.propagation() != tuicore::Propagation::Stopped
            && self.handle_key(event, ctx)
        {
            return EventOutcome::Handled;
        }
        if Self::returns_to_data_view(event) && data_view_route && outcome == EventOutcome::Ignored
        {
            ctx.focus(initial_focus());
            ctx.stop_propagation();
            return EventOutcome::Handled;
        }
        outcome
    }
    fn tick(&mut self, dt: Duration, settings: AnimationSettings) -> TickResult {
        let fade = self.service.completion_fade_seconds();
        for state in self.pages_mut().states() {
            instances::set_completion_fade(&state, fade);
        }
        self.service.poll_opencode();
        self.poll_opencode_action();
        if self.refresh_schedule.tick(Instant::now()) {
            self.service.poll_environments();
        }
        let opencode_enabled = self.toolbar_state.borrow().opencode_enabled;
        let mut changed = self.sync_environment();
        changed |= self.poll_description_save();
        changed |= self.poll_creation_launches();
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
                } else {
                    self.view.layer_mut().set_bottom_left(String::new());
                }
                changed = true;
            }
        }
        let mut result = self.view.tick(dt, settings);
        result.layout |= std::mem::take(&mut self.details_layout_pending);
        result.layout |= opencode_enabled != self.toolbar_state.borrow().opencode_enabled;
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
        self.tabs_mut().set_focused(true, ctx.animation());
    }
    fn dispatch_focus(&mut self, target: &FocusTarget, focused: bool, ctx: &mut FocusCtx<Msg>) {
        self.view.dispatch_focus(target, focused, ctx);
        self.tabs_mut().set_focused(true, ctx.animation());
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

fn overview_tabs(opencode_enabled: bool, sessions: bool) -> Tabs<Msg> {
    let mut pages = Vec::new();
    if opencode_enabled {
        pages.push(Tab::text("Sessions", ""));
    }
    pages.push(Tab::text("Instances", ""));
    let mut tabs = Tabs::new(pages)
        .selected(usize::from(opencode_enabled && !sessions))
        .focused(true)
        .variant(TabsVariant::OneRow)
        .action_hotkey("yy", |_| Msg::CopyName)
        .action_hotkey("yi", |_| Msg::CopyName)
        .action_hotkey("yd", |_| Msg::CopyDescription)
        .action_hotkey("yw", |_| Msg::CopyWorkspace)
        .action_hotkey("yu", |_| Msg::CopyGatewayUrl);
    for sequence in ["yy", "yi", "yd", "yw", "yu"] {
        tabs.set_action_hotkey_visible(sequence, false);
    }
    tabs
}

fn details_width_percent(width: u16) -> u16 {
    if width < MOBILE_TABS_WIDTH { 100 } else { 75 }
}

#[cfg(test)]
mod tests;
