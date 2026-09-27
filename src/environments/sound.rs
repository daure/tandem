use std::{
    fs,
    io::ErrorKind,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant},
};

use crate::store::completion::SoundChoice;

const SOUND_FILE: &str = "/usr/share/sounds/freedesktop/stereo/complete.oga";
const TIMEOUT: Duration = Duration::from_secs(3);

pub(crate) fn available() -> Vec<SoundChoice> {
    let mut roots = Vec::new();
    if let Some(home) = dirs::data_local_dir() {
        roots.push(home.join("sounds"));
    }
    let data_dirs = std::env::var_os("XDG_DATA_DIRS")
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "/usr/local/share:/usr/share".into());
    roots.extend(
        std::env::split_paths(&data_dirs)
            .filter(|path| path.is_absolute())
            .map(|path| path.join("sounds")),
    );
    discover(&roots)
}

fn discover(roots: &[PathBuf]) -> Vec<SoundChoice> {
    let mut choices = vec![SoundChoice {
        id: String::new(),
        label: "System default (complete)".into(),
    }];
    for root in roots {
        collect(root, root, 0, &mut choices);
    }
    choices[1..].sort_by(|a, b| a.label.cmp(&b.label).then(a.id.cmp(&b.id)));
    let mut seen = std::collections::HashSet::new();
    choices.retain(|choice| seen.insert(choice.id.clone()));
    choices
}

fn collect(root: &Path, directory: &Path, depth: usize, choices: &mut Vec<SoundChoice>) {
    let entries = match fs::read_dir(directory) {
        Ok(entries) => entries,
        Err(error) if error.kind() == ErrorKind::NotFound => return,
        Err(error) => {
            crate::diagnostics::record_error("cannot list desktop sounds", &error);
            return;
        }
    };
    for entry in entries {
        let Ok(entry) = entry else { continue };
        let path = entry.path();
        let Ok(kind) = entry.file_type() else {
            continue;
        };
        if kind.is_dir() && depth < 6 {
            collect(root, &path, depth + 1, choices);
        } else if matches!(
            path.extension().and_then(|value| value.to_str()),
            Some("oga" | "ogg" | "wav")
        ) && path.is_file()
        {
            let Some(id) = path.to_str() else { continue };
            choices.push(SoundChoice {
                id: id.into(),
                label: path
                    .strip_prefix(root)
                    .unwrap_or(&path)
                    .with_extension("")
                    .display()
                    .to_string(),
            });
        }
    }
}

pub(crate) fn play_completion(selection: &str, cancelled: impl Fn() -> bool) -> Result<(), String> {
    let file = if selection.is_empty() {
        SOUND_FILE
    } else {
        selection
    };
    let mut failures = Vec::new();
    for (program, arguments) in [
        (
            "canberra-gtk-play",
            if selection.is_empty() {
                vec!["-i", "complete"]
            } else {
                vec!["-f", file]
            },
        ),
        ("pw-play", vec![file]),
        ("paplay", vec![file]),
    ] {
        if cancelled() {
            return Ok(());
        }
        match play(program, &arguments, &cancelled) {
            Ok(()) => return Ok(()),
            Err(error) if error.kind() == ErrorKind::NotFound => {}
            Err(error) => failures.push(format!("{program}: {error}")),
        }
    }
    if failures.is_empty() {
        Err("no supported desktop audio player was found".into())
    } else {
        Err(format!("completion sound failed: {}", failures.join("; ")))
    }
}

fn play(program: &str, arguments: &[&str], cancelled: &impl Fn() -> bool) -> std::io::Result<()> {
    let mut child = Command::new(program)
        .args(arguments)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?;
    let started = Instant::now();
    loop {
        if cancelled() {
            let _ = child.kill();
            let _ = child.wait();
            return Ok(());
        }
        if let Some(status) = child.try_wait()? {
            return if status.success() {
                Ok(())
            } else {
                Err(std::io::Error::other(format!("exited {status}")))
            };
        }
        if started.elapsed() >= TIMEOUT {
            let _ = child.kill();
            let _ = child.wait();
            return Err(std::io::Error::new(
                ErrorKind::TimedOut,
                "playback timed out",
            ));
        }
        thread::sleep(Duration::from_millis(20));
    }
}

#[cfg(test)]
#[path = "tests/sound.rs"]
mod tests;
