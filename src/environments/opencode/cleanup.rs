use std::{
    fs::File,
    io::{Read, Seek, SeekFrom},
    os::{
        fd::{AsRawFd, FromRawFd},
        unix::process::CommandExt,
    },
    path::PathBuf,
    process::{Child, Command, Stdio},
};

use serde::{Deserialize, Serialize};

use super::Observer;
use crate::environments::config::Config;

#[derive(Deserialize, Serialize)]
pub(crate) struct Request {
    pub directory: String,
    pub servers: Vec<String>,
    presence: PathBuf,
    daemons: PathBuf,
}

impl Request {
    pub(crate) fn observer(&self) -> Observer {
        Observer {
            presence: self.presence.clone(),
            daemons: self.daemons.clone(),
            ..Observer::from_env()
        }
    }
}

#[derive(Deserialize, Serialize)]
pub(crate) struct Outcome {
    pub cleared: bool,
    pub removed: Vec<(String, String)>,
    pub error: Option<String>,
}

impl Outcome {
    pub(crate) fn failed(error: String) -> Self {
        Self {
            cleared: false,
            removed: Vec::new(),
            error: Some(error),
        }
    }
}

pub(crate) struct Worker {
    child: Child,
    output: File,
}

impl Worker {
    pub(crate) fn wait(mut self) -> Outcome {
        let result = (|| {
            let status = self.child.wait().map_err(|error| error.to_string())?;
            if !status.success() {
                return Err(format!("OpenCode cleanup worker stopped: {status}"));
            }
            self.output.rewind().map_err(|error| error.to_string())?;
            serde_json::from_reader((&self.output).take(4_194_304))
                .map_err(|error| format!("OpenCode cleanup result unavailable: {error}"))
        })();
        result.unwrap_or_else(Outcome::failed)
    }
}

pub(crate) fn launch(
    config: &Config,
    observer: &Observer,
    directory: &str,
    servers: Vec<String>,
) -> Result<Worker, String> {
    let output = tempfile::tempfile_in(&config.home).map_err(|error| error.to_string())?;
    let descriptor = output.as_raw_fd();
    #[cfg(target_os = "linux")]
    let mut command = Command::new("/proc/self/exe");
    #[cfg(not(target_os = "linux"))]
    let mut command = Command::new(std::env::current_exe().map_err(|error| error.to_string())?);
    #[cfg(not(test))]
    command.args(["opencode-cleanup-worker", &descriptor.to_string()]);
    #[cfg(test)]
    command
        .args([
            "--exact",
            "service::opencode::sessions::tests::worker_entry",
            "--ignored",
        ])
        .env("TANDEM_TEST_CLEANUP_FD", descriptor.to_string());
    command
        .env("TANDEM_HOME", &config.home)
        .env("TANDEM_NAMESPACE", &config.namespace)
        .env("TANDEM_GATEWAY_PORT", config.port.to_string())
        .env("TANDEM_INSTRUCTIONS_FILE", &config.instructions)
        .env("TANDEM_PROCESS_MODE", "opencode-cleanup-worker")
        .env(
            "TANDEM_OPENCODE_CLEANUP",
            serde_json::to_string(&Request {
                directory: directory.to_owned(),
                servers,
                presence: observer.presence.clone(),
                daemons: observer.daemons.clone(),
            })
            .map_err(|error| error.to_string())?,
        )
        .current_dir(&config.home)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    // An inherited regular file keeps results writable after the initiating TUI exits.
    unsafe {
        command.pre_exec(move || {
            if libc::setsid() == -1 || libc::fcntl(descriptor, libc::F_SETFD, 0) == -1 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
    let child = command
        .spawn()
        .map_err(|error| format!("Cannot launch OpenCode cleanup: {error}"))?;
    Ok(Worker { child, output })
}

pub(crate) fn request() -> Result<Request, String> {
    serde_json::from_str(
        &std::env::var("TANDEM_OPENCODE_CLEANUP").map_err(|error| error.to_string())?,
    )
    .map_err(|error| error.to_string())
}

pub(crate) fn claim_output(descriptor: i32) -> Result<File, String> {
    if descriptor < 3 || unsafe { libc::fcntl(descriptor, libc::F_GETFD) } == -1 {
        return Err("OpenCode cleanup result descriptor unavailable".into());
    }
    let output = unsafe { File::from_raw_fd(descriptor) };
    if !output
        .metadata()
        .map_err(|error| error.to_string())?
        .is_file()
    {
        return Err("OpenCode cleanup result must be a regular file".into());
    }
    if unsafe { libc::fcntl(descriptor, libc::F_SETFD, libc::FD_CLOEXEC) } == -1 {
        return Err(std::io::Error::last_os_error().to_string());
    }
    Ok(output)
}

pub(crate) fn write_outcome(output: &mut File, outcome: &Outcome) -> Result<(), String> {
    output
        .seek(SeekFrom::Start(0))
        .map_err(|error| error.to_string())?;
    serde_json::to_writer(output, outcome).map_err(|error| error.to_string())
}
