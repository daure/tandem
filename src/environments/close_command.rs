use std::{
    path::Path,
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant},
};

use super::command::Progress;

pub(super) struct CloseCommand {
    command: Result<String, String>,
    warning: Progress,
}

impl CloseCommand {
    pub(super) fn new(command: Result<String, String>, warning: Progress) -> Self {
        Self { command, warning }
    }

    pub(super) fn run(&self, name: &str, workspace: &Path, progress: &Progress) {
        let result = match &self.command {
            Ok(command) if command.trim().is_empty() => return,
            Ok(command) => {
                progress(format!("Running close command for {name}"));
                execute(command, name, workspace, Duration::from_secs(10))
            }
            Err(error) => Err(format!("cannot read saved command: {error}")),
        };
        if let Err(error) = result {
            let warning = format!("Close command for {name} failed: {error}; continuing deletion");
            progress(warning.clone());
            (self.warning)(warning.clone());
            crate::diagnostics::record_error(
                "workspace close command",
                &std::io::Error::other(warning),
            );
        }
    }
}

#[cfg(test)]
impl Default for CloseCommand {
    fn default() -> Self {
        Self::new(Ok(String::new()), std::sync::Arc::new(|_| {}))
    }
}

fn execute(command: &str, name: &str, workspace: &Path, timeout: Duration) -> Result<(), String> {
    let mut process = Command::new("sh");
    process
        .args(["-c", command])
        .env("TANDEM_INSTANCE", name)
        .env("TANDEM_WORKSPACE", workspace)
        .current_dir(workspace)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        process.process_group(0);
    }
    let mut child = process
        .spawn()
        .map_err(|error| format!("cannot launch: {error}"))?;
    let started = Instant::now();
    loop {
        let error = match child.try_wait() {
            Ok(Some(status)) => {
                return if status.success() {
                    Ok(())
                } else {
                    Err(format!("exited with {status}"))
                };
            }
            Ok(None) if started.elapsed() < timeout => {
                thread::sleep(Duration::from_millis(20));
                continue;
            }
            Ok(None) => "timed out".to_owned(),
            Err(error) => format!("cannot wait: {error}"),
        };
        #[cfg(unix)]
        // Stop the shell's children too, so a timed-out cleanup cannot keep using the workspace.
        unsafe {
            libc::kill(-(child.id() as i32), libc::SIGKILL);
        }
        let _ = child.kill();
        let _ = child.wait();
        return Err(error);
    }
}

#[cfg(test)]
#[path = "tests/close_command.rs"]
mod tests;
