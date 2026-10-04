use ratatui::{
    Frame,
    layout::{Constraint, Rect},
    style::Style,
    text::{Line, Span, Text},
};
use std::{cell::RefCell, rc::Rc, time::Duration};
use tuicore::{
    AnimationSettings, Button, ChildKey, ChildSlot, Column, DataView, DataViewTypedEvent,
    DialogLayer, EventCtx, EventOutcome, EventRoute, FocusCtx, FocusId, FocusTarget,
    HotkeyLabelMode, KeySpec, LayoutCtx, LayoutProposal, LayoutResult, LayoutSizeHint,
    LifecycleCtx, RenderCtx, TickResult, TuiEvent, TuiNode,
};

mod dialogs;
mod enabled;
mod node;
mod prompt;

use super::{Msg, events::clean};
use crate::store::rules::{Acceptance, Rule, Snapshot};

pub(super) const FOCUS: &str = "rule-list";
const DATA_SLOT: &str = "rule-data";
pub(super) type SharedState = Rc<RefCell<Snapshot>>;
pub(super) type FocusState = Rc<RefCell<Option<String>>>;

#[derive(Clone, Debug)]
pub(crate) struct Draft {
    pub rule: Rule,
    pub saved: Rule,
    pub dirty: bool,
    pub pending_toggle: Option<PendingToggle>,
    pub prompt_error: Option<String>,
}

#[derive(Clone, Copy, Debug)]
pub(crate) enum PendingToggle {
    Enabled(bool),
    StartInstance(bool),
}

#[derive(Clone, PartialEq)]
enum Entry {
    Rule(Box<Rule>),
    Acceptance(Box<super::acceptances::Target>),
}

impl Entry {
    fn id(&self) -> String {
        match self {
            Self::Rule(rule) => format!("rule:{}", rule.definition.name),
            Self::Acceptance(row) => format!("acceptance:{}", row.acceptance.id),
        }
    }
    fn search(&self) -> String {
        match self {
            Self::Rule(rule) => format!(
                "{} {} {} {} {}",
                rule.definition.name,
                rule.definition.description,
                rule.definition.template,
                rule.definition.model,
                rule.definition.variant.as_deref().unwrap_or_default()
            ),
            Self::Acceptance(row) => row.search(),
        }
    }
}

enum Scope {
    Rules,
    Rule(String),
}

type RuleView = DialogLayer<DataView<Entry, String>, super::action_menu::ActionMenu>;

pub(super) struct Rules {
    view: RuleView,
    shared: SharedState,
    scope: Scope,
    rows: Vec<Entry>,
    initial: Vec<Acceptance>,
    control: Option<ChildSlot<Button<Msg>, Msg>>,
    control_area: Rect,
    view_area: Rect,
    requested: FocusState,
    context: super::acceptances::SharedContext,
}

impl Rules {
    pub(super) fn new(shared: SharedState, keys: [KeySpec; 10]) -> Self {
        Self::build(shared, Scope::Rules, Vec::new(), keys)
    }
    pub(super) fn with_focus_request(mut self, requested: FocusState) -> Self {
        self.requested = requested;
        self
    }
    pub(super) fn for_rule(
        shared: SharedState,
        name: String,
        keys: [KeySpec; 10],
        context: super::acceptances::SharedContext,
    ) -> Self {
        let mut view = Self::build(shared, Scope::Rule(name), Vec::new(), keys);
        view.context = context;
        view.sync();
        view
    }

