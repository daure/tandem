use std::{
    io::ErrorKind,
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant},
};

const SOUND_FILE: &str = "/usr/share/sounds/freedesktop/stereo/complete.oga";
const TIMEOUT: Duration = Duration::from_secs(3);

pub(crate) fn play_completion() -> Result<(), String> {
    let mut failures = Vec::new();
    for (program, arguments) in [
        ("canberra-gtk-play", vec!["-i", "complete"]),
        ("pw-play", vec![SOUND_FILE]),
        ("paplay", vec![SOUND_FILE]),
    ] {
        match play(program, &arguments) {
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

fn play(program: &str, arguments: &[&str]) -> std::io::Result<()> {
    let mut child = Command::new(program)
        .args(arguments)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?;
    let started = Instant::now();
    loop {
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
