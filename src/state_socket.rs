use std::{
    fs,
    os::unix::fs::{DirBuilderExt, FileTypeExt, MetadataExt, PermissionsExt},
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};

use serde_json::{Value, json};
use tokio::{
    io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader},
    net::{UnixListener, UnixStream},
    sync::{Semaphore, watch},
};

use crate::{
    environments::{config::Config, state_stream::Store},
    store::state_stream,
};

const MAX_CLIENTS: usize = 16;
const WRITE_TIMEOUT: Duration = Duration::from_secs(5);

mod delivery;

pub(crate) struct Listener {
    listener: UnixListener,
    path: PathBuf,
    inode: u64,
    device: u64,
    _lease: crate::environments::gateway::Lock,
}

pub(crate) fn socket_path(config: &Config) -> Result<PathBuf, String> {
    let root = std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| config.home.join("runtime"));
    let default = dirs::data_local_dir().map(|path| path.join("tandem"));
    let directory = if config.namespace == "tandem" && default.as_ref() == Some(&config.home) {
        root.join("tandem")
    } else {
        let mut hash = 0xcbf29ce484222325u64;
        for byte in config
            .home
            .as_os_str()
            .as_encoded_bytes()
            .iter()
            .copied()
            .chain([0])
            .chain(config.namespace.bytes())
        {
            hash = (hash ^ u64::from(byte)).wrapping_mul(0x100000001b3);
        }
        root.join("tandem").join(format!("{hash:016x}"))
    };
    let path = directory.join("state.sock");
    if path.as_os_str().as_encoded_bytes().len() >= 108 {
        return Err("state socket path exceeds the Unix socket path limit; set XDG_RUNTIME_DIR to a shorter private directory".into());
    }
    Ok(path)
}

fn private_directory(path: &Path) -> Result<(), String> {
    match fs::DirBuilder::new().mode(0o700).create(path) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(error) => return Err(error.to_string()),
    }
    let metadata = fs::symlink_metadata(path).map_err(|error| error.to_string())?;
    if !metadata.is_dir()
        || metadata.uid() != unsafe { libc::geteuid() }
        || fs::canonicalize(path).map_err(|error| error.to_string())? != path
    {
        return Err(
            "state socket directory must be a real directory owned by the current user".into(),
        );
    }
    fs::set_permissions(path, fs::Permissions::from_mode(0o700)).map_err(|error| error.to_string())
}

impl Listener {
    pub(crate) fn bind(config: &Config) -> Result<Self, String> {
        let lease = crate::environments::gateway::lock(
            config,
            &format!("state-stream-{}", config.namespace),
        )?;
        let path = socket_path(config)?;
        let directory = path.parent().ok_or("state socket directory missing")?;
        let root = std::env::var_os("XDG_RUNTIME_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| config.home.join("runtime"));
        if !root.exists() {
            private_directory(&root)?;
        }
        let metadata = fs::symlink_metadata(&root).map_err(|error| error.to_string())?;
        if !metadata.is_dir()
            || metadata.uid() != unsafe { libc::geteuid() }
            || metadata.mode() & 0o077 != 0
            || fs::canonicalize(&root).map_err(|error| error.to_string())? != root
        {
            return Err(
                "state runtime directory must be private and owned by the current user".into(),
            );
        }
        private_directory(&root.join("tandem"))?;
        private_directory(directory)?;
        match fs::symlink_metadata(&path) {
            Ok(metadata)
                if metadata.file_type().is_socket()
                    && metadata.uid() == unsafe { libc::geteuid() } =>
            {
                if std::os::unix::net::UnixStream::connect(&path).is_ok() {
                    return Err("state socket already has a live listener".into());
                }
                fs::remove_file(&path).map_err(|error| error.to_string())?;
            }
            Ok(_) => {
                return Err("state socket path contains an unexpected filesystem object".into());
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.to_string()),
        }
        let listener = UnixListener::bind(&path).map_err(|error| error.to_string())?;
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600))
            .map_err(|error| error.to_string())?;
        let metadata = fs::symlink_metadata(&path).map_err(|error| error.to_string())?;
        Ok(Self {
            listener,
            path,
            inode: metadata.ino(),
            device: metadata.dev(),
            _lease: lease,
        })
    }
}

