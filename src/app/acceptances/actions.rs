use tuicore::{EventCtx, Flex, FlexItem, Notification, Paragraph, TextInput, TextareaInput};

use super::{Command, Msg, Target};
use crate::app::{App, Intent, dialogs};

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
            Command::RenameSession | Command::DeleteSession => {
                if let Some(crate::app::opencode::Target::Session { id, .. }) = target
                    .selected
                    .as_ref()
                    .and_then(|row| row.opencode.as_ref())
                {
                    let action = if command == Command::RenameSession {
                        crate::app::opencode::SessionAction::Rename
                    } else {
                        crate::app::opencode::SessionAction::Delete
                    };
                    self.request_opencode_session_action(id.clone(), action, ctx);
                }
            }
            Command::Report => {
                let report = super::super::markdown::Markdown::readonly(
                    "report",
                    Ok(self.service.read_event_report(target.acceptance.id)),
                    |report| report.markdown,
                );
                let details = target.report.as_ref();
                let metadata = format!(
                    "Acceptance #{} · event #{}{}",
                    target.acceptance.id,
                    target.acceptance.event_sequence,
                    details
                        .map(|report| format!(
                            " · Cleanup: {:?}{}",
                            report.cleanup_state,
                            report
                                .cleanup_error
                                .as_ref()
                                .map(|error| format!("\n{error}"))
                                .unwrap_or_default(),
                        ))
                        .unwrap_or_default(),
                );
                self.intent = None;
                self.open(
                    Box::new(
                        dialogs::dialog("Acceptance report").host(
                            Flex::column()
                                .child(
                                    "title",
                                    TextInput::new().panel("Title").disabled(true).value(
                                        details
                                            .map(|report| report.title.as_str())
                                            .unwrap_or_default(),
                                    ),
                                    FlexItem::fit_content(),
                                )
                                .child(
                                    "summary",
                                    TextareaInput::new()
                                        .panel("Summary")
                                        .disabled(true)
                                        .min_rows(2)
                                        .max_rows(8)
                                        .value(
                                            details
                                                .map(|report| report.summary.as_str())
                                                .unwrap_or_default(),
                                        ),
                                    FlexItem::fit_content(),
                                )
                                .child(
                                    "metadata",
                                    Paragraph::new(metadata),
                                    FlexItem::fit_content(),
                                )
                                .child("report", report, FlexItem::fill(1)),
                        ),
                    ),
                    ctx,
                );
                self.details_open = true;
                self.resize_details_dialog();
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
                    let view = if self.events_active {
                        crate::app::opencode::CreationView::Events {
                            state: self.pages_mut().event_focus(),
                            acceptance: target.acceptance.id,
                        }
                    } else {
                        crate::app::opencode::CreationView::Instances(self.instances.clone())
                    };
                    self.create_opencode_session_at(row, view, ctx);
                }
            }
            Command::Details => {
                if let Some(row) = &target.selected {
                    self.open_opencode_dialog(row, ctx);
                }
            }
            Command::ClosePanel => {
                if let Some(row) = &target.selected {
                    self.close_opencode(row, ctx);
                }
            }
            Command::Session => {
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
