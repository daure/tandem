use std::{cell::RefCell, rc::Rc};

use ratatui::widgets::Borders;
use tuicore::{
    EventCtx, Flex, FlexItem, FormField, KeySpec, Language, Tab, Tabs, TabsVariant, TextInput,
    TextareaInput,
};

use super::{Draft, PendingToggle, Rules};
use crate::{
    app::{App, Msg, dialogs},
    store::rules::Rule,
};

impl App {
    pub(in crate::app) fn open_rule(&mut self, rule: Rule, ctx: &mut EventCtx<Msg>) {
        let shared = self.pages_mut().rules_state();
        let draft = self
            .rule_toggle
            .iter()
            .chain(self.rule_autosave.iter())
            .chain(self.rule_autosaves.iter())
            .find(|draft| draft.borrow().saved.definition.name == rule.definition.name)
            .cloned()
            .unwrap_or_else(|| {
                Rc::new(RefCell::new(Draft {
                    rule: rule.clone(),
                    saved: rule,
                    dirty: false,
                    pending_toggle: None,
                    prompt_error: None,
                }))
            });
        let rule = draft.borrow().rule.clone();
        draft.borrow_mut().prompt_error = self
            .service
            .validate_rule_prompt(&rule.definition.initial_prompt)
            .err();
        let script_draft = draft.clone();
        let script = TextareaInput::new()
            .value(&rule.definition.script)
            .language(Language::Rust)
            .min_rows(5)
            .on_change(move |value| {
                script_draft.borrow_mut().rule.definition.script = value;
                Msg::RuleDraftChanged(script_draft.clone())
            });
        let description_draft = draft.clone();
        let description = TextInput::new()
            .value(&rule.definition.description)
            .on_change(move |value| {
                description_draft.borrow_mut().rule.definition.description = value;
                Msg::RuleDraftChanged(description_draft.clone())
            });
        let model_draft = draft.clone();
        let model = TextInput::new()
            .value(&rule.definition.model)
            .on_change(move |value| {
                model_draft.borrow_mut().rule.definition.model = value;
                Msg::RuleDraftChanged(model_draft.clone())
            });
        let template_draft = draft.clone();
        let variant_draft = draft.clone();
        let variant = TextInput::new()
            .value(rule.definition.variant.as_deref().unwrap_or_default())
            .on_change(move |value| {
                variant_draft.borrow_mut().rule.definition.variant =
                    (!value.is_empty()).then_some(value);
                Msg::RuleDraftChanged(variant_draft.clone())
            });
        let template = TextInput::new()
            .value(&rule.definition.template)
            .on_change(move |value| {
                template_draft.borrow_mut().rule.definition.template = value;
                Msg::RuleDraftChanged(template_draft.clone())
            });
        let prompt_draft = draft.clone();
        let prompt = TextareaInput::new()
            .value(&rule.definition.initial_prompt)
            .language(Language::Glimmer)
            .min_rows(3)
            .on_change(move |value| {
                prompt_draft.borrow_mut().rule.definition.initial_prompt = value;
                Msg::RuleDraftChanged(prompt_draft.clone())
            });
        let settings = tuicore::ScrollContainer::vertical(
            Flex::column()
                .child(
                    "enabled",
                    super::enabled::SettingsToggle::enabled(draft.clone()),
                    FlexItem::fit_content(),
                )
                .child(
                    "start-instance",
                    super::enabled::SettingsToggle::start_instance(draft.clone()),
                    FlexItem::fit_content(),
                )
                .child(
                    "description",
                    FormField::new("Description", description),
                    FlexItem::fit_content(),
                )
                .child(
                    "template",
                    FormField::new("Handler template", template),
                    FlexItem::fit_content(),
                )
                .child(
                    "model",
                    FormField::new("Session model", model),
                    FlexItem::fit_content(),
                )
                .child(
                    "variant",
                    FormField::new("Thinking variant (blank uses OpenCode default)", variant),
                    FlexItem::fit_content(),
                )
                .child(
                    "prompt",
                    super::prompt::PromptField::new(prompt, draft.clone()),
                    FlexItem::fit_content(),
                ),
        );
        let tabs = Tabs::new(vec![
            Tab::new(
                "Accepted events",
                Rules::for_rule(
                    shared,
                    rule.definition.name,
                    self.keys,
                    self.pages_mut().acceptance_context(),
                ),
            ),
            Tab::new("Script", script),
            Tab::new("Settings", settings),
        ])
        .variant(TabsVariant::OneRow)
        .edge_borders(Borders::TOP);
        let modal = dialogs::dialog("Rule details").host(Flex::column().child(
            "rule-tabs",
            tabs,
            FlexItem::fill(1),
        ));
        self.intent = None;
        self.open(Box::new(modal), ctx);
        self.rule_editor = Some(draft);
        self.details_open = true;
        self.resize_details_dialog();
    }