impl Drop for Listener {
    fn drop(&mut self) {
        if fs::symlink_metadata(&self.path).is_ok_and(|metadata| {
            metadata.ino() == self.inode
                && metadata.dev() == self.device
                && metadata.file_type().is_socket()
        }) {
            let _ = fs::remove_file(&self.path);
        }
    }
}

#[cfg(test)]
fn sequence(cursor: &str) -> i64 {
    cursor
        .rsplit_once(':')
        .and_then(|(_, value)| value.parse().ok())
        .unwrap_or(-1)
}

fn encode(value: &Value) -> Result<Arc<[u8]>, String> {
    let mut bytes = serde_json::to_vec(value).map_err(|error| error.to_string())?;
    if bytes.len() > state_stream::MAX_FRAME_BYTES {
        return Err("state frame exceeds 16 MiB".into());
    }
    bytes.push(b'\n');
    Ok(Arc::from(bytes))
}

pub(crate) async fn serve(store: Arc<Store>, socket: Listener) -> Result<(), String> {
    let semaphore = Arc::new(Semaphore::new(MAX_CLIENTS));
    let adapter = store.clone();
    let mut cursor = tokio::task::spawn_blocking(move || adapter.head())
        .await
        .map_err(|error| error.to_string())??;
    let (updates, _) = watch::channel(cursor.clone());
    let mut ticker = tokio::time::interval(Duration::from_millis(500));
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut tasks = tokio::task::JoinSet::new();
    let mut terminate = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        .map_err(|error| error.to_string())?;
    loop {
        tokio::select! {
            accepted = socket.listener.accept() => {
                let (stream, _) = accepted.map_err(|error| error.to_string())?;
                if stream.peer_cred().map_err(|error| error.to_string())?.uid() != unsafe { libc::geteuid() } { continue; }
                let Ok(permit) = semaphore.clone().try_acquire_owned() else { continue; };
                let store = store.clone();
                let updates = updates.subscribe();
                tasks.spawn(async move {
                    let _ = delivery::client(stream, store, updates, Arc::new(permit)).await;
                });
            }
            _ = ticker.tick() => {
                let adapter = store.clone();
                let head = tokio::task::spawn_blocking(move || adapter.head()).await.map_err(|error| error.to_string())??;
                if head != cursor {
                    cursor = head;
                    updates.send_replace(cursor.clone());
                }
            }
            _ = tasks.join_next(), if !tasks.is_empty() => {}
            _ = tokio::signal::ctrl_c() => return Ok(()),
            _ = terminate.recv() => return Ok(()),
        }
    }
}

#[cfg(test)]
#[path = "state_socket/tests/mod.rs"]
mod tests;

async fn write_frame(
    write: &mut tokio::net::unix::OwnedWriteHalf,
    frame: &[u8],
) -> Result<(), String> {
    tokio::time::timeout(WRITE_TIMEOUT, write.write_all(frame))
        .await
        .map_err(|_| "state subscriber write timed out".to_owned())?
        .map_err(|error| error.to_string())
}

pub(crate) async fn watch(config: &Config, since: Option<String>) -> Result<(), String> {
    let path = socket_path(config)?;
    let mut stream = UnixStream::connect(&path).await.map_err(|error| {
        format!(
            "cannot connect to {}: {error}; run tandem serve-state",
            path.display()
        )
    })?;
    if stream.peer_cred().map_err(|error| error.to_string())?.uid() != unsafe { libc::geteuid() } {
        return Err("state server belongs to another user".into());
    }
    stream
        .write_all(format!("{}\n", json!({"since": since})).as_bytes())
        .await
        .map_err(|error| error.to_string())?;
    let mut reader = BufReader::new(stream);
    let mut stdout = tokio::io::stdout();
    loop {
        let mut frame = Vec::new();
        let count = (&mut reader)
            .take((state_stream::MAX_FRAME_BYTES + 2) as u64)
            .read_until(b'\n', &mut frame)
            .await
            .map_err(|error| error.to_string())?;
        if count == 0 {
            return Ok(());
        }
        if frame.len() > state_stream::MAX_FRAME_BYTES + 1 || frame.last() != Some(&b'\n') {
            return Err("state server returned an invalid frame".into());
        }
        stdout
            .write_all(&frame)
            .await
            .map_err(|error| error.to_string())?;
        stdout.flush().await.map_err(|error| error.to_string())?;
    }
}
