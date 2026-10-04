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

mod acceptances;
mod action_menu;
mod bulk;
mod creation;
mod details;
mod dialogs;
mod events;
mod instances;
mod opencode;
mod operations;
mod overview;
mod properties;
mod providers;
mod refresh;
mod route_menu;
mod row_actions;
mod rows;
mod rules;
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
        instance.workspace_only
            || instance.is_running()
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
    StartInstanceChanged(bool),
    OpenSettings,
    CompletionFadeChanged(String),
    CompletionSoundSelected(String),
    EventAcceptanceSoundSelected(String),
    Refresh,
    StopAll,
    PurgeAll,
    SetRunningOnly(bool),
    SetBranchInstances(bool),
    SetOpencodeIntegration(bool),
    SetClearOpencodeHistory(bool),
    SetOpencodeHistory(bool),
    SetAttachedSessionsOnly(bool),
    SetCompletionSound(bool),
    CopyName,
    CopyDescription,
    CopyWorkspace,
    CopyGatewayUrl,
    Submit,
    OpenEvent(Box<crate::store::events::Record>),
    ReplayEvent(i64),
    ReplayEventConfirmed(i64),
    DeleteEvents(crate::store::events::Deletion),
    DeleteEventsConfirmed(crate::store::events::Deletion),
    OpenRule(Box<crate::store::rules::Rule>),
    SaveRule(Box<crate::store::rules::Rule>),
    RuleBulkAction(bool),
    SaveRules(Vec<crate::store::rules::Rule>),
    RuleDraftChanged(Rc<RefCell<rules::Draft>>),
    RuleDraftEnabled(Rc<RefCell<rules::Draft>>, bool),
    RuleDraftStartInstance(Rc<RefCell<rules::Draft>>, bool),
    FocusEvent(i64),
    FocusRule(String),
    AcceptanceInstance(String),
    AcceptanceAction(Box<acceptances::Target>, row_actions::Command),
    RecreateAcceptanceConfirmed(Box<crate::store::rules::Acceptance>, Option<String>),
    ProviderAction(String, crate::store::providers::Action),
    ProviderStreamAction(String, String, crate::store::providers::Action),
    ProviderStreamDetails(String, Box<crate::store::providers::Stream>),
    ProviderStreamEvents(String, String),
    ProviderBulkAction(crate::store::providers::Action),
    ProviderDetails(Box<crate::store::providers::Provider>),
    ShowProvider(String),
    OpenRowMenu(row_actions::Target),
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
    tab_counts: (usize, usize),
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
    settings_sound_choice: Rc<RefCell<Option<Msg>>>,
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
    event_acceptance_count: Option<u64>,
    opencode_action: Option<opencode::PendingAction>,
    acceptance_recreation: Option<tokio::sync::oneshot::Receiver<Result<(), String>>>,
    acceptance_selection: Rc<RefCell<Option<acceptances::Target>>>,
    events_active: bool,
    providers_active: bool,
    rules_active: bool,
    provider_actions: Vec<providers::PendingAction>,
    provider_confirmation: Option<(String, crate::store::providers::Action)>,
    provider_stream_confirmation: Option<(String, String, crate::store::providers::Action)>,
    provider_bulk_confirmation: Option<(Vec<String>, crate::store::providers::Action)>,
    event_action: Option<tokio::sync::oneshot::Receiver<Result<i64, crate::store::events::Error>>>,
    event_deletion: Option<crate::store::events::Deletion>,
    rule_save: Option<tokio::sync::oneshot::Receiver<Result<crate::store::rules::Rule, String>>>,
    rule_saves: Vec<crate::store::rules::Rule>,
    rule_editor: Option<Rc<RefCell<rules::Draft>>>,
    rule_toggle: Option<Rc<RefCell<rules::Draft>>>,
    rule_autosave: Option<Rc<RefCell<rules::Draft>>>,
    rule_autosaves: Vec<Rc<RefCell<rules::Draft>>>,
    event_focus_action: Option<
        tokio::sync::oneshot::Receiver<
            Result<crate::store::events::Record, crate::store::events::Error>,
        >,
    >,
}

