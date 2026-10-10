use std::time::Duration;

use super::{Observer, tabs::Control, transport, valid_id};
use crate::store::opencode::{
    Pane, PromptOutcome, PromptedSession, Session, SessionPrompt, WhenBusy,
};

struct PromptClient {
    pane: Pane,
    control: Control,
    server: String,
}

impl Observer {
    pub(crate) async fn prompt_new_session(
        &self,
        directory: &str,
        name: &str,
        current: &str,
        input: &SessionPrompt,
        instructions: &str,
    ) -> Result<PromptedSession, String> {
        let existing = self.prompt_client(directory, current, None).await?;
        let client = match existing {
            Some(client) => client,
            None => {
                // Launch without input; the authenticated companion returns the delivery receipt.
                let pane = self.new_session(directory, name, current, None, None, None).await
                    .map_err(|error| format!("{error}; client launch may be uncertain, initial prompt was not submitted"))?;
                match self.wait_prompt_control(directory, &pane, false).await? {
                    Some((control, server)) => PromptClient { pane, control, server },
                    None => return Ok(PromptedSession {
                        session_id: None,
                        server: None,
                        pane,
                        prompt_outcome: PromptOutcome::NotSubmitted,
                        error: Some("OpenCode companion unavailable; run tandem opencode-setup and reopen the client. Initial prompt was not submitted".into()),
                    }),
                }
            }
        };
        self.send_prompt(
            client,
            current,
            serde_json::json!({
                "directory": directory,
                "initial_prompt": input.launch.prompt,
                "model": input.launch.selector(),
                "variant": input.launch.variant,
                "agent": input.agent,
                "instructions": instructions,
            }),
            None,
        )
        .await
    }

    pub(crate) async fn resolve_prompt_session(
        &self,
        id: &str,
        known: Vec<Session>,
    ) -> Result<Session, String> {
        if !valid_id(id) {
            return Err("Invalid OpenCode conversation ID".into());
        }
        let observer = self.clone();
        let servers =
            tokio::task::spawn_blocking(move || observer.inventory_with_sessions(&known).1)
                .await
                .map_err(|error| error.to_string())?;
        let client = transport::client()?;
        let mut found = None;
        for server in servers.into_keys() {
            if !transport::is_v2(&client, &server).await? {
                continue;
            }
            let Some(value) =
                super::v2::envelope(&client, &server, &format!("/api/session/{id}")).await?
            else {
                continue;
            };
            let directory = value["location"]["directory"]
                .as_str()
                .filter(|path| std::path::Path::new(path).is_absolute())
                .ok_or("OpenCode conversation has an invalid location")?;
            if value["id"] != id || found.is_some() {
                return Err("OpenCode conversation identity is ambiguous or mismatched".into());
            }
            found = Some(Session {
                id: id.into(),
                directory: directory.into(),
                server,
                ..Default::default()
            });
        }
        found.ok_or("OpenCode conversation is unavailable on known reachable V2 servers".into())
    }

    pub(crate) async fn prompt_existing_session(
        &self,
        session: &Session,
        name: &str,
        current: &str,
        input: &SessionPrompt,
        when_busy: WhenBusy,
        instructions: &str,
    ) -> Result<PromptedSession, String> {
        let client = match self
            .prompt_client(&session.directory, current, Some(session))
            .await?
        {
            Some(client) => client,
            None => {
                let pane = self
                    .attach_location(session, name, current, None)
                    .await
                    .map_err(|error| {
                        format!("{error}; client launch may be uncertain, prompt was not submitted")
                    })?;
                match self.wait_prompt_control(&session.directory, &pane, true).await? {
                    Some((control, server)) if server == session.server => PromptClient { pane, control, server },
                    _ => return Ok(PromptedSession {
                        session_id: Some(session.id.clone()), server: Some(session.server.clone()), pane,
                        prompt_outcome: PromptOutcome::NotSubmitted,
                        error: Some("OpenCode companion unavailable or server changed; prompt was not submitted. Run tandem opencode-setup and reopen the client".into()),
                    }),
                }
            }
        };
        self.send_prompt(
            client,
            current,
            serde_json::json!({
                "directory": session.directory,
                "sessionID": session.id,
                "initial_prompt": input.launch.prompt,
                "model": input.launch.selector(),
                "variant": input.launch.variant,
                "agent": input.agent,
                "when_busy": when_busy,
                "instructions": instructions,
            }),
            Some(&session.id),
        )
        .await
    }

    async fn prompt_client(
        &self,
        directory: &str,
        current: &str,
        target: Option<&Session>,
    ) -> Result<Option<PromptClient>, String> {
        let observer = self.clone();
        let path = directory.to_owned();
        let session = current.to_owned();
        let target = target.cloned();
        tokio::task::spawn_blocking(move || {
            let mut clients: Vec<_> = observer
                .presences()
                .into_iter()
                .filter(|presence| {
                    presence.directory == path
                        && presence.pane_id.is_some()
                        && presence.tab_control.as_ref().is_some_and(|control| {
                            if let Some(target) = &target {
                                control.session_prompts
                                    && presence.server == target.server
                                    && (control.session_tabs || presence.id == target.id)
                            } else {
                                control.prompted_sessions && control.session_tabs
                            }
                        })
                })
                .collect();
            clients.sort_by_key(|presence| {
                (
                    presence.zellij_session != session,
                    presence.zellij_session.clone(),
                    presence.pane_id,
                )
            });
            clients.into_iter().next().map(|presence| PromptClient {
                pane: Pane {
                    session: presence.zellij_session,
                    id: presence.pane_id.unwrap(),
                    tab_id: 0,
                    tab_name: String::new(),
                },
                control: presence.tab_control.unwrap(),
                server: presence.server,
            })
        })
        .await
        .map_err(|error| error.to_string())
    }

