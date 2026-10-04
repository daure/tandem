use ratatui::text::Text;
use tuicore::{
    Dropdown, DropdownCommitMode, DropdownSearchMode, DropdownVariant, EventCtx, Flex, FlexItem,
    Notification, Paragraph,
};

use super::{Command, Msg, Target};
use crate::app::{App, Intent, dialogs, rows::Row};

impl App {
    pub(in crate::app) fn acceptance_action(
        &mut self,
        requested: Target,
        command: Command,
        ctx: &mut EventCtx<Msg>,
    ) {
        let context = self.pages_mut().acceptance_context();
        let rules = self.pages_mut().rules_state();
        let snapshot = rules.borrow();
        let acceptance = snapshot
            .acceptances
            .iter()
            .find(|row| row.id == requested.acceptance.id)
            .unwrap_or(&requested.acceptance);
        let mut target = Target::new(
            acceptance,
            snapshot.workspaces.get(&acceptance.id),
            &context.borrow(),
            true,
        )
        .with_report(snapshot.reports.get(&acceptance.id));
        drop(snapshot);
        target.selected = requested
            .selected
            .as_ref()
            .and_then(|row| target.conversations.iter().find(|other| other.id == row.id))
            .cloned();
        if requested.selected.is_some() && target.selected.is_none() {
            ctx.notify(Notification::warning(
                "Conversation unavailable",
                "Refresh the conversation list and try again",
            ));
            return;
        }
        if !target.enabled(command) && command != Command::Details {
            ctx.notify(Notification::warning(
                "Action unavailable",
                "Refresh the event and instance inventory before retrying",
            ));
            return;
        }
        match command {
            Command::Report => {
                let report = super::super::markdown::Markdown::new(
                    "report",
                    Ok(self.service.read_event_report(target.acceptance.id)),
                    |report| {
                        format!(
                            "# {}\n\n{}\n\nAcceptance #{} · event #{}\n\nCleanup: {:?}{}\n\n---\n\n{}",
                            report.details.title,
                            report.details.summary,
                            report.details.acceptance_id,
                            report.details.event_sequence,
                            report.details.cleanup_state,
                            report
                                .details
                                .cleanup_error
                                .map(|error| format!("\n\n{error}"))
                                .unwrap_or_default(),
                            report.markdown
                        )
                    },
                );
                self.intent = None;
                self.open_compact(
                    Box::new(
                        dialogs::dialog("Acceptance report").host(Flex::column().child(
                            "report",
                            report,
                            FlexItem::fill(1),
                        )),
                    ),
                    ctx,
                );
            }
            Command::Delete => self.handle_message(
                Msg::DeleteEvents(crate::store::events::Deletion::Acceptance(
                    target.acceptance.id,
                )),
                ctx,
            ),
            Command::Rule => self.handle_message(Msg::FocusRule(target.acceptance.rule_name), ctx),
            Command::Routes => {
                let routes = self.instance_routes(&target.acceptance.instance);
                if routes.is_empty() {
                    ctx.notify(Notification::warning(
                        "Routes unavailable",
                        "This instance has no service routes",
                    ));
                } else {
                    self.handle_message(Msg::Close, ctx);
                    self.open_routes(routes, ctx);
                }
            }
            Command::Instance => {
                self.handle_message(Msg::AcceptanceInstance(target.acceptance.instance), ctx)
            }
            Command::CreateInstance => self.confirm_acceptance_recreation(&target, None, ctx),
            Command::PurgeInstance => {
                let name = target.acceptance.instance;
                self.intent = Some(Intent::Purge(name.clone()));
                self.open(dialogs::confirm_purge(&name), ctx);
            }
            Command::NewSession => {
                if let Some(row) = &target.instance {
                    self.create_opencode_session(row, ctx);
                }
            }
            Command::Details => {
                if let Some(row) = &target.selected {
                    self.open_opencode_dialog(row, ctx);
                }
            }
            Command::Session => {
                if target.selected.is_none() {
                    let parent = format!("acceptance:{}", target.acceptance.id);
                    let mut choices: Vec<_> = target
                        .conversations
                        .iter()
                        .filter(|row| {
                            row.opencode.is_some() && row.parent.as_deref() == Some(&parent)
                        })
                        .cloned()
                        .collect();
                    if choices.len() > 1 {
                        self.open_acceptance_picker(target, choices, ctx);
                        return;
                    }
                    target.selected = choices.pop();
                }
                let Some(row) = &target.selected else {
                    return;
                };
                if target.instance.is_none() {
                    if !target.available {
                        ctx.notify(Notification::warning(
                            "Instance inventory unavailable",
                            "Refresh the instance inventory before recreating its workspace",
                        ));
                        return;
                    }
                    if let Some(crate::app::opencode::Target::Session { id, .. }) = &row.opencode {
                        self.confirm_acceptance_recreation(&target, Some(id.clone()), ctx);
                    } else {
                        ctx.notify(Notification::warning(
                            "Instance unavailable",
                            "Recreate its workspace before opening a new client",
                        ));
                    }
                } else {
                    self.activate_opencode(row, ctx);
                }
            }
            _ => {}
        }
    }

