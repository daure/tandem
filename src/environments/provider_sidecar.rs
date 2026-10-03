use std::{
    fs::{File, OpenOptions, TryLockError},
    io::Write,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant},
};

use serde::{Deserialize, Serialize};

use super::{
    config::{Config, private_file, read_text},
    gateway,
};

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Receipt {
    pub pid: u32,
    pub origin: String,
    pub identity: String,
}

pub(crate) struct Lease {
    _file: File,
    config: Config,
    pub identity: String,
}

fn lease_path(config: &Config) -> PathBuf {
    config
        .home
        .join("locks")
        .join(format!("provider-sidecar-{}", config.namespace))
}

fn receipt_path(config: &Config) -> PathBuf {
    config
        .home
        .join("locks")
        .join(format!("provider-sidecar-{}.json", config.namespace))
}

fn lease_file(config: &Config) -> Result<File, String> {
    let mut options = OpenOptions::new();
    options.create(true).truncate(false).read(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600).custom_flags(libc::O_NOFOLLOW);
    }
    options
        .open(lease_path(config))
        .map_err(|error| error.to_string())
}

impl Lease {
    pub(crate) fn acquire(config: &Config) -> Result<Self, String> {
        let file = lease_file(config)?;
        file.try_lock()
            .map_err(|_| "provider sidecar already owns this namespace".to_string())?;
        let connection = rusqlite::Connection::open(config.home.join("settings.sqlite3"))
            .map_err(|error| error.to_string())?;
        let identity = connection
            .query_row("SELECT lower(hex(randomblob(24)))", [], |row| row.get(0))
            .map_err(|error| error.to_string())?;
        Ok(Self {
            _file: file,
            config: config.clone(),
            identity,
        })
    }

    pub(crate) fn publish(&self, origin: String) -> Result<(), String> {
        let receipt = Receipt {
            pid: std::process::id(),
            origin,
            identity: self.identity.clone(),
        };
        let path = receipt_path(&self.config);
        let mut file = private_file(&path, false).map_err(|error| error.to_string())?;
        file.write_all(
            serde_json::to_string(&receipt)
                .map_err(|error| error.to_string())?
                .as_bytes(),
        )
        .and_then(|()| file.sync_all())
        .map_err(|error| error.to_string())
    }
}

pub(crate) fn running(config: &Config) -> Option<Receipt> {
    let file = lease_file(config).ok()?;
    if !matches!(file.try_lock(), Err(TryLockError::WouldBlock)) {
        return None;
    }
    let receipt: Receipt = serde_json::from_str(&read_text(&receipt_path(config)).ok()?).ok()?;
    let url = reqwest::Url::parse(&receipt.origin).ok()?;
    if url.scheme() != "http"
        || url.host_str() != Some("127.0.0.1")
        || !url.username().is_empty()
        || url.password().is_some()
        || url.port().is_none()
    {
        return None;
    }
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(1))
        .no_proxy()
        .build()
        .ok()?;
    let response: Receipt = serde_json::from_str(
        &client
            .get(format!("{}/v1/identity", receipt.origin))
            .send()
            .ok()?
            .error_for_status()
            .ok()?
            .text()
            .ok()?,
    )
    .ok()?;
    (response == receipt).then_some(receipt)
}

