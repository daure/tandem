use std::{
    fs::{File, OpenOptions, TryLockError},
    io::{Read, Seek, SeekFrom, Write},
    path::Path,
    thread,
    time::{Duration, Instant},
};

const STOP_TIMEOUT: Duration = Duration::from_secs(5);

pub(crate) struct DevServerLease {
    file: File,
}

impl DevServerLease {
    pub(crate) fn replace(home: &Path, port: u16) -> Result<Self, String> {
        let mut file = open_lease(home, port)?;
        match file.try_lock() {
            Ok(()) => {}
            Err(TryLockError::WouldBlock) => {
                let pid = read_pid(&mut file)
                    .ok_or("a development server holds the lease without a valid process ID")?;
                terminate(pid)?;
                wait_for_lease(&file)?;
            }
            Err(error) => return Err(format!("cannot lock development server lease: {error}")),
        }
        write_pid(&mut file)?;
        Ok(Self { file })
    }
}

impl Drop for DevServerLease {
    fn drop(&mut self) {
        let _ = self.file.set_len(0);
        if let Err(error) = self.file.unlock() {
            crate::diagnostics::record_error("cannot release development server lease", &error);
        }
    }
}

fn open_lease(home: &Path, port: u16) -> Result<File, String> {
    let mut options = OpenOptions::new();
    options.create(true).truncate(false).read(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600).custom_flags(libc::O_NOFOLLOW);
    }
    options
        .open(home.join("locks").join(format!("dev-server-{port}")))
        .map_err(|error| format!("cannot open development server lease: {error}"))
}

fn read_pid(file: &mut File) -> Option<u32> {
    file.seek(SeekFrom::Start(0)).ok()?;
    let mut value = String::new();
    file.read_to_string(&mut value).ok()?;
    value.trim().parse().ok().filter(|pid| *pid != 0)
}

fn write_pid(file: &mut File) -> Result<(), String> {
    file.set_len(0).map_err(|error| error.to_string())?;
    file.seek(SeekFrom::Start(0))
        .map_err(|error| error.to_string())?;
    writeln!(file, "{}", std::process::id()).map_err(|error| error.to_string())?;
    file.sync_data().map_err(|error| error.to_string())
}

fn terminate(pid: u32) -> Result<(), String> {
    #[cfg(unix)]
    {
        let result = unsafe { libc::kill(pid as libc::pid_t, libc::SIGTERM) };
        if result == 0 {
            Ok(())
        } else {
            Err(format!(
                "cannot stop development server process {pid}: {}",
                std::io::Error::last_os_error()
            ))
        }
    }
    #[cfg(not(unix))]
    {
        let _ = pid;
        Err("a development server is already running; stop it before starting another".into())
    }
}

fn wait_for_lease(file: &File) -> Result<(), String> {
    let deadline = Instant::now() + STOP_TIMEOUT;
    loop {
        match file.try_lock() {
            Ok(()) => return Ok(()),
            Err(TryLockError::WouldBlock) if Instant::now() < deadline => {
                thread::sleep(Duration::from_millis(50));
            }
            Err(TryLockError::WouldBlock) => {
                return Err("timed out waiting for the previous development server to stop".into());
            }
            Err(error) => return Err(format!("cannot lock development server lease: {error}")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::read_pid;

    #[test]
    fn development_server_lease_requires_a_nonzero_process_id() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("lease");
        for (value, expected) in [("42\n", Some(42)), ("0", None), ("pid", None)] {
            std::fs::write(&path, value).unwrap();
            assert_eq!(read_pid(&mut std::fs::File::open(&path).unwrap()), expected);
        }
    }
}