    pub(in crate::app) fn autosave_rule_draft(&mut self, draft: Rc<RefCell<Draft>>) {
        {
            let mut draft = draft.borrow_mut();
            draft.dirty = true;
            draft.rule.definition.enabled = false;
            draft.prompt_error = self
                .service
                .validate_rule_prompt(&draft.rule.definition.initial_prompt)
                .err();
        }
        if !self
            .rule_autosaves
            .iter()
            .any(|queued| Rc::ptr_eq(queued, &draft))
        {
            self.rule_autosaves.push(draft);
        }
        self.start_rule_autosave();
    }

    pub(in crate::app) fn set_rule_draft_enabled(
        &mut self,
        draft: Rc<RefCell<Draft>>,
        enabled: bool,
        ctx: &mut EventCtx<Msg>,
    ) {
        self.set_rule_draft_toggle(draft, PendingToggle::Enabled(enabled), ctx);
    }

    pub(in crate::app) fn set_rule_draft_start_instance(
        &mut self,
        draft: Rc<RefCell<Draft>>,
        start: bool,
        ctx: &mut EventCtx<Msg>,
    ) {
        self.set_rule_draft_toggle(draft, PendingToggle::StartInstance(start), ctx);
    }

    fn set_rule_draft_toggle(
        &mut self,
        draft: Rc<RefCell<Draft>>,
        toggle: PendingToggle,
        ctx: &mut EventCtx<Msg>,
    ) {
        if !self.rule_action_available(ctx) {
            return;
        }
        let mut target = {
            let draft = draft.borrow();
            if draft.pending_toggle.is_some()
                || draft.dirty
                || draft.rule.definition != draft.saved.definition
                || !self.rule_autosaves.is_empty()
            {
                ctx.notify(tuicore::Notification::warning(
                    "Rule not saved",
                    "Correct invalid fields and wait for saving to finish before changing rule settings",
                ));
                return;
            }
            draft.saved.clone()
        };
        let destination = match toggle {
            PendingToggle::Enabled(enabled) => {
                target.definition.enabled = enabled;
                (!enabled).then_some(target.zellij_session)
            }
            PendingToggle::StartInstance(start) => {
                target.definition.start_instance = start;
                Some(target.zellij_session)
            }
        };
        self.rule_save = Some(self.service.save_rule(
            target.definition,
            Some(target.revision),
            destination,
            true,
        ));
        draft.borrow_mut().pending_toggle = Some(toggle);
        self.rule_toggle = Some(draft);
    }

    fn rule_save_error(&mut self, error: String) {
        self.notify(tuicore::Notification::error("Cannot save rule", error));
    }

    fn start_rule_autosave(&mut self) {
        if self.rule_save.is_some() {
            return;
        }
        if !self.rule_saves.is_empty() {
            let rule = self.rule_saves.remove(0);
            let destination = (!rule.definition.enabled).then_some(rule.zellij_session);
            self.rule_save = Some(self.service.save_rule(
                rule.definition,
                Some(rule.revision),
                destination,
                true,
            ));
            return;
        }
        while !self.rule_autosaves.is_empty() {
            let draft = self.rule_autosaves.remove(0);
            let rule = {
                let mut draft = draft.borrow_mut();
                if !draft.dirty {
                    continue;
                }
                if draft.saved.definition.enabled {
                    let mut paused = draft.saved.clone();
                    paused.definition.enabled = false;
                    paused
                } else if draft.prompt_error.is_some() {
                    draft.dirty = false;
                    continue;
                } else {
                    draft.dirty = false;
                    draft.rule.clone()
                }
            };
            self.rule_save = Some(self.service.save_rule(
                rule.definition,
                Some(rule.revision),
                Some(rule.zellij_session),
                false,
            ));
            self.rule_autosave = Some(draft);
            return;
        }
    }

    pub(in crate::app) fn request_save_rule(&mut self, rule: Rule, ctx: &mut EventCtx<Msg>) {
        if !self.rule_action_available(ctx) {
            return;
        }
        let destination = (!rule.definition.enabled).then_some(rule.zellij_session);
        self.rule_save =
            Some(
                self.service
                    .save_rule(rule.definition, Some(rule.revision), destination, true),
            );
    }