pub(crate) fn ensure(config: &Config) -> Result<String, String> {
    let progress: super::command::Progress = std::sync::Arc::new(|_| {});
    let _gate = gateway::lock_until(
        config,
        &format!("provider-sidecar-start-{}", config.namespace),
        Instant::now() + Duration::from_secs(30),
        &progress,
    )?;
    let executable = installed_executable()?;
    if let Some(receipt) = running(config) {
        if executable_is_current(&receipt, &executable)? {
            return Ok(receipt.origin);
        }
        stop_outdated(config, &receipt)?;
    }
    let file = lease_file(config)?;
    match file.try_lock() {
        Ok(()) => {
            file.unlock().map_err(|error| error.to_string())?;
        }
        Err(TryLockError::WouldBlock) => {
            return Err("provider sidecar lease is held but readiness cannot be verified".into());
        }
        Err(error) => return Err(error.to_string()),
    }
    let log_path = config.home.join("provider-sidecar.log");
    let mut options = OpenOptions::new();
    options.create(true).append(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600).custom_flags(libc::O_NOFOLLOW);
    }
    let log = options.open(log_path).map_err(|error| error.to_string())?;
    let mut command = Command::new(executable);
    command.arg("provider-sidecar-worker");
    if let Ok(source) = read_text(&receipt_path(config))
        && let Ok(receipt) = serde_json::from_str::<Receipt>(&source)
        && let Ok(url) = reqwest::Url::parse(&receipt.origin)
        && url.host_str() == Some("127.0.0.1")
        && url.scheme() == "http"
        && let Some(port) = url.port()
    {
        command.args(["--bind", &format!("127.0.0.1:{port}")]);
    }
    command
        .env("TANDEM_HOME", &config.home)
        .env("TANDEM_NAMESPACE", &config.namespace)
        .env("TANDEM_GATEWAY_PORT", config.port.to_string())
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::from(log));
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        unsafe {
            command.pre_exec(|| {
                if libc::setsid() < 0 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }
    }
    let mut child = command.spawn().map_err(|error| error.to_string())?;
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        if let Some(receipt) = running(config) {
            thread::spawn(move || {
                let _ = child.wait();
            });
            return Ok(receipt.origin);
        }
        if let Some(status) = child.try_wait().map_err(|error| error.to_string())? {
            return Err(format!(
                "provider sidecar exited {status}; inspect its private log"
            ));
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            return Err("provider sidecar startup timed out".into());
        }
        thread::sleep(Duration::from_millis(100));
    }
}

fn installed_executable() -> Result<PathBuf, String> {
    let executable = std::env::current_exe().map_err(|error| error.to_string())?;
    #[cfg(target_os = "linux")]
    {
        use std::os::unix::ffi::{OsStrExt, OsStringExt};
        // Open clients can retain an unlinked executable after an atomic installation.
        // Resolve its installed path so those clients cannot downgrade the receiver.
        if let Some(path) = executable
            .as_os_str()
            .as_bytes()
            .strip_suffix(b" (deleted)")
        {
            return Ok(std::ffi::OsString::from_vec(path.to_vec()).into());
        }
    }
    Ok(executable)
}

#[cfg(target_os = "linux")]
fn executable_is_current(receipt: &Receipt, executable: &Path) -> Result<bool, String> {
    use std::os::unix::fs::MetadataExt;
    let running = std::fs::metadata(format!("/proc/{}/exe", receipt.pid))
        .map_err(|error| format!("cannot inspect provider sidecar executable: {error}"))?;
    let installed = std::fs::metadata(executable)
        .map_err(|error| format!("cannot inspect installed Tandem executable: {error}"))?;
    Ok(running.dev() == installed.dev() && running.ino() == installed.ino())
}

#[cfg(not(target_os = "linux"))]
fn executable_is_current(_receipt: &Receipt, _executable: &Path) -> Result<bool, String> {
    Ok(true)
}

#[cfg(target_os = "linux")]
fn stop_outdated(config: &Config, receipt: &Receipt) -> Result<(), String> {
    use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
    let descriptor = unsafe { libc::syscall(libc::SYS_pidfd_open, receipt.pid, 0) };
    if descriptor < 0 {
        return Err(format!(
            "cannot pin outdated provider sidecar process: {}",
            std::io::Error::last_os_error()
        ));
    }
    let process = unsafe { OwnedFd::from_raw_fd(descriptor as i32) };
    if running(config).as_ref() != Some(receipt) {
        return Err("provider sidecar identity changed before replacement; retry".into());
    }
    // A pidfd prevents signaling an unrelated process if the receipt's PID is reused.
    let result = unsafe {
        libc::syscall(
            libc::SYS_pidfd_send_signal,
            process.as_raw_fd(),
            libc::SIGINT,
            std::ptr::null::<libc::siginfo_t>(),
            0,
        )
    };
    if result < 0 {
        return Err(format!(
            "cannot stop outdated provider sidecar: {}",
            std::io::Error::last_os_error()
        ));
    }
    let file = lease_file(config)?;
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        match file.try_lock() {
            Ok(()) => return Ok(()),
            Err(TryLockError::WouldBlock) => {}
            Err(error) => return Err(error.to_string()),
        }
        if Instant::now() >= deadline {
            return Err(
                "outdated provider sidecar did not release its lease; retry after shutdown".into(),
            );
        }
        thread::sleep(Duration::from_millis(100));
    }
}

#[cfg(not(target_os = "linux"))]
fn stop_outdated(_config: &Config, _receipt: &Receipt) -> Result<(), String> {
    Err("automatic provider sidecar replacement requires Linux".into())
}