pub(crate) fn root(service: AppService) -> App {
    #[cfg(test)]
    tests::init_ui();
    tuicore::set_keybindings(
        tuicore::keybindings()
            .with_tabs_close([KeySpec::plain('x')])
            .with_data_view_activate([KeySpec::plain('d')]),
    );
    let keys = service.environment_keys();
    let snapshot = service.environment_snapshot();
    let opencode_enabled = service.opencode_enabled();
    let opencode_snapshot = service.opencode_snapshot();
    let event_acceptance_count = service.event_snapshot().accepted_attempts;
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
        overview_tabs(opencode_enabled, opencode_enabled, (0, 0)),
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
        tab_counts: (0, 0),
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
        event_acceptance_count,
        opencode_action: None,
        acceptance_recreation: None,
        acceptance_selection: Rc::new(RefCell::new(None)),
        events_active: false,
        providers_active: false,
        rules_active: false,
        provider_actions: Vec::new(),
        provider_confirmation: None,
        provider_stream_confirmation: None,
        provider_bulk_confirmation: None,
        event_action: None,
        event_deletion: None,
        rule_save: None,
        rule_saves: Vec::new(),
        rule_editor: None,
        rule_toggle: None,
        rule_autosave: None,
        rule_autosaves: Vec::new(),
        event_focus_action: None,
    }
}

