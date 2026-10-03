use std::{path::Path, time::Duration};

use serde::Deserialize;

use super::{Observer, Pane, transport, valid_id};

#[derive(Clone, Deserialize, serde::Serialize)]
pub(super) struct Control {
    server: String,
    token: String,
}

impl Control {
    pub(super) async fn request(
        &self,
        path: &str,
        body: serde_json::Value,
    ) -> Result<String, String> {
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
        result["id"]
            .as_str()
            .filter(|id| valid_id(id))
            .map(str::to_owned)
            .ok_or_else(|| {
                "OpenCode tab action returned no session; check the client before retrying".into()
            })
    }
}

impl Observer {
    pub(crate) async fn new_session_tab(
        &self,
        directory: &str,
        current: &str,
        destination: Option<&Pane>,
    ) -> Result<Option<Pane>, String> {
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
        control
            .request("/tabs", serde_json::json!({"directory": directory}))
            .await?;
        let pane = Pane {
            tab_id: actual.tab_id,
            tab_name: actual.tab_name.clone(),
            ..destination.clone()
        };
        self.focus_pane(current, &pane, actual.is_floating).await?;
        Ok(Some(pane))
    }
}
