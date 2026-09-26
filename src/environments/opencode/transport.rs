use std::{path::Path, process::Stdio, time::Duration};

use serde::de::DeserializeOwned;
use tokio::{io::AsyncReadExt, process::Command};

const LIMIT: u64 = 8 * 1024 * 1024;

pub(super) fn local_server(value: &str) -> Option<String> {
    let url = reqwest::Url::parse(value).ok()?;
    let host = url.host_str()?;
    if url.scheme() != "http"
        || !(host == "localhost"
            || host
                .parse::<std::net::IpAddr>()
                .is_ok_and(|ip| ip.is_loopback()))
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || url.path() != "/"
    {
        return None;
    }
    Some(url.as_str().trim_end_matches('/').to_owned())
}

pub(super) fn client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(2))
        .redirect(reqwest::redirect::Policy::none())
        .no_proxy()
        .build()
        .map_err(|error| error.to_string())
}

pub(super) async fn get<T: DeserializeOwned>(
    client: &reqwest::Client,
    server: &str,
    path: &str,
) -> Result<T, String> {
    get_optional(client, server, path)
        .await?
        .ok_or_else(|| format!("{server}: 404"))
}

pub(super) async fn get_optional<T: DeserializeOwned>(
    client: &reqwest::Client,
    server: &str,
    path: &str,
) -> Result<Option<T>, String> {
    let server = local_server(server).ok_or("OpenCode server must be local HTTP")?;
    let mut request = client.get(format!("{server}{path}"));
    if let Ok(password) = std::env::var("OPENCODE_SERVER_PASSWORD") {
        request = request.basic_auth(
            std::env::var("OPENCODE_SERVER_USERNAME").unwrap_or_else(|_| "opencode".into()),
            Some(password),
        );
    }
    let response = request
        .send()
        .await
        .map_err(|_| format!("{server}: unavailable"))?;
    if response.status() == reqwest::StatusCode::NOT_FOUND {
        return Ok(None);
    }
    let mut response = response
        .error_for_status()
        .map_err(|error| format!("{server}: {}", error.status().map_or(0, |s| s.as_u16())))?;
    let mut bytes = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| "OpenCode response failed")?
    {
        if bytes.len() + chunk.len() > LIMIT as usize {
            return Err("OpenCode response exceeds 8 MiB".into());
        }
        bytes.extend_from_slice(&chunk);
    }
    serde_json::from_slice(&bytes)
        .map(Some)
        .map_err(|_| "Invalid OpenCode response".into())
}

pub(super) async fn zellij(program: &Path, args: &[String]) -> Result<String, String> {
    let mut command = Command::new(program);
    command
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    let mut child = command.spawn().map_err(|_| "Zellij unavailable")?;
    let mut stdout = child
        .stdout
        .take()
        .ok_or("Zellij output unavailable")?
        .take(LIMIT + 1);
    let mut output = Vec::new();
    let result = tokio::time::timeout(Duration::from_secs(2), async {
        stdout
            .read_to_end(&mut output)
            .await
            .map_err(|error| error.to_string())?;
        if output.len() > LIMIT as usize {
            return Err("Zellij output exceeds 8 MiB".into());
        }
        let status = child.wait().await.map_err(|error| error.to_string())?;
        if !status.success() {
            return Err(format!("Zellij command failed ({status})"));
        }
        String::from_utf8(output).map_err(|_| "Invalid Zellij output".into())
    })
    .await;
    result.map_err(|_| "Zellij command timed out".to_owned())?
}