    async fn send_prompt(
        &self,
        client: PromptClient,
        current: &str,
        body: serde_json::Value,
        target_id: Option<&str>,
    ) -> Result<PromptedSession, String> {
        let PromptClient {
            pane,
            control,
            server,
        } = client;
        let directory = body["directory"]
            .as_str()
            .ok_or("OpenCode prompt directory is missing")?;
        let panes = self.list_panes(&pane.session).await?;
        let actual = panes
            .iter()
            .find(|actual| actual.id == pane.id && !actual.is_plugin && !actual.exited)
            .ok_or("OpenCode pane has closed; initial prompt was not submitted")?;
        let pane = Pane {
            tab_id: actual.tab_id,
            tab_name: actual.tab_name.clone(),
            ..pane
        };
        let server =
            transport::local_server(&server).ok_or("OpenCode server must be local HTTP")?;
        let observer = self.clone();
        let path = directory.to_owned();
        let target = pane.clone();
        let expected = control.clone();
        let expected_server = server.clone();
        let attached = tokio::task::spawn_blocking(move || {
            observer.presences().into_iter().any(|presence| {
                presence.directory == path
                    && presence.zellij_session == target.session
                    && presence.pane_id == Some(target.id)
                    && transport::local_server(&presence.server).as_deref()
                        == Some(expected_server.as_str())
                    && presence.tab_control.as_ref() == Some(&expected)
            })
        })
        .await
        .map_err(|error| error.to_string())?;
        if !attached {
            return Err("OpenCode client changed; initial prompt was not submitted".into());
        }
        let response = control
            .request_json(
                if target_id.is_some() {
                    "/sessions/prompt"
                } else {
                    "/sessions"
                },
                body,
            )
            .await;
        let mut outcome = match response.and_then(|value| {
            let receipt = parse_receipt(value, &server)?;
            if target_id.is_some_and(|id| receipt.0.as_deref() != Some(id)) {
                return Err("OpenCode returned a different conversation identity".into());
            }
            Ok(receipt)
        }) {
            Ok((session_id, prompt_outcome, error)) => PromptedSession {
                session_id,
                server: Some(server),
                pane,
                prompt_outcome,
                error,
            },
            Err(error) => PromptedSession {
                session_id: target_id.map(str::to_owned),
                server: Some(server),
                pane,
                prompt_outcome: PromptOutcome::Uncertain,
                error: Some(format!(
                    "{error}; session creation or prompt delivery may be uncertain. Inspect the client before retrying"
                )),
            },
        };
        if outcome.session_id.is_some()
            && let Err(error) = self
                .focus_pane(current, &outcome.pane, actual.is_floating)
                .await
        {
            outcome.error = Some(match outcome.error {
                Some(previous) => format!("{previous}; {error}"),
                None => error,
            });
        }
        Ok(outcome)
    }

    async fn wait_prompt_control(
        &self,
        directory: &str,
        pane: &Pane,
        existing: bool,
    ) -> Result<Option<(Control, String)>, String> {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(15);
        loop {
            let observer = self.clone();
            let directory = directory.to_owned();
            let pane = pane.clone();
            let control = tokio::task::spawn_blocking(move || {
                observer
                    .presences()
                    .into_iter()
                    .find(|presence| {
                        presence.directory == directory
                            && presence.zellij_session == pane.session
                            && presence.pane_id == Some(pane.id)
                            && presence.tab_control.as_ref().is_some_and(|control| {
                                if existing {
                                    control.session_prompts
                                } else {
                                    control.prompted_sessions
                                }
                            })
                    })
                    .map(|presence| (presence.tab_control.unwrap(), presence.server))
            })
            .await
            .map_err(|error| error.to_string())?;
            if control.is_some() || tokio::time::Instant::now() >= deadline {
                return Ok(control);
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }
}

type Receipt = (Option<String>, PromptOutcome, Option<String>);

fn parse_receipt(value: serde_json::Value, server: &str) -> Result<Receipt, String> {
    if value["server"]
        .as_str()
        .and_then(transport::local_server)
        .as_deref()
        != Some(server)
    {
        return Err("OpenCode returned a different server identity".into());
    }
    let session_id = match &value["session_id"] {
        serde_json::Value::Null => None,
        serde_json::Value::String(id) if valid_id(id) => Some(id.clone()),
        _ => return Err("OpenCode returned an invalid session identity".into()),
    };
    let prompt_outcome: PromptOutcome = serde_json::from_value(value["prompt_outcome"].clone())
        .map_err(|_| "OpenCode returned an invalid delivery outcome")?;
    if prompt_outcome.accepted() && session_id.is_none() {
        return Err("OpenCode returned no session for the submitted prompt".into());
    }
    let error = value["error"].as_str().map(|error| {
        error
            .chars()
            .filter(|character| !character.is_control())
            .take(512)
            .collect()
    });
    Ok((session_id, prompt_outcome, error))
}
