use std::{
    io::{BufRead, BufReader, Read},
    process::{Command, Stdio},
    sync::Arc,
    thread,
    time::{Duration, Instant},
};

pub(crate) type Progress = Arc<dyn Fn(String) + Send + Sync>;

pub(super) fn remaining(deadline: Instant) -> Result<Duration, String> {
    deadline
        .checked_duration_since(Instant::now())
        .filter(|value| !value.is_zero())
        .ok_or_else(|| "operation deadline reached".into())
}

pub(crate) fn docker() -> Command {
    let mut command = Command::new("docker");
    command
        .env("DOCKER_CLI_HINTS", "false")
        .stdin(Stdio::null());
    command
}

pub(crate) fn run(
    mut command: Command,
    timeout: Duration,
    progress: Option<Progress>,
) -> Result<String, String> {
    if timeout.is_zero() {
        return Err("operation deadline reached".into());
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    let mut child = command
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| format!("could not launch Docker: {error}"))?;
    let stdout = child.stdout.take().ok_or("Docker stdout unavailable")?;
    let stderr = child.stderr.take().ok_or("Docker stderr unavailable")?;
    let out_progress = progress.clone();
    let stream_logs = progress.is_some();
    let out = thread::spawn(move || capture(stdout, 8 * 1024 * 1024, out_progress, stream_logs));
    let err = thread::spawn(move || capture(stderr, 64 * 1024, progress, true));
    let started = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) if out.is_finished() && err.is_finished() => break Ok(status),
            Ok(_) if started.elapsed() < timeout => thread::sleep(Duration::from_millis(40)),
            result => {
                #[cfg(unix)]
                // The Compose plugin inherits this process group and may hold our pipes open.
                unsafe {
                    libc::kill(-(child.id() as i32), libc::SIGKILL);
                }
                let _ = child.kill();
                let _ = child.wait();
                break Err(match result {
                    Err(error) => error.to_string(),
                    _ => "Docker command timed out; inspect remaining containers before retrying"
                        .into(),
                });
            }
        }
    };
    let stdout = out.join().map_err(|_| "Docker stdout reader failed");
    let stderr = err.join().map_err(|_| "Docker stderr reader failed");
    let status = status?;
    let stdout = stdout??;
    let stderr = stderr??;
    if !status.success() {
        return Err(format!("Docker exited {status}: {}", stderr.trim()));
    }
    Ok(stdout)
}

fn capture(
    reader: impl Read,
    limit: usize,
    progress: Option<Progress>,
    truncate_logs: bool,
) -> Result<String, String> {
    let mut reader = BufReader::new(reader);
    let mut stored = Vec::new();
    let mut chunk = Vec::new();
    let mut truncated = false;
    loop {
        chunk.clear();
        let count = reader
            .by_ref()
            .take(8192)
            .read_until(b'\n', &mut chunk)
            .map_err(|error| error.to_string())?;
        if count == 0 {
            break;
        }
        if let Some(progress) = &progress {
            let text: String = String::from_utf8_lossy(&chunk)
                .chars()
                .filter(|character| !character.is_control() || *character == '\t')
                .take(500)
                .collect();
            if !text.trim().is_empty() {
                progress(text);
            }
        }
        let available = limit.saturating_sub(stored.len());
        if truncate_logs {
            stored.extend_from_slice(&chunk);
            stored.drain(..stored.len().saturating_sub(limit));
        } else {
            stored.extend_from_slice(&chunk[..count.min(available)]);
        }
        truncated |= count > available;
    }
    if truncated && !truncate_logs {
        return Err(format!("Docker output exceeded {limit} bytes"));
    }
    let text = String::from_utf8_lossy(&stored);
    Ok(if truncated {
        format!("[earlier output truncated]\n{text}")
    } else {
        text.into_owned()
    })
}
