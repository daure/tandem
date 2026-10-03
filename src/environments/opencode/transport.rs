use std::{path::Path, process::Stdio, time::Duration};

use serde::de::DeserializeOwned;
use tokio::{
    io::{AsyncRead, AsyncReadExt},
    process::Command,
};

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
    request(client, server, path, reqwest::Method::GET).await
}

pub(super) async fn delete(
    client: &reqwest::Client,
    server: &str,
    path: &str,
) -> Result<(), String> {
    match request::<bool>(client, server, path, reqwest::Method::DELETE).await? {
        Some(true) | None => Ok(()),
        Some(false) => Err("OpenCode refused session deletion".into()),
    }
}

async fn request<T: DeserializeOwned>(
    client: &reqwest::Client,
    server: &str,
    path: &str,
    method: reqwest::Method,
) -> Result<Option<T>, String> {
    if is_v2(client, server).await? {
        let value = if method == reqwest::Method::DELETE {
            let path = path.split('?').next().unwrap_or(path);
            raw(client, server, &format!("/api{path}"), method).await?
        } else {
            super::v2::read(client, server, path).await?
        };
        return value
            .map(serde_json::from_value)
            .transpose()
            .map_err(|_| "Invalid OpenCode V2 response".into());
    }
    raw(client, server, path, method)
        .await?
        .map(serde_json::from_value)
        .transpose()
        .map_err(|_| "Invalid OpenCode response".into())
}

pub(super) async fn is_v2(client: &reqwest::Client, server: &str) -> Result<bool, String> {
    Ok(raw(client, server, "/api/info", reqwest::Method::GET)
        .await?
        .is_some_and(|value| {
            value["version"]
                .as_str()
                .is_some_and(|version| version.starts_with("2."))
        }))
}

pub(super) fn password(server: &str) -> Option<String> {
    let state = dirs::state_dir()?;
    let daemons = std::env::var_os("OC_DAEMON_STATE")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| state.join("opencode-daemon"));
    let port = reqwest::Url::parse(server).ok()?.port()?;
    for directory in super::entries(&daemons) {
        if super::read_small(&directory.join("port"))
            .and_then(|text| text.trim().parse::<u16>().ok())
            == Some(port)
            && let Some(secret) = private_text(&directory.join("password"))
        {
            return Some(secret.trim().to_owned());
        }
    }
    if let Some(text) = private_text(&state.join("opencode/service.json"))
        && let Ok(value) = serde_json::from_str::<serde_json::Value>(&text)
        && value["url"].as_str().and_then(local_server).as_deref() == Some(server)
        && let Some(password) = value["password"].as_str()
    {
        return Some(password.into());
    }
    std::env::var("OPENCODE_PASSWORD")
        .or_else(|_| std::env::var("OPENCODE_SERVER_PASSWORD"))
        .ok()
}

fn private_text(path: &Path) -> Option<String> {
    use std::os::unix::fs::MetadataExt;
    let metadata = std::fs::symlink_metadata(path).ok()?;
    if !metadata.is_file()
        || metadata.mode() & 0o077 != 0
        || metadata.uid() != unsafe { libc::geteuid() }
    {
        return None;
    }
    super::read_small(path)
}

pub(super) async fn raw(
    client: &reqwest::Client,
    server: &str,
    path: &str,
    method: reqwest::Method,
) -> Result<Option<serde_json::Value>, String> {
    let server = local_server(server).ok_or("OpenCode server must be local HTTP")?;
    let mut request = client.request(method, format!("{server}{path}"));
    if let Some(password) = password(&server) {
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
    if response.status() == reqwest::StatusCode::NO_CONTENT {
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
    let action = args
        .windows(2)
        .find(|pair| pair[0] == "action")
        .map(|pair| pair[1].as_str())
        .or_else(|| args.first().map(String::as_str))
        .unwrap_or("command");
    let mut command = Command::new(program);
    command
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    let mut child = command.spawn().map_err(|_| "Zellij unavailable")?;
    let stdout = child.stdout.take().ok_or("Zellij output unavailable")?;
    let stderr = child
        .stderr
        .take()
        .ok_or("Zellij error output unavailable")?;
    let result = tokio::time::timeout(Duration::from_secs(2), async {
        let (output, stderr, status) = tokio::try_join!(
            zellij_output(stdout, LIMIT),
            zellij_output(stderr, 64 * 1024),
            async { child.wait().await.map_err(|error| error.to_string()) },
        )?;
        let stderr = String::from_utf8_lossy(&stderr);
        if action == "list-sessions"
            && status.code() == Some(1)
            && output.is_empty()
            && stderr.trim() == "No active zellij sessions found."
        {
            return Ok(String::new());
        }
        let succeeded =
            status.success() || status.code() == Some(2) && focus_unchanged(action, args, &stderr);
        if !succeeded {
            let detail = stderr
                .chars()
                .filter(|character| !character.is_control() || matches!(character, '\n' | '\t'))
                .collect::<String>();
            let error = format!("Zellij {action} failed ({status}): {}", detail.trim());
            crate::diagnostics::record_error(
                &format!("OpenCode Zellij action {action}"),
                &std::io::Error::other(error.clone()),
            );
            let mut message = error.chars().take(2048).collect::<String>();
            if error.chars().count() > 2048 {
                message.push_str("… (see Tandem diagnostic log)");
            }
            return Err(message);
        }
        String::from_utf8(output).map_err(|_| "Invalid Zellij output".into())
    })
    .await;
    result.map_err(|_| format!("Zellij {action} timed out"))?
}

fn focus_unchanged(action: &str, args: &[String], stderr: &str) -> bool {
    match action {
        "hide-floating-panes" | "show-floating-panes" => true,
        "focus-pane-id" => args
            .last()
            .and_then(|pane| pane.strip_prefix("terminal_"))
            .and_then(|id| id.parse::<u32>().ok())
            .is_some_and(|id| stderr.trim() == format!("Pane Terminal({id}) is already focused")),
        _ => false,
    }
}

async fn zellij_output(stream: impl AsyncRead + Unpin, limit: u64) -> Result<Vec<u8>, String> {
    let mut bytes = Vec::new();
    stream
        .take(limit + 1)
        .read_to_end(&mut bytes)
        .await
        .map_err(|error| error.to_string())?;
    if bytes.len() as u64 > limit {
        return Err(format!("Zellij output exceeds {limit} bytes"));
    }
    Ok(bytes)
}
