use std::{io::Read, os::unix::fs::PermissionsExt, process::Stdio};

use tokio::{
    io::{AsyncBufReadExt, AsyncReadExt, BufReader},
    process::{Child, Command},
};

pub(super) async fn start(
    directory: &str,
) -> Result<(Child, String, tempfile::TempDir, tempfile::TempDir), String> {
    let root = tempfile::tempdir().map_err(|error| error.to_string())?;
    let data = dirs::data_dir().ok_or("OpenCode data directory unavailable")?;
    let config = root.path().join("config/opencode");
    std::fs::create_dir_all(&config).map_err(|error| error.to_string())?;
    let mut bytes = [0u8; 32];
    std::fs::File::open("/dev/urandom")
        .and_then(|mut file| file.read_exact(&mut bytes))
        .map_err(|error| error.to_string())?;
    let password = bytes
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    let mut child = Command::new("opencode")
        .args(["serve", "--hostname", "127.0.0.1", "--port", "0"])
        .env("OPENCODE_PURE", "1")
        .env("OPENCODE_DISABLE_AUTOUPDATE", "1")
        .env("HOME", root.path())
        .env("XDG_DATA_HOME", data)
        .env("XDG_CONFIG_HOME", root.path().join("config"))
        .env("XDG_CACHE_HOME", root.path().join("cache"))
        .env("XDG_STATE_HOME", root.path().join("state"))
        .env("OPENCODE_CONFIG_DIR", config)
        .env_remove("OPENCODE_CONFIG")
        .env(
            "OPENCODE_CONFIG_CONTENT",
            "{\"autoupdate\":false,\"plugin\":[]}",
        )
        .env("OPENCODE_DISABLE_PROJECT_CONFIG", "1")
        .env("OPENCODE_DISABLE_MODELS_FETCH", "1")
        .env("OPENCODE_PASSWORD", &password)
        .env("OPENCODE_SERVER_PASSWORD", &password)
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
        if let Some(url) = line
            .strip_prefix("server listening on ")
            .or_else(|| line.strip_prefix("opencode server listening on "))
        {
            let url = super::super::transport::local_server(url.trim())
                .ok_or("OpenCode cleanup server returned an invalid address")?;
            let daemons = super::super::Observer::from_env().daemons;
            std::fs::create_dir_all(&daemons).map_err(|error| error.to_string())?;
            let receipt = tempfile::Builder::new()
                .prefix("tandem-cleanup-")
                .tempdir_in(daemons)
                .map_err(|error| error.to_string())?;
            std::fs::set_permissions(receipt.path(), std::fs::Permissions::from_mode(0o700))
                .map_err(|error| error.to_string())?;
            let port = reqwest::Url::parse(&url)
                .ok()
                .and_then(|url| url.port())
                .ok_or("Cleanup server has no port")?;
            std::fs::write(receipt.path().join("port"), port.to_string())
                .map_err(|error| error.to_string())?;
            let secret = receipt.path().join("password");
            std::fs::write(&secret, password).map_err(|error| error.to_string())?;
            std::fs::set_permissions(secret, std::fs::Permissions::from_mode(0o600))
                .map_err(|error| error.to_string())?;
            return Ok((child, url, root, receipt));
        }
    }
    Err("OpenCode cleanup server exited without a listening address".into())
}
