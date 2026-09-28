use std::{
    fs::File,
    process::{Child, Command, Stdio},
    sync::{Arc, Mutex},
};

use super::{Config, Environments, Record, Startup, Writer, gateway, journal, lease, read, write};
use crate::store::environments::Operation;

pub(crate) fn launch(
    environments: &Environments,
    operation: &Operation,
    timeout: u64,
    startup: &mut Startup,
) -> Result<Child, String> {
    let config = &environments.config;
    let lock = match startup.instance_lock.take() {
        Some(lock) => lock,
        None => gateway::lock(config, &format!("instance-{}", operation.name))?,
    };
    let lease_lock = gateway::lock(config, &lease(&operation.id))?;
    let record = Record {
        operation: operation.clone(),
        description: startup.description.clone(),
        branch_instances: startup.branch_instances,
        kind: environments.startup_kind(&operation.id).unwrap_or_default(),
        started_at: journal::now(),
        timeout,
        owner_pid: 0,
        workspace_ready: false,
        services: Vec::new(),
    };
    write(config, &record)?;
    let result = spawn(config, &record, &lock, &lease_lock);
    match result {
        Ok(child) => {
            lock.handoff();
            lease_lock.handoff();
            Ok(child)
        }
        Err(error) => {
            let mut record = record;
            record.operation.state = crate::store::environments::OperationState::Failed;
            record.operation.error = Some(error.clone());
            write(config, &record)?;
            Err(error)
        }
    }
}

#[cfg(unix)]
fn spawn(
    config: &Config,
    record: &Record,
    lock: &gateway::Lock,
    lease: &gateway::Lock,
) -> Result<Child, String> {
    use std::os::unix::process::CommandExt;
    let mut command = Command::new(std::env::current_exe().map_err(|error| error.to_string())?);
    #[cfg(not(test))]
    command.args([
        "startup-worker",
        &record.operation.name,
        &record.operation.id,
        &lock.descriptor().to_string(),
        &lease.descriptor().to_string(),
    ]);
    #[cfg(test)]
    command
        .args([
            "--exact",
            "environments::startup::tests::worker_entry",
            "--ignored",
        ])
        .env("TANDEM_TEST_WORKER_NAME", &record.operation.name)
        .env("TANDEM_TEST_WORKER_ID", &record.operation.id)
        .env("TANDEM_TEST_WORKER_LOCK", lock.descriptor().to_string())
        .env("TANDEM_TEST_WORKER_LEASE", lease.descriptor().to_string());
    command
        .env("TANDEM_HOME", &config.home)
        .env("TANDEM_NAMESPACE", &config.namespace)
        .env("TANDEM_GATEWAY_PORT", config.port.to_string())
        .env("TANDEM_INSTRUCTIONS_FILE", &config.instructions)
        .env("TANDEM_PROCESS_MODE", "startup-worker")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    let descriptors = [lock.descriptor(), lease.descriptor()];
    // Only async-signal-safe calls are permitted between fork and exec.
    unsafe {
        command.pre_exec(move || {
            if libc::setsid() == -1 {
                return Err(std::io::Error::last_os_error());
            }
            for descriptor in descriptors {
                if libc::fcntl(descriptor, libc::F_SETFD, 0) == -1 {
                    return Err(std::io::Error::last_os_error());
                }
            }
            Ok(())
        });
    }
    command
        .spawn()
        .map_err(|error| format!("cannot launch startup worker: {error}"))
}

#[cfg(not(unix))]
fn spawn(_: &Config, _: &Record, _: &gateway::Lock, _: &gateway::Lock) -> Result<Child, String> {
    Err("detached startup requires Unix".into())
}

pub(crate) struct Claimed {
    pub record: Record,
    pub writer: Arc<Writer>,
    pub instance_lock: gateway::Lock,
    pub _lease: gateway::Lock,
}

pub(crate) fn claim(
    config: &Config,
    name: &str,
    id: &str,
    instance_fd: i32,
    lease_fd: i32,
) -> Result<Claimed, String> {
    let mut record = read(config, name)?.ok_or("startup request missing")?;
    if record.operation.id != id
        || record.operation.state != crate::store::environments::OperationState::Running
        || record.owner_pid != 0
    {
        return Err("startup request is not available for this worker".into());
    }
    if instance_fd == lease_fd {
        return Err("startup lock descriptors must differ".into());
    }
    let instance_lock = inherit(
        config,
        &format!("instance-{}", record.operation.name),
        instance_fd,
    )?;
    let lease_lock = inherit(config, &lease(id), lease_fd)?;
    record.owner_pid = std::process::id();
    record.operation.progress = vec!["Startup worker accepted; preparing instance".into()];
    write(config, &record)?;
    let writer = Arc::new(Writer {
        config: config.clone(),
        record: Mutex::new(record.clone()),
        error: Mutex::new(None),
    });
    Ok(Claimed {
        record,
        writer,
        instance_lock,
        _lease: lease_lock,
    })
}

#[cfg(unix)]
fn inherit(config: &Config, resource: &str, descriptor: i32) -> Result<gateway::Lock, String> {
    use std::os::{fd::FromRawFd, unix::fs::MetadataExt};
    if descriptor < 3 || unsafe { libc::fcntl(descriptor, libc::F_GETFD) } == -1 {
        return Err("startup lock descriptor is unavailable".into());
    }
    let file = unsafe { File::from_raw_fd(descriptor) };
    let expected = std::fs::symlink_metadata(config.home.join("locks").join(resource))
        .map_err(|error| error.to_string())?;
    let actual = file.metadata().map_err(|error| error.to_string())?;
    if !expected.file_type().is_file()
        || actual.dev() != expected.dev()
        || actual.ino() != expected.ino()
        || !gateway::is_locked(config, resource)?
    {
        return Err("startup lock ownership mismatch".into());
    }
    // Compose and Git must not retain operation locks after the worker dies.
    if unsafe { libc::fcntl(descriptor, libc::F_SETFD, libc::FD_CLOEXEC) } == -1 {
        return Err(std::io::Error::last_os_error().to_string());
    }
    Ok(gateway::Lock::inherited(file))
}

#[cfg(not(unix))]
fn inherit(_: &Config, _: &str, _: i32) -> Result<gateway::Lock, String> {
    Err("detached startup requires Unix".into())
}