    pub(in crate::app) fn request_rule_bulk_action(
        &mut self,
        enabled: bool,
        ctx: &mut EventCtx<Msg>,
    ) {
        if !self.rule_action_available(ctx) {
            return;
        }
        let rules: Vec<_> = self
            .pages_mut()
            .rules_state()
            .borrow()
            .rules
            .iter()
            .filter(|rule| rule.definition.enabled != enabled)
            .cloned()
            .map(|mut rule| {
                rule.definition.enabled = enabled;
                rule
            })
            .collect();
        if rules.is_empty() {
            return;
        }
        let action = if enabled { "Activate" } else { "Deactivate" };
        let mut description = format!("{action} {} rules?", rules.len());
        if enabled {
            description
                .push_str(" Each match prepares a new instance and launches a model session.");
        }
        let modal = dialogs::dialog(&format!("{action} all rules"))
            .actions([
                tuicore::DialogAction::new("Ok")
                    .hotkey(KeySpec::plain('o'))
                    .on_trigger(move || Msg::SaveRules(rules.clone())),
                tuicore::DialogAction::new("Cancel")
                    .hotkey(KeySpec::plain('c'))
                    .on_trigger(|| Msg::Close),
            ])
            .host(Flex::column().child(
                "approval",
                tuicore::Paragraph::new(description),
                FlexItem::fit_content(),
            ));
        self.details_open = false;
        self.open_compact(Box::new(modal), ctx);
    }

    pub(in crate::app) fn save_rules(&mut self, rules: Vec<Rule>, ctx: &mut EventCtx<Msg>) {
        if !self.rule_action_available(ctx) {
            return;
        }
        self.rule_saves = rules;
        self.start_rule_autosave();
    }

    fn rule_action_available(&self, ctx: &mut EventCtx<Msg>) -> bool {
        if self.rule_save.is_some() || !self.rule_saves.is_empty() {
            ctx.notify(tuicore::Notification::warning(
                "Rule action unavailable",
                "Wait for rule saving to finish",
            ));
            return false;
        }
        true
    }

    pub(in crate::app) fn poll_rule_save(&mut self) -> bool {
        let result = match self.rule_save.as_mut() {
            Some(reply) => match reply.try_recv() {
                Ok(result) => result,
                Err(tokio::sync::oneshot::error::TryRecvError::Empty) => return false,
                Err(_) => Err("rule worker stopped".into()),
            },
            None => return false,
        };
        self.rule_save = None;
        if let Some(draft) = self.rule_toggle.take() {
            let toggle = draft.borrow_mut().pending_toggle.take();
            match result {
                Ok(rule) => {
                    {
                        let mut draft = draft.borrow_mut();
                        draft.rule.revision = rule.revision;
                        draft.rule.zellij_session = rule.zellij_session.clone();
                        if matches!(toggle, Some(PendingToggle::StartInstance(_))) {
                            draft.rule.definition.start_instance = rule.definition.start_instance;
                        }
                        if !draft.dirty {
                            draft.rule = rule.clone();
                        }
                        draft.saved = rule.clone();
                    }
                    self.service.poll_rules();
                    if matches!(toggle, Some(PendingToggle::Enabled(_))) {
                        self.notify(tuicore::Notification::success(
                            if rule.definition.enabled {
                                "Rule activated"
                            } else {
                                "Rule deactivated"
                            },
                            rule.definition.name,
                        ));
                    }
                }
                Err(error) => self.rule_save_error(error),
            }
            self.start_rule_autosave();
            return true;
        }
        if let Some(draft) = self.rule_autosave.take() {
            match result {
                Ok(rule) => {
                    {
                        let mut draft = draft.borrow_mut();
                        draft.rule.revision = rule.revision;
                        draft.saved = rule;
                    }
                    self.service.poll_rules();
                    if draft.borrow().dirty
                        && !self
                            .rule_autosaves
                            .iter()
                            .any(|queued| Rc::ptr_eq(queued, &draft))
                    {
                        self.rule_autosaves.push(draft);
                    }
                }
                Err(error) => self.rule_save_error(error),
            }
            self.start_rule_autosave();
            return true;
        }
        match result {
            Ok(rule) => {
                if let Some(editor) = &self.rule_editor {
                    let mut draft = editor.borrow_mut();
                    if draft.saved.definition.name == rule.definition.name {
                        draft.rule.revision = rule.revision;
                        draft.rule.zellij_session = rule.zellij_session.clone();
                        if !draft.dirty {
                            draft.rule = rule.clone();
                        }
                        draft.saved = rule.clone();
                    }
                }
                self.service.poll_rules();
                self.notify(tuicore::Notification::success(
                    if rule.definition.enabled {
                        "Rule activated"
                    } else {
                        "Rule deactivated"
                    },
                    rule.definition.name,
                ));
            }
            Err(error) => self.notify(tuicore::Notification::error("Cannot save rule", error)),
        }
        self.start_rule_autosave();
        true
    }
}