impl App {
    fn update_snapshot(&mut self, snapshot: EnvironmentSnapshot) -> bool {
        let opencode_enabled = self.service.opencode_enabled();
        let tabs_changed = self.toolbar_state.borrow().opencode_enabled != opencode_enabled;
        if tabs_changed {
            let events_active = self.events_active;
            let providers_active = self.providers_active;
            let rules_active = self.rules_active;
            self.select_overview(self.attached_sessions_only && opencode_enabled);
            *self.tabs_mut() = overview_tabs(
                opencode_enabled,
                self.attached_sessions_only,
                self.tab_counts,
            );
            if events_active {
                self.tabs_mut().select_index(1);
                self.events_active = true;
                self.pages_mut().select_events();
            }
            if providers_active {
                let index = self.providers_tab_index();
                self.tabs_mut().select_index(index);
                self.providers_active = true;
                self.pages_mut().select_providers();
            }
            if rules_active {
                let index = self.rules_tab_index();
                self.tabs_mut().select_index(index);
                self.rules_active = true;
                self.pages_mut().select_rules();
            }
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
        if !initial_load_completed
            && !tabs_changed
            && snapshot.resource_revision.is_some()
            && snapshot.resource_revision == self.snapshot.resource_revision
        {
            let previous = self.toolbar_state.borrow();
            toolbar_state.totals = previous.totals.clone();
            toolbar_state.available_memory_bytes = previous.available_memory_bytes;
            toolbar_state.cpu_temperature_millicelsius = previous.cpu_temperature_millicelsius;
            toolbar_state.has_running_instances = previous.has_running_instances;
        }
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
        if self.events_active || self.providers_active || self.rules_active {
            return None;
        }
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

    fn sync_tab_counts(&mut self) -> bool {
        let counts = self.pages_mut().tab_counts();
        if counts == self.tab_counts {
            return false;
        }
        self.tab_counts = counts;
        let selected = self.tabs_mut().selected_index();
        *self.tabs_mut() = overview_tabs(
            self.service.opencode_enabled(),
            self.attached_sessions_only,
            counts,
        )
        .selected(selected);
        true
    }

    fn instances_tab_index(&self) -> usize {
        if self.service.opencode_enabled() {
            2
        } else {
            0
        }
    }

    fn providers_tab_index(&self) -> usize {
        if self.service.opencode_enabled() {
            4
        } else {
            3
        }
    }

    fn select_overview(&mut self, sessions: bool) {
        self.events_active = false;
        self.providers_active = false;
        self.rules_active = false;
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
        let opencode = self.opencode_snapshot.clone();
        self.pages_mut()
            .update_acceptance_context(snapshot, &opencode);
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
        if self.events_active {
            ctx.focus(tuicore::FocusRequest::Target(FocusId::new(events::FOCUS)));
        }
        ctx.request_layout();
        ctx.request_redraw();
    }

    pub(crate) fn handle_message(&mut self, message: Msg, ctx: &mut EventCtx<Msg>) {
        match message {
            Msg::OpenRowMenu(target) => {
                let menu = self.menu_layer_mut();
                menu.layer_mut().open_row(target, ctx);
                menu.set_active_with_context(true, ctx);
            }
            Msg::ShowProvider(identity) => {
                if self.pages_mut().highlight_provider(&identity) {
                    let index = self.providers_tab_index();
                    self.tabs_mut().select_index(index);
                    self.providers_active = true;
                    self.events_active = false;
                    self.pages_mut().select_providers();
                    ctx.focus(tuicore::FocusRequest::Target(FocusId::new(
                        providers::FOCUS,
                    )));
                    ctx.request_layout();
                } else {
                    ctx.notify(Notification::error(
                        "Provider unavailable",
                        "No installed provider package matches this event's source",
                    ));
                }
            }
            Msg::ProviderStreamEvents(provider, stream) => {
                self.rules_active = false;
                self.tabs_mut().select_index(1);
                self.events_active = true;
                self.providers_active = false;
                self.pages_mut().select_events();
                self.pages_mut().filter_event_stream(provider, stream);
                ctx.focus(tuicore::FocusRequest::Target(FocusId::new(events::FOCUS)));
                ctx.request_layout();
            }
            Msg::ProviderAction(name, action) => self.request_provider_action(name, action, ctx),
            Msg::ProviderStreamAction(name, stream, action) => {
                self.request_provider_stream_action(name, stream, action, ctx)
            }
            Msg::ProviderStreamDetails(name, stream) => {
                self.open_provider_stream_details(&name, &stream, ctx)
            }
            Msg::ProviderBulkAction(action) => self.request_provider_bulk_action(action, ctx),
            Msg::ProviderDetails(provider) => self.open_provider_details(&provider, ctx),
            Msg::OpenEvent(row) => self.open_event(&row, ctx),
            Msg::DeleteEvents(target) => self.confirm_delete_events(target, ctx),
            Msg::DeleteEventsConfirmed(target) => {
                if self.event_action.is_none() {
                    self.event_action = Some(self.service.delete_events(target));
                    self.event_deletion = Some(target);
                }
                self.handle_message(Msg::Close, ctx);
            }
            Msg::ReplayEvent(sequence) => {
                let modal = dialogs::dialog("Replay event")
                    .actions([
                        tuicore::DialogAction::new("Ok").hotkey(KeySpec::plain('o')).on_trigger(move || Msg::ReplayEventConfirmed(sequence)),
                        tuicore::DialogAction::new("Cancel").hotkey(KeySpec::plain('c')).on_trigger(|| Msg::Close),
                    ])
                    .host(Flex::column().child("approval", tuicore::Paragraph::new("Re-run enabled rules? Matches may create new instances and send prompts."), tuicore::FlexItem::fit_content()));
                self.details_open = false;
                self.open(Box::new(modal), ctx);
            }
            Msg::ReplayEventConfirmed(sequence) => {
                if self.event_action.is_none() {
                    self.event_action = Some(self.service.replay_event(sequence));
                }
                self.handle_message(Msg::Close, ctx);
            }
            Msg::OpenRule(rule) => self.open_rule(*rule, ctx),
            Msg::SaveRule(rule) => self.request_save_rule(*rule, ctx),
            Msg::RuleBulkAction(enabled) => self.request_rule_bulk_action(enabled, ctx),
            Msg::SaveRules(rules) => {
                self.handle_message(Msg::Close, ctx);
                self.save_rules(rules, ctx);
            }
            Msg::RuleDraftChanged(draft) => self.autosave_rule_draft(draft),
            Msg::RuleDraftEnabled(draft, enabled) => {
                self.set_rule_draft_enabled(draft, enabled, ctx);
            }
            Msg::RuleDraftStartInstance(draft, start) => {
                self.set_rule_draft_start_instance(draft, start, ctx);
            }
            Msg::FocusEvent(sequence) => {
                self.event_focus_action = Some(self.service.retained_event(sequence));
                self.handle_message(Msg::Close, ctx);
            }
            Msg::FocusRule(name) => {
                if self.pages_mut().highlight_rule(&name) {
                    self.handle_message(Msg::Close, ctx);
                    let index = self.rules_tab_index();
                    self.tabs_mut().select_index(index);
                    self.sync_overview_tab(ctx);
                    ctx.focus(tuicore::FocusRequest::Target(FocusId::new(rules::FOCUS)));
                    ctx.request_layout();
                } else {
                    ctx.notify(Notification::warning("Rule unavailable", name));
                }
            }
            Msg::AcceptanceInstance(name) => {
                if !self
                    .snapshot
                    .instances
                    .iter()
                    .any(|instance| instance.name == name)
                {
                    ctx.notify(Notification::warning("Instance unavailable", name));
                    return;
                }
                self.handle_message(Msg::Close, ctx);
                self.select_overview(false);
                let index = self.instances_tab_index();
                self.tabs_mut().select_index(index);
                if self.running_only
                    && !visible_rows(
                        &self.snapshot,
                        &self.service.operations(),
                        &self.opencode_snapshot,
                        true,
                    )
                    .iter()
                    .any(|row| row.instance.as_deref() == Some(name.as_str()))
                {
                    self.set_running_only(false, ctx);
                }
                self.update_snapshot(self.snapshot.clone());
                instances::request_view_reset(&self.instances);
                instances::select_instance(&self.instances, &name);
                ctx.focus(initial_focus());
                ctx.request_layout();
            }
            Msg::AcceptanceAction(target, command) => self.acceptance_action(*target, command, ctx),
            Msg::RecreateAcceptanceConfirmed(row, conversation) => {
                if self.acceptance_recreation.is_some() {
                    ctx.notify(Notification::warning(
                        "Recreation in progress",
                        "Wait for the current recreation to finish",
                    ));
                } else {
                    self.acceptance_recreation =
                        Some(self.service.recreate_acceptance(row.id, conversation, true));
                    self.handle_message(Msg::Close, ctx);
                    ctx.notify(Notification::info("Recreating instance", &row.instance));
                }
            }
            Msg::SetOpencodeIntegration(enabled) => {
                self.acceptance_selection.borrow_mut().take();
                match self.service.set_opencode_enabled(enabled) {
                    Ok(reply) => self.settings_save = Some(reply),
                    Err(error) => ctx.notify(Notification::error("Cannot save settings", error)),
                }
            }
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
                let index = if enabled {
                    0
                } else {
                    self.instances_tab_index()
                };
                self.tabs_mut()
                    .select_index_with_settings(index, ctx.animation());
                self.update_snapshot(self.snapshot.clone());
                ctx.request_layout();
                ctx.request_redraw();
            }
            Msg::SetCompletionSound(enabled) => {
                self.event_acceptance_count = self.service.event_snapshot().accepted_attempts;
                self.completion_sound = enabled;
                self.toolbar_state.borrow_mut().completion_sound = enabled;
                ctx.request_redraw();
            }
            Msg::Close => {
                self.acceptance_selection.borrow_mut().take();
                self.rule_editor = None;
                self.provider_confirmation = None;
                self.provider_stream_confirmation = None;
                self.provider_bulk_confirmation = None;
                self.service.cancel_opencode_conversation();
                self.settings_save = None;
                self.view.set_active_with_context(false, ctx);
                ctx.focus(if self.providers_active {
                    tuicore::FocusRequest::Target(FocusId::new(providers::FOCUS))
                } else if self.events_active {
                    tuicore::FocusRequest::Target(FocusId::new(events::FOCUS))
                } else if self.rules_active {
                    tuicore::FocusRequest::Target(FocusId::new(rules::FOCUS))
                } else {
                    initial_focus()
                });
                self.intent = None;
                self.details_open = false;
                self.template_details = None;
            }
            Msg::NameChanged(name) => self.name = name,
            Msg::DescriptionChanged(description) => self.description = description,
            Msg::InitialPromptChanged(prompt) => self.creation.prompt = prompt,
            Msg::StartInstanceChanged(start) => self.creation.start_instance = start,
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
            Msg::EventAcceptanceSoundSelected(value) => {
                match self.service.set_event_acceptance_sound_choice(value) {
                    Ok(reply) => self.settings_save = Some(reply),
                    Err(error) => ctx.notify(Notification::error("Cannot select sound", error)),
                }
            }
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
                if let Some((name, stream, action)) = self.provider_stream_confirmation.take() {
                    self.start_provider_stream_action(name, stream, action);
                    self.handle_message(Msg::Close, ctx);
                    ctx.request_layout();
                    ctx.request_redraw();
                    return;
                }
                if let Some((names, action)) = self.provider_bulk_confirmation.take() {
                    for name in names {
                        self.start_provider_action(name, action, true);
                    }
                    self.view.set_active_with_context(false, ctx);
                    ctx.focus(tuicore::FocusRequest::Target(FocusId::new(
                        providers::FOCUS,
                    )));
                    ctx.request_layout();
                    ctx.request_redraw();
                    return;
                }
                if let Some((name, action)) = self.provider_confirmation.take() {
                    self.start_provider_action(name, action, true);
                    self.view.set_active_with_context(false, ctx);
                    ctx.focus(tuicore::FocusRequest::Target(FocusId::new(
                        providers::FOCUS,
                    )));
                    ctx.request_layout();
                    ctx.request_redraw();
                    return;
                }
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
                            self.creation.start_instance,
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
                        self.handle_message(Msg::Close, ctx);
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
        self.rule_editor = None;
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

    fn open_compact(&mut self, modal: Modal, ctx: &mut EventCtx<Msg>) {
        self.open(modal, ctx);
        self.view.set_fit_content_max(dialogs::COMPACT_WIDTH, 34);
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
                [
                    &self.service.completion_sound_choice(),
                    &self.service.event_acceptance_sound_choice(),
                ],
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

    fn row_enter_action(&self, row: &Row) -> Option<action_menu::Action> {
        use action_menu::Action;
        if row.informational {
            return None;
        }
        if let Some(target) = &row.opencode {
            return match target {
                opencode::Target::Session { .. } | opencode::Target::Client { .. } => {
                    Some(if target.attached() {
                        Action::GotoPanel
                    } else {
                        Action::OpenPanel
                    })
                }
                opencode::Target::Workspace => None,
            };
        }
        if row.cleanup_target.is_some() {
            return Some(Action::Details);
        }
        if row.is_template() && row.template_available {
            return Some(Action::NewInstance);
        }
        if row.gateway_url.is_some() {
            return Some(Action::OpenBrowser);
        }
        if row.service.is_none()
            && row.checkout_path.is_none()
            && row
                .instance
                .as_deref()
                .is_some_and(|name| !self.instance_routes(name).is_empty())
        {
            return Some(Action::OpenRoutes);
        }
        None
    }

    fn activate_row(&mut self, ctx: &mut EventCtx<Msg>) -> bool {
        let Some(row) = self.selected() else {
            return false;
        };
        let Some(action) = self.row_enter_action(&row) else {
            return false;
        };
        match action {
            action_menu::Action::OpenPanel | action_menu::Action::GotoPanel => {
                self.activate_opencode(&row, ctx);
            }
            action_menu::Action::OpenBrowser | action_menu::Action::OpenRoutes => {
                self.open_selected_route(ctx);
            }
            _ => self.action(action.index(), ctx),
        }
        true
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
        let enter_action = self.row_enter_action(&row);
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
                workspace_missing: row.workspace_missing,
                enter_action,
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
                action_menu::Action::Row(command) => {
                    if let Some(message) =
                        self.menu_layer_mut().layer_mut().take_row_message(command)
                    {
                        self.handle_message(message, ctx);
                    }
                    return;
                }
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
                action_menu::Action::OpenRoutes => {
                    self.open_selected_route(ctx);
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
        self.instance_routes(&name)
    }

    fn instance_routes(&self, name: &str) -> Vec<RouteChoice> {
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
        self.open_routes(routes, ctx)
    }

    fn open_routes(&mut self, routes: Vec<RouteChoice>, ctx: &mut EventCtx<Msg>) -> bool {
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
        ctx.focus(if self.events_active {
            tuicore::FocusRequest::Target(FocusId::new(events::FOCUS))
        } else if self.rules_active {
            tuicore::FocusRequest::Target(FocusId::new(rules::FOCUS))
        } else {
            initial_focus()
        });
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

    fn overview_requested(event: &TuiEvent) -> bool {
        matches!(event, TuiEvent::Hotkey(tuicore::HotkeyEvent::Commit(sequence)) if sequence == "shift+h")
            || matches!(event, TuiEvent::Key(key) if KeySpec::shifted('h').matches(*key))
    }

    fn show_agent_overview(&mut self, event: &TuiEvent, ctx: &mut EventCtx<Msg>) -> bool {
        if !Self::overview_requested(event) || self.view.is_active() || self.transient_menu_active()
        {
            return false;
        }
        let event = TuiEvent::Hotkey(tuicore::HotkeyEvent::Commit("shift+h".into()));
        self.running_only = true;
        self.opencode_history = false;
        self.completion_sound = false;
        self.select_overview(self.service.opencode_enabled());
        self.tabs_mut()
            .select_index_with_settings(0, ctx.animation());
        {
            let mut toolbar = self.toolbar_state.borrow_mut();
            toolbar.running_only = true;
            toolbar.show_saved = false;
            toolbar.completion_sound = false;
        }
        let operations = self.service.operations();
        self.update_overview_rows(&self.snapshot.clone(), &operations, false);
        self.pages_mut().reset_overviews(&event, ctx);
        ctx.request_layout();
        ctx.request_redraw();
        ctx.stop_propagation();
        true
    }

    fn navigate_tabs(&mut self, event: &TuiEvent, ctx: &mut EventCtx<Msg>) -> bool {
        if self.view.is_active() || self.transient_menu_active() {
            return false;
        }
        let TuiEvent::Key(key) = event else {
            return false;
        };
        let bindings = tuicore::keybindings();
        if !bindings.tabs().previous_matches(*key) && !bindings.tabs().next_matches(*key) {
            return false;
        }
        let count = if self.service.opencode_enabled() {
            5
        } else {
            4
        };
        let current = self.tabs_mut().selected_index();
        let next = if bindings.tabs().previous_matches(*key) {
            (current + count - 1) % count
        } else {
            (current + 1) % count
        };
        self.tabs_mut()
            .select_index_with_settings(next, ctx.animation());
        self.sync_overview_tab(ctx);
        ctx.request_layout();
        ctx.request_redraw();
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
        if self.events_active || self.providers_active || self.rules_active {
            return false;
        }
        if instances::is_searching(&self.instances) {
            return false;
        }
        if matches!(event, TuiEvent::Key(key) if KeySpec::key(tuicore::Key::Enter).matches(*key))
            && self.activate_row(ctx)
        {
            ctx.stop_propagation();
            return true;
        }
        if let TuiEvent::Key(key) = event
            && KeySpec::shifted('n').matches(*key)
            && self.service.opencode_enabled()
            && self.attached_sessions_only
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
            && KeySpec::plain('e').matches(*key)
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
        let selection = self.acceptance_selection.borrow_mut().take();
        if let Some(target) = selection {
            self.acceptance_action(target, row_actions::Command::Session, ctx);
        }
        let sound = self.settings_sound_choice.borrow_mut().take();
        if let Some(sound) = sound {
            self.handle_message(sound, ctx);
        }
        self.sync_overview_tab(ctx);
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
        self.sync_tab_counts();
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
        self.service.poll_rules();
        let rules = self.service.rule_snapshot();
        let rules_changed = self.pages_mut().update_rules(rules);
        self.service.poll_providers();
        let providers = self.service.provider_snapshot();
        let providers_changed = self.pages_mut().update_providers(providers);
        self.service.poll_events();
        let events = self.service.event_snapshot();
        let events_changed = self.update_event_snapshot(events);
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
        changed |= events_changed;
        changed |= providers_changed;
        changed |= rules_changed;
        changed |= self.poll_rule_save();
        changed |= self.poll_event_focus();
        let provider_action_changed = self.poll_provider_action();
        changed |= provider_action_changed;
        changed |= self.poll_event_action();
        changed |= self.poll_description_save();
        changed |= self.poll_creation_launches();
        changed |= self.poll_acceptance_recreation();
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
        let tab_counts_changed = self.sync_tab_counts();
        changed |= tab_counts_changed;
        let mut result = self.view.tick(dt, settings);
        result.layout |= tab_counts_changed;
        result.layout |= provider_action_changed;
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

fn overview_tabs(opencode_enabled: bool, sessions: bool, counts: (usize, usize)) -> Tabs<Msg> {
    let mut pages = Vec::new();
    if opencode_enabled {
        pages.push(Tab::text("Sessions", ""));
    }
    if !opencode_enabled {
        pages.push(Tab::text("Instances", ""));
    }
    pages.push(Tab::text("Events", ""));
    if opencode_enabled {
        pages.push(Tab::text("Instances", ""));
    }
    for (title, count) in [("Rules", counts.0), ("Streams", counts.1)] {
        let title = if count == 0 {
            title.into()
        } else {
            format!("{title} ({count})")
        };
        pages.push(Tab::text(title, ""));
    }
    let mut tabs = Tabs::new(pages)
        .selected(if opencode_enabled && !sessions { 2 } else { 0 })
        .focused(true)
        .variant(TabsVariant::OneRow)
        .tab_stop(false)
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
