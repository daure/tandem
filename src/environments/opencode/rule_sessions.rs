use super::{Observer, navigation::new_client_command, prompted::PromptClient, transport, v2};
use crate::store::{
    opencode::{PromptedSession, Session},
    rules::Acceptance,
};

impl Observer {
    pub(crate) async fn named_rule_session_exists(
        &self,
        session: &Session,
    ) -> Result<bool, String> {
        if !super::valid_id(&session.id) {
            return Err("Invalid named conversation identity".into());
        }
        let client = transport::client()?;
        if !transport::is_v2(&client, &session.server).await? {
            return Err("Named conversations require an available OpenCode V2 server".into());
        }
        let Some(value) = v2::envelope(
            &client,
            &session.server,
            &format!("/api/session/{}", session.id),
        )
        .await?
        else {
            return Ok(false);
        };
        if value["id"] != session.id || value["location"]["directory"] != session.directory {
            return Err(
                "Named conversation workspace or identity changed; inspect before dispatching"
                    .into(),
            );
        }
        Ok(true)
    }

    pub(crate) async fn prepare_named_rule_prompt(
        &self,
        directory: &str,
        name: &str,
        current: &str,
        session: Option<&Session>,
        focus: bool,
    ) -> Result<PromptClient, String> {
        let client = match self.prompt_client(directory, current, session).await? {
            Some(client) => client,
            None => {
                let mut command = vec![
                    "env".into(),
                    "TANDEM_INITIAL_PROMPT=".into(),
                    "TANDEM_SESSION_INSTRUCTIONS=".into(),
                    "TANDEM_SESSION_MODEL=".into(),
                    "TANDEM_SESSION_VARIANT=".into(),
                ];
                command.extend(match session {
                    Some(session) => vec![
                        "opencode-station".into(),
                        "connect".into(),
                        session.server.clone(),
                        directory.into(),
                        "--session".into(),
                        session.id.clone(),
                    ],
                    None => new_client_command(directory),
                });
                let pane = self
                    .launch_panel(directory, name, current, None, &command, focus)
                    .await?;
                let (control, server) = self.wait_prompt_control(directory, &pane, session.is_some()).await?
                    .ok_or("OpenCode companion unavailable; run tandem opencode-setup and restart the client")?;
                PromptClient {
                    pane,
                    control,
                    server,
                }
            }
        };
        if !client.control.named_rule_sessions {
            return Err("OpenCode companion lacks named-session support; run tandem opencode-setup and restart the client".into());
        }
        if session.is_some_and(|session| session.server != client.server) {
            return Err("Named conversation server changed; prompt was not submitted".into());
        }
        Ok(client)
    }

    pub(crate) async fn send_named_rule_prompt(
        &self,
        client: PromptClient,
        current: &str,
        session: &Session,
        acceptance: &Acceptance,
        instructions: &str,
        creating: bool,
    ) -> Result<PromptedSession, String> {
        let rule = acceptance.launch_definition();
        let prompt = acceptance.session_prompt(creating)?;
        let mut body = serde_json::json!({
            "directory": session.directory, "initial_prompt": prompt,
            "model": rule.session_launch().selector(), "variant": rule.variant,
            "instructions": instructions, "when_busy": rule.delivery_mode,
            "focus": rule.focus_pane,
        });
        if creating {
            body["create_id"] = serde_json::json!(session.id);
            body["title"] = serde_json::json!(rule.session_name);
        } else {
            body["sessionID"] = serde_json::json!(session.id);
        }
        self.send_prompt(
            client,
            current,
            body,
            (!creating).then_some(session.id.as_str()),
        )
        .await
    }
}
