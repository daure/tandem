use reqwest::Method;
use serde_json::{Value, json};

use super::{Observer, transport, v2, valid_id};
use crate::store::opencode::{PromptOutcome, PromptedSession, Session, SessionPrompt};
use crate::store::rules::DeliveryMode;

impl Observer {
    pub(crate) async fn background_server(&self, directory: &str) -> Result<String, String> {
        let observer = self.clone();
        let servers = tokio::task::spawn_blocking(move || observer.inventory().1)
            .await
            .map_err(|error| error.to_string())?;
        let scoped: Vec<_> = servers
            .iter()
            .filter(|(_, dirs)| dirs.contains(directory))
            .map(|(server, _)| server.clone())
            .collect();
        let candidates = if scoped.is_empty() {
            servers.into_keys().collect()
        } else {
            scoped
        };
        let client = transport::client()?;
        let mut available = Vec::new();
        for server in candidates {
            if transport::is_v2(&client, &server).await.unwrap_or(false) {
                available.push(server);
            }
        }
        match available.as_slice() {
            [server] => Ok(server.clone()),
            [] => Err("Clientless dispatch requires an available OpenCode V2 server".into()),
            _ => Err(
                "Workspace has multiple OpenCode V2 servers; clientless dispatch is ambiguous"
                    .into(),
            ),
        }
    }

    pub(crate) async fn create_background_session(
        &self,
        directory: &str,
        server: &str,
        input: &SessionPrompt,
        identity: Option<(&str, &str)>,
    ) -> Result<PromptedSession, String> {
        let client = transport::client()?;
        let mut body = json!({"location":{"directory":directory}});
        if let Some(model) = background_model(&client, server, directory, input).await? {
            body["model"] = model;
        }
        if let Some(agent) = &input.agent {
            body["agent"] = json!(agent);
        }
        if let Some((id, title)) = identity {
            body["id"] = json!(id);
            body["title"] = json!(title);
        }
        let mut receipt = PromptedSession {
            session_id: None,
            server: Some(server.into()),
            pane: None,
            prompt_outcome: PromptOutcome::NotSubmitted,
            error: None,
            creation_confirmed_absent: false,
        };
        let result = transport::mutate(&client, server, "/api/session", Method::POST, body).await;
        match result {
            Ok(Some(value))
                if value["data"]["id"].as_str().is_some_and(valid_id)
                    && value["data"]["location"]["directory"] == directory
                    && identity.is_none_or(|(id, _)| value["data"]["id"] == id) =>
            {
                receipt.session_id = value["data"]["id"].as_str().map(str::to_owned);
            }
            _ => {
                receipt.prompt_outcome = PromptOutcome::Uncertain;
                receipt.error = Some("Session creation is uncertain; initial prompt was not submitted. Inspect server history before replaying".into());
            }
        }
        Ok(receipt)
    }

    pub(crate) async fn submit_background_prompt(
        &self,
        session: &Session,
        input: &SessionPrompt,
        instructions: &str,
        delivery: DeliveryMode,
        existing: bool,
    ) -> PromptedSession {
        let mut receipt = PromptedSession {
            session_id: Some(session.id.clone()),
            server: Some(session.server.clone()),
            pane: None,
            prompt_outcome: PromptOutcome::NotSubmitted,
            error: None,
            creation_confirmed_absent: false,
        };
        let result = async {
            let client = transport::client()?;
            if !self.named_rule_session_exists(session).await? {
                return Err("OpenCode conversation is missing".to_owned());
            }
            if existing && delivery == DeliveryMode::Interrupt {
                transport::mutate(&client, &session.server, &format!("/api/session/{}/interrupt?resume=false", session.id), Method::POST, json!({})).await?;
            }
            if !instructions.is_empty() {
                transport::mutate(&client, &session.server, &format!("/api/experimental/session/{}/instructions/entries/tandem.services", session.id), Method::PUT, json!({"value":instructions})).await?;
            }
            if existing && let Some(model) = background_model(&client, &session.server, &session.directory, input).await? {
                transport::mutate(&client, &session.server, &format!("/api/session/{}/model", session.id), Method::POST, json!({"model":model})).await?;
            }
            receipt.prompt_outcome = PromptOutcome::Uncertain;
            let response = transport::mutate(&client, &session.server, &format!("/api/session/{}/prompt", session.id), Method::POST,
                json!({"text": input.launch.prompt, "delivery": if delivery == DeliveryMode::Queue {"queue"} else {"steer"}, "resume":true})).await?;
            if response.as_ref().is_none_or(|value| value["data"]["sessionID"] != session.id) {
                return Err("OpenCode returned no prompt admission receipt".into());
            }
            receipt.prompt_outcome = if delivery == DeliveryMode::Queue { PromptOutcome::Queued } else { PromptOutcome::Submitted };
            Ok::<_, String>(())
        }.await;
        if result.is_err() {
            receipt.error = Some(
                if receipt.prompt_outcome == PromptOutcome::Uncertain {
                    "Prompt delivery is uncertain; inspect this conversation before retrying"
                } else {
                    "Session setup failed; prompt was not submitted"
                }
                .into(),
            );
        }
        receipt
    }
}

async fn background_model(
    client: &reqwest::Client,
    server: &str,
    directory: &str,
    input: &SessionPrompt,
) -> Result<Option<Value>, String> {
    if let Some(selector) = input.launch.selector() {
        let (model, variant) = selector
            .split_once('#')
            .map_or((selector.as_str(), None), |(model, variant)| {
                (model, Some(variant))
            });
        let (provider, id) = model.split_once('/').ok_or("Invalid model selector")?;
        let mut model = json!({"providerID":provider, "id":id});
        if let Some(variant) = variant {
            model["variant"] = json!(variant);
        }
        return Ok(Some(model));
    }
    let Some(variant) = &input.launch.variant else {
        return Ok(None);
    };
    let mut url = reqwest::Url::parse("http://localhost/api/model/default").expect("static URL");
    url.query_pairs_mut()
        .append_pair("location[directory]", directory);
    let model = v2::envelope(
        client,
        server,
        &format!("{}?{}", url.path(), url.query().unwrap_or_default()),
    )
    .await?
    .ok_or("OpenCode default model is unavailable")?;
    let provider = model["providerID"]
        .as_str()
        .ok_or("Invalid default model provider")?;
    let id = model["id"]
        .as_str()
        .ok_or("Invalid default model identity")?;
    Ok(Some(
        json!({"providerID":provider, "id":id, "variant":variant}),
    ))
}
