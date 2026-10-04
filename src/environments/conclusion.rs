use std::{
    fs::File,
    process::{Child, Command, Stdio},
};

use super::{config::Config, gateway, startup};

pub(super) fn lease(id: i64) -> String {
    format!("conclusion-{id}")
}

pub(crate) fn reserve(config: &Config, id: i64) -> Result<gateway::Lock, String> {
    gateway::lock(config, &lease(id))
}

pub(crate) fn launch(
    config: &Config,
    id: i64,
    name: &str,
    instance_lock: gateway::Lock,
    rule_lease: File,
    completion: gateway::Lock,
) -> Result<Child, String> {
    use std::os::unix::process::CommandExt;
    let rules = gateway::Lock::inherited(rule_lease);
    #[cfg(target_os = "linux")]
    let mut command = Command::new("/proc/self/exe");
    #[cfg(not(target_os = "linux"))]
    let mut command = Command::new(std::env::current_exe().map_err(|error| error.to_string())?);
    let descriptors = [
        instance_lock.descriptor(),
        rules.descriptor(),
        completion.descriptor(),
    ];
    #[cfg(not(test))]
    command.args([
        "conclusion-worker",
        &id.to_string(),
        name,
        &descriptors[0].to_string(),
        &descriptors[1].to_string(),
        &descriptors[2].to_string(),
    ]);
    #[cfg(test)]
    command
        .args([
            "--exact",
            "service::instance_mcp::tests::worker_entry",
            "--ignored",
        ])
        .env("TANDEM_TEST_CONCLUSION_ID", id.to_string())
        .env("TANDEM_TEST_CONCLUSION_NAME", name)
        .env(
            "TANDEM_TEST_CONCLUSION_FDS",
            format!("{},{},{}", descriptors[0], descriptors[1], descriptors[2]),
        );
    command
        .env("TANDEM_HOME", &config.home)
        .env("TANDEM_NAMESPACE", &config.namespace)
        .env("TANDEM_GATEWAY_PORT", config.port.to_string())
        .env("TANDEM_INSTRUCTIONS_FILE", &config.instructions)
        .env("TANDEM_PROCESS_MODE", "conclusion-worker")
        .current_dir(&config.home)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    // The worker must survive client closure; external commands must not inherit its locks.
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
    let child = command
        .spawn()
        .map_err(|error| format!("cannot launch conclusion worker: {error}"))?;
    instance_lock.handoff();
    rules.handoff();
    completion.handoff();
    Ok(child)
}

pub(crate) struct Claimed {
    pub instance_lock: gateway::Lock,
    pub _rules: gateway::Lock,
    pub _completion: gateway::Lock,
}

pub(crate) fn claim(
    config: &Config,
    id: i64,
    name: &str,
    descriptors: [i32; 3],
) -> Result<Claimed, String> {
    if id <= 0
        || descriptors[0] == descriptors[1]
        || descriptors[0] == descriptors[2]
        || descriptors[1] == descriptors[2]
    {
        return Err("conclusion identity and lock descriptors must be distinct and valid".into());
    }
    crate::store::environments::validate_instance_name(name)?;
    Ok(Claimed {
        instance_lock: startup::inherit(config, &format!("instance-{name}"), descriptors[0])?,
        _rules: startup::inherit(
            config,
            &format!("rules-{}", config.namespace),
            descriptors[1],
        )?,
        _completion: startup::inherit(config, &lease(id), descriptors[2])?,
    })
}
