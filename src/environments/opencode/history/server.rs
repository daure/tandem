use std::process::Stdio;

use tokio::{
    io::{AsyncBufReadExt, AsyncReadExt, BufReader},
    process::{Child, Command},
};

pub(super) async fn start(directory: &str) -> Result<(Child, String), String> {
    let mut child = Command::new("opencode")
        .args(["serve", "--hostname", "127.0.0.1", "--port", "0"])
        .env("OPENCODE_PURE", "1")
        .env("OPENCODE_DISABLE_AUTOUPDATE", "1")
        .current_dir(directory)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .map_err(|error| format!("cannot start OpenCode cleanup server: {error}"))?;
    let output = child
        .stdout
        .take()
        .ok_or("OpenCode server output unavailable")?;
    let mut lines = BufReader::new(output.take(64 * 1024)).lines();
    while let Some(line) = lines.next_line().await.map_err(|error| error.to_string())? {
        if let Some(url) = line.strip_prefix("opencode server listening on ") {
            let url = super::super::transport::local_server(url.trim())
                .ok_or("OpenCode cleanup server returned an invalid address")?;
            return Ok((child, url));
        }
    }
    Err("OpenCode cleanup server exited without a listening address".into())
}