    fn confirm_acceptance_recreation(
        &mut self,
        target: &Target,
        conversation: Option<String>,
        ctx: &mut EventCtx<Msg>,
    ) {
        let acceptance = target.acceptance.clone();
        let title = if conversation.is_some() {
            "Recreate instance and reopen conversation"
        } else {
            "Recreate instance"
        };
        let modal = dialogs::dialog(title).actions([
            tuicore::DialogAction::new("Ok").hotkey(tuicore::KeySpec::plain('o'))
                .on_trigger(move || Msg::RecreateAcceptanceConfirmed(Box::new(acceptance.clone()), conversation.clone())),
            tuicore::DialogAction::new("Cancel").hotkey(tuicore::KeySpec::plain('c')).on_trigger(|| Msg::Close),
        ]).host(Flex::column().child("warning", Paragraph::new(format!(
            "Recreate {} from {}?\nThis creates a fresh workspace; deleted files, uncommitted changes and runtime data are not restored.\nSaved conversations are preserved. The original rule prompt is not resent.",
            target.acceptance.instance, target.acceptance.rule.definition.template)), FlexItem::fit_content()));
        self.intent = None;
        self.open_compact(Box::new(modal), ctx);
    }

    fn open_acceptance_picker(
        &mut self,
        target: Target,
        choices: Vec<Row>,
        ctx: &mut EventCtx<Msg>,
    ) {
        let original = target.acceptance.session_id.clone();
        let selected = self.acceptance_selection.clone();
        let mut dropdown = Dropdown::single_rich(choices, |row: &Row| row.id.clone(),
            |row: &Row| row.label.lines().next().unwrap_or_default().to_owned(),
            move |row, _, _| {
                let original = matches!(&row.opencode, Some(crate::app::opencode::Target::Session { id, .. }) if Some(id) == original.as_ref());
                Text::raw(format!("{}{}", row.label.lines().next().unwrap_or_default(), if original { " · original" } else { "" }))
            })
            .variant(DropdownVariant::Filled).search_mode(DropdownSearchMode::Fuzzy)
            .commit_mode(DropdownCommitMode::Explicit).centered(true).show_field_when_open(false)
            .max_popup_width(u16::MAX).max_popup_height(12)
            .on_select(move |ids| {
                if let Some(row) = target.conversations.iter().find(|row| ids.contains(&row.id)) {
                    let mut target = target.clone(); target.selected = Some(row.clone());
                    *selected.borrow_mut() = Some(target);
                }
            });
        dropdown.open_with_context(ctx);
        self.intent = None;
        self.open_compact(
            Box::new(
                dialogs::dialog("OpenCode conversations").host(Flex::column().child(
                    "conversation-picker",
                    dropdown,
                    FlexItem::fill(1),
                )),
            ),
            ctx,
        );
    }

    pub(in crate::app) fn poll_acceptance_recreation(&mut self) -> bool {
        let result = match self
            .acceptance_recreation
            .as_mut()
            .map(|reply| reply.try_recv())
        {
            Some(Ok(result)) => result,
            Some(Err(tokio::sync::oneshot::error::TryRecvError::Closed)) => {
                Err("Recreation worker stopped".into())
            }
            _ => return false,
        };
        self.acceptance_recreation = None;
        self.notify(match result {
            Ok(()) => {
                Notification::success("Instance recreated", "Saved conversation history preserved")
            }
            Err(error) => Notification::error("Cannot recreate instance", error),
        });
        true
    }
}
