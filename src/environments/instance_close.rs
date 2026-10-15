use std::{
    os::unix::process::CommandExt,
    process::{Child, Command, Stdio},
};

use serde::{Deserialize, Serialize};

use super::{
    config::Config,
    gateway,
    opencode::{Observer, closing::Plan},
    startup,
};

#[derive(Deserialize, Serialize)]
pub(crate) struct Request {
    pub name: String,
    pub identity: String,
    pub plan: Plan,
    presence: std::path::PathBuf,
    daemons: std::path::PathBuf,
    zellij: std::path::PathBuf,
}

impl Request {
    pub(crate) fn observer(&self) -> Observer {
        Observer {
            presence: self.presence.clone(),
            daemons: self.daemons.clone(),
            zellij: self.zellij.clone(),
            ..Observer::from_env()
        }
    }
}

pub(crate) fn launch(
    config: &Config,
    observer: &Observer,
    name: &str,
    identity: String,
    plan: Plan,
    lock: gateway::Lock,
) -> Result<Child, String> {
    let request = serde_json::to_string(&Request {
        name: name.into(),
        identity,
        plan,
        presence: observer.presence.clone(),
        daemons: observer.daemons.clone(),
        zellij: observer.zellij.clone(),
    })
    .map_err(|error| error.to_string())?;
    if request.len() > 65_536 {
        return Err("too many attachments to close in one request".into());
    }
    #[cfg(target_os = "linux")]
    let mut command = Command::new("/proc/self/exe");
    #[cfg(not(target_os = "linux"))]
    let mut command = Command::new(std::env::current_exe().map_err(|error| error.to_string())?);
    let descriptor = lock.descriptor();
    #[cfg(not(test))]
    command.args(["instance-close-worker", name, &descriptor.to_string()]);
    #[cfg(test)]
    command
        .args([
            "--exact",
            "service::instance_close::tests::worker_entry",
            "--ignored",
        ])
        .env("TANDEM_TEST_CLOSE_FD", descriptor.to_string());
    command
        .env("TANDEM_HOME", &config.home)
        .env("TANDEM_NAMESPACE", &config.namespace)
        .env("TANDEM_GATEWAY_PORT", config.port.to_string())
        .env("TANDEM_INSTRUCTIONS_FILE", &config.instructions)
        .env("TANDEM_PROCESS_MODE", "instance-close-worker")
        .env("TANDEM_INSTANCE_CLOSE", request)
        .current_dir(&config.home)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
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
        .map_err(|error| format!("cannot launch instance closure: {error}"))?;
    lock.handoff();
    Ok(child)
}

pub(crate) fn claim(
    config: &Config,
    name: &str,
    descriptor: i32,
) -> Result<(Request, gateway::Lock), String> {
    crate::store::environments::validate_instance_name(name)?;
    let lock = startup::inherit(config, &format!("instance-{name}"), descriptor)?;
    let text = std::env::var("TANDEM_INSTANCE_CLOSE").map_err(|error| error.to_string())?;
    if text.len() > 65_536 {
        return Err("instance closure request exceeds limit".into());
    }
    let request: Request = serde_json::from_str(&text).map_err(|error| error.to_string())?;
    if request.name != name {
        return Err("instance closure belongs to another instance".into());
    }
    Ok((request, lock))
}