    fn build(
        shared: SharedState,
        scope: Scope,
        initial: Vec<Acceptance>,
        keys: [KeySpec; 10],
    ) -> Self {
        let rules = matches!(scope, Scope::Rules);
        let details = shared.clone();
        let view = DataView::new(Vec::new(), Entry::id)
            .focus_id(if rules { FOCUS } else { "acceptance-list" })
            .columns(vec![
                Column::multiline(
                    "rule",
                    if rules { "Rules" } else { "Acceptances" },
                    Constraint::Fill(1),
                    move |row: &Entry, _| {
                        let mut text = entry_text(row);
                        if let Entry::Rule(rule) = row
                            && let Some(error) = details
                                .borrow()
                                .evaluation_errors
                                .iter()
                                .find(|error| error.rule_name == rule.definition.name)
                        {
                            text.lines[1] = Line::from(format!(
                                "{} · Last evaluation failure: {}",
                                clean(&rule.definition.description),
                                clean(&error.error)
                            ));
                        }
                        text
                    },
                )
                .search_key(Entry::search),
            ])
            .headers(false)
            .action_bar(true)
            .filter_controls(false)
            .row_height_by(|row| match row {
                Entry::Acceptance(target) => target.height(),
                Entry::Rule(_) => 2,
            })
            .empty_state(tuicore::SeasonalEmptyState::new(if rules {
                "No rules configured. Create a Rhai rule through MCP."
            } else {
                "No matching acceptances."
            }));
        let view = DialogLayer::new(view, super::action_menu::ActionMenu::new(keys))
            .active(false)
            .fit_content()
            .fit_content_max(42, super::action_menu::MENU_HEIGHT)
            .child_overlays_use_base_bounds(true);
        let mut result = Self {
            view,
            shared: shared.clone(),
            scope,
            rows: Vec::new(),
            initial,
            control: rules.then(|| {
                let state = shared.clone();
                ChildSlot::new(
                    "rule-activate-all",
                    Button::new("Activate all")
                        .hotkey("shift+a")
                        .hotkey_focus_enabled(false)
                        .hotkey_label_mode(HotkeyLabelMode::Inline)
                        .on_press(move || Msg::RuleBulkAction(bulk_enabled(&state.borrow()))),
                )
            }),
            control_area: Rect::default(),
            view_area: Rect::default(),
            requested: Rc::new(RefCell::new(None)),
            context: Rc::new(RefCell::new(super::acceptances::Context::default())),
        };
        result.sync();
        result
    }

    fn sync(&mut self) -> bool {
        let shared = self.shared.borrow();
        let rows: Vec<_> = match &self.scope {
            Scope::Rules => shared
                .rules
                .iter()
                .cloned()
                .map(|rule| Entry::Rule(Box::new(rule)))
                .collect(),
            scope => {
                let mut acceptances = shared.acceptances.clone();
                for row in &self.initial {
                    if !acceptances.iter().any(|other| other.id == row.id) {
                        acceptances.push(row.clone());
                    }
                }
                acceptances.sort_by_key(|row| std::cmp::Reverse(row.id));
                acceptances
                    .into_iter()
                    .filter(|row| match scope {
                        Scope::Rule(name) => &row.rule_name == name,
                        Scope::Rules => false,
                    })
                    .map(|row| {
                        Entry::Acceptance(Box::new(
                            super::acceptances::Target::new(
                                &row,
                                shared.workspaces.get(&row.id),
                                &self.context.borrow(),
                                true,
                            )
                            .with_report(shared.reports.get(&row.id)),
                        ))
                    })
                    .collect()
            }
        };
        let changed = rows != self.rows;
        if changed {
            self.view.base_mut().set_rows(rows.clone());
            self.view.base_mut().take_events();
            self.rows = rows;
        }
        if let Some(name) = self.requested.borrow_mut().take() {
            self.view.base_mut().clear_search();
            self.view.base_mut().clear_filters();
            self.view.base_mut().highlight_id(&format!("rule:{name}"));
            self.view.base_mut().reveal_highlighted();
            return true;
        }
        changed
    }

    fn action(&mut self, event: &TuiEvent, ctx: &mut EventCtx<Msg>) -> bool {
        if self.view.is_active() {
            return false;
        }
        if super::App::returns_to_data_view(event)
            && self
                .control
                .as_ref()
                .is_some_and(|control| control.child().is_focused())
        {
            ctx.focus(tuicore::FocusRequest::Target(FocusId::new(FOCUS)));
            ctx.request_redraw();
            ctx.stop_propagation();
            return true;
        }
        if matches!(self.scope, Scope::Rules) && super::App::overview_requested(event) {
            self.view.base_mut().clear_search();
            self.view.base_mut().clear_filters();
            if let Some(row) = self.rows.first() {
                self.view.base_mut().highlight_id(&row.id());
            }
            self.view.base_mut().reveal_highlighted();
            ctx.focus(tuicore::FocusRequest::Target(FocusId::new(FOCUS)));
            ctx.stop_propagation();
            return true;
        }
        if self.view.base().is_searching() {
            return false;
        }
        let TuiEvent::Key(key) = event else {
            return false;
        };
        if matches!(self.scope, Scope::Rules) && KeySpec::shifted('a').matches(*key) {
            let shared = self.shared.borrow();
            let enabled = bulk_enabled(&shared);
            if shared
                .rules
                .iter()
                .any(|rule| rule.definition.enabled != enabled)
            {
                ctx.emit(Msg::RuleBulkAction(enabled));
            }
            ctx.stop_propagation();
            return true;
        }
        let Some(id) = self.view.base().highlighted_id() else {
            return false;
        };
        let Some(row) = self.rows.iter().find(|row| row.id() == id) else {
            return false;
        };
        if KeySpec::key(tuicore::Key::Enter).matches(*key) && !self.view.base().is_focused() {
            return false;
        }
        match row {
            Entry::Rule(rule) if KeySpec::key(tuicore::Key::Enter).matches(*key) => {
                if let Some(message) =
                    super::row_actions::Target::Rule(rule.clone()).enter_message()
                {
                    ctx.emit(message);
                }
            }
            Entry::Rule(rule) if KeySpec::plain('.').matches(*key) => {
                ctx.emit(Msg::OpenRowMenu(super::row_actions::Target::Rule(
                    rule.clone(),
                )));
            }
            Entry::Rule(rule)
                if KeySpec::plain('a').matches(*key)
                    || KeySpec::key(tuicore::Key::Char(' ')).matches(*key) =>
            {
                let mut rule = rule.as_ref().clone();
                rule.definition.enabled = !rule.definition.enabled;
                ctx.emit(Msg::SaveRule(Box::new(rule)));
            }
            Entry::Acceptance(row) if KeySpec::plain('.').matches(*key) => {
                self.view.layer_mut().open_row(
                    super::row_actions::Target::AcceptanceContext(row.clone()),
                    ctx,
                );
                self.view.set_active_with_context(true, ctx);
            }
            Entry::Acceptance(row) => return row.action(event, ctx),
            _ => return false,
        }
        ctx.stop_propagation();
        true
    }

