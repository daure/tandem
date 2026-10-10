use std::{path::Path, time::Duration};

use serde::Deserialize;

use super::{Observer, Pane, transport, valid_id};

#[derive(Clone, PartialEq, Eq, Deserialize, serde::Serialize)]
pub(super) struct Control {
    server: String,
    token: String,
    #[serde(default)]
    pub(super) prompted_sessions: bool,
    #[serde(default)]
    pub(super) session_prompts: bool,
    #[serde(default)]
    pub(super) session_tabs: bool,
}

pub(super) enum CloseOutcome {
    LegacyPane,
    SiblingsRemain,
    ClientEmpty,
}

impl Control {
    pub(super) async fn request(
        &self,
        path: &str,
        body: serde_json::Value,
    ) -> Result<String, String> {
        let result = self.request_json(path, body).await?;
        result["id"]
            .as_str()
            .filter(|id| valid_id(id))
            .map(str::to_owned)
            .ok_or_else(|| {
                "OpenCode tab action returned no session; check the client before retrying".into()
            })
    }

    pub(super) async fn request_json(
        &self,
        path: &str,
        body: serde_json::Value,
    ) -> Result<serde_json::Value, String> {
        let server = transport::local_server(&self.server)
            .ok_or("OpenCode tab control must be local HTTP")?;
        let mut response = transport::client()?
            .post(format!("{server}{path}"))
            .timeout(Duration::from_secs(12))
            .bearer_auth(&self.token)
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .body(body.to_string())
            .send()
            .await
            .map_err(|_| "OpenCode tab request failed; check the client before retrying")?;
        let succeeded = response.status().is_success();
        let mut bytes = Vec::new();
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|_| "OpenCode tab response failed; check the client before retrying")?
        {
            if bytes.len() + chunk.len() > 16_384 {
                return Err("OpenCode tab response is too large".into());
            }
            bytes.extend_from_slice(&chunk);
        }
        let result: serde_json::Value = serde_json::from_slice(&bytes)
            .map_err(|_| "Invalid OpenCode tab response; check the client before retrying")?;
        if !succeeded {
            return Err(result["error"]
                .as_str()
                .unwrap_or("OpenCode refused the tab action")
                .chars()
                .filter(|character| !character.is_control())
                .take(512)
                .collect());
        }
        Ok(result)
    }
}

impl Observer {
    pub(super) async fn close_session_tab(
        &self,
        id: &str,
        pane: &Pane,
    ) -> Result<CloseOutcome, String> {
        let observer = self.clone();
        let target = pane.clone();
        let session_id = id.to_owned();
        let control = tokio::task::spawn_blocking(move || {
            let presences = observer.presences();
            let attached = |presence: &&super::Presence| {
                presence.zellij_session == target.session && presence.pane_id == Some(target.id)
            };
            let presence = presences
                .iter()
                .filter(attached)
                .find(|presence| presence.id == session_id)
                .ok_or("The OpenCode tab closed or changed; refresh and try again")?;
            let siblings = presences.iter().filter(attached)
                .any(|other| !other.id.is_empty() && other.id != session_id);
            if presence.tab_index.is_none() && !siblings {
                return Ok(None);
            }
            if siblings && presence.tab_control.is_none() {
                return Err("Cannot close one tab without companion tab control; run tandem opencode-setup and reopen the client".to_owned());
            }
            Ok(presence.tab_control.clone())
        })
        .await
        .map_err(|error| error.to_string())??;
        let Some(control) = control else {
            return Ok(CloseOutcome::LegacyPane);
        };
        let panes = self.list_panes(&pane.session).await?;
        if !panes
            .iter()
            .any(|actual| actual.id == pane.id && !actual.is_plugin && !actual.exited)
        {
            return Err("OpenCode pane has closed; refresh and try again".into());
        }
        let closed = control
            .request("/tabs/close", serde_json::json!({"sessionID": id}))
            .await?;
        if closed != id {
            return Err("OpenCode returned a different conversation; refresh and try again".into());
        }
        self.wait_for_closed_tab(id, pane).await
    }

    async fn wait_for_closed_tab(&self, id: &str, pane: &Pane) -> Result<CloseOutcome, String> {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(6);
        loop {
            let observer = self.clone();
            let target = pane.clone();
            let id = id.to_owned();
            let closed = tokio::task::spawn_blocking(move || {
                let presences = observer.presences();
                let attached: Vec<_> = presences
                    .iter()
                    .filter(|presence| {
                        presence.zellij_session == target.session
                            && presence.pane_id == Some(target.id)
                    })
                    .collect();
                if attached.is_empty() || attached.iter().any(|presence| presence.id == id) {
                    return None;
                }
                Some(if attached.iter().any(|presence| !presence.id.is_empty()) {
                    CloseOutcome::SiblingsRemain
                } else {
                    CloseOutcome::ClientEmpty
                })
            })
            .await
            .map_err(|error| error.to_string())?;
            if let Some(closed) = closed {
                return Ok(closed);
            }
            if tokio::time::Instant::now() >= deadline {
                return Err(
                    "OpenCode tab closure is not yet observed; refresh and check the client".into(),
                );
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    }

    pub(crate) async fn new_session_tab(
        &self,
        directory: &str,
        current: &str,
        destination: Option<&Pane>,
        instructions: Option<&str>,
    ) -> Result<Option<crate::store::opencode::SessionLocation>, String> {
        let Some(destination) = destination else {
            return Ok(None);
        };
        if current.is_empty() {
            return Err("Run Tandem inside Zellij to create an OpenCode session".into());
        }
        let observer = self.clone();
        let target = destination.clone();
        let path = directory.to_owned();
        let control = tokio::task::spawn_blocking(move || {
            if !Path::new(&path).is_absolute() || !Path::new(&path).is_dir() {
                return Err("OpenCode workspace directory is unavailable".to_owned());
            }
            Ok(observer
                .presences()
                .into_iter()
                .find(|presence| {
                    presence.zellij_session == target.session && presence.pane_id == Some(target.id)
                })
                .and_then(|presence| presence.tab_control))
        })
        .await
        .map_err(|error| error.to_string())??;
        let Some(control) = control else {
            return Ok(None);
        };
        let panes = self.list_panes(&destination.session).await?;
        let actual = panes
            .iter()
            .find(|pane| pane.id == destination.id && !pane.is_plugin && !pane.exited)
            .ok_or("OpenCode pane has closed; refresh and try again")?;
        let session_id = control
            .request(
                "/tabs",
                serde_json::json!({"directory": directory, "instructions": instructions}),
            )
            .await?;
        let pane = Pane {
            tab_id: actual.tab_id,
            tab_name: actual.tab_name.clone(),
            ..destination.clone()
        };
        self.focus_pane(current, &pane, actual.is_floating).await?;
        Ok(Some(crate::store::opencode::SessionLocation {
            session_id: Some(session_id),
            pane,
        }))
    }
}