    fn drain(&mut self, ctx: &mut EventCtx<Msg>) {
        if self.view.is_active() {
            let action = self.view.layer_mut().take_action();
            if action.is_some() || !self.view.layer().is_open() {
                self.view.set_active_with_context(false, ctx);
                if let Some(super::action_menu::Action::Row(command)) = action
                    && let Some(message) = self.view.layer_mut().take_row_message(command)
                {
                    ctx.emit(message);
                }
            }
        }
        for event in self.view.base_mut().take_events() {
            if let DataViewTypedEvent::Activated { row_id } = event
                && let Some(row) = self.rows.iter().find(|row| row.id() == row_id)
            {
                ctx.emit(match row {
                    Entry::Rule(rule) => Msg::OpenRule(rule.clone()),
                    Entry::Acceptance(row) => Msg::FocusEvent(row.acceptance.event_sequence),
                });
            }
        }
    }
}

fn bulk_enabled(snapshot: &Snapshot) -> bool {
    !snapshot.rules.iter().any(|rule| rule.definition.enabled)
}

fn entry_text(row: &Entry) -> Text<'static> {
    let theme = tuicore::theme();
    match row {
        Entry::Rule(rule) => Text::from(vec![
            Line::from(vec![
                Span::styled(
                    if rule.definition.enabled {
                        " "
                    } else {
                        " "
                    },
                    Style::default().fg(if rule.definition.enabled {
                        theme.success_fg()
                    } else {
                        theme.text_fg()
                    }),
                ),
                Span::raw(format!(
                    "{} 󰠲 {} 󰧑 {} · {} · ",
                    clean(&rule.definition.name),
                    clean(&rule.definition.template),
                    clean(rule.definition.model.split('#').next().unwrap_or_default()),
                    clean(
                        rule.definition
                            .variant
                            .as_deref()
                            .or_else(|| rule
                                .definition
                                .model
                                .split_once('#')
                                .map(|(_, variant)| variant))
                            .unwrap_or("default")
                    )
                )),
                Span::raw(if rule.definition.start_instance {
                    "󰒋"
                } else {
                    "󰒏"
                }),
            ]),
            Line::from(clean(&rule.definition.description)),
        ]),
        Entry::Acceptance(row) => row.text("⠋", None),
    }
}

impl super::App {
    pub(super) fn rules_tab_index(&self) -> usize {
        if self.service.opencode_enabled() {
            3
        } else {
            2
        }
    }

    pub(super) fn poll_event_focus(&mut self) -> bool {
        let result = match self.event_focus_action.as_mut() {
            Some(reply) => match reply.try_recv() {
                Ok(result) => result,
                Err(tokio::sync::oneshot::error::TryRecvError::Empty) => return false,
                Err(_) => Err(crate::store::events::Error::Storage(
                    "event worker stopped".into(),
                )),
            },
            None => return false,
        };
        self.event_focus_action = None;
        match result {
            Ok(record) => {
                self.running_only = false;
                self.toolbar_state.borrow_mut().running_only = false;
                self.update_snapshot(self.snapshot.clone());
                self.tabs_mut().select_index(1);
                self.events_active = true;
                self.providers_active = false;
                self.rules_active = false;
                self.pages_mut().select_events();
                self.pages_mut().focus_event(record);
            }
            Err(error) => self.notify(tuicore::Notification::error(
                "Event unavailable",
                error.to_string(),
            )),
        }
        true
    }
}
