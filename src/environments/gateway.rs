use std::{
    fs::{File, OpenOptions, TryLockError},
    io::Write,
    path::Path,
    time::{Duration, Instant},
};

use serde_json::{Value, json};

use super::{
    command::{Progress, docker, remaining, run},
    compose,
    config::{Config, private_file},
};

#[derive(Debug)]
pub(crate) struct Lock(File);

impl Drop for Lock {
    fn drop(&mut self) {
        // A concurrently forked child can briefly retain the open file description.
        if let Err(error) = self.0.unlock() {
            crate::diagnostics::record_error("cannot release operation lock", &error);
        }
    }
}

pub(crate) fn lock(config: &Config, resource: &str) -> Result<Lock, String> {
    let file = open_lock(config, resource)?;
    file.try_lock()
        .map_err(|error| lock_error(resource, error))?;
    Ok(Lock(file))
}

pub(super) fn is_locked(config: &Config, resource: &str) -> Result<bool, String> {
    let file = open_lock(config, resource)?;
    match file.try_lock_shared() {
        Ok(()) => {
            drop(Lock(file));
            Ok(false)
        }
        Err(TryLockError::WouldBlock) => Ok(true),
        Err(error) => Err(lock_error(resource, error)),
    }
}

pub(super) fn shared_lock(config: &Config, resource: &str) -> Result<Lock, String> {
    let file = open_lock(config, resource)?;
    file.try_lock_shared()
        .map_err(|error| lock_error(resource, error))?;
    Ok(Lock(file))
}

pub(super) fn lock_until(
    config: &Config,
    resource: &str,
    deadline: Instant,
    progress: &Progress,
) -> Result<Lock, String> {
    let file = open_lock(config, resource)?;
    let mut waiting = false;
    loop {
        let budget =
            remaining(deadline).map_err(|_| format!("timed out waiting for {resource} lock"))?;
        match file.try_lock() {
            Ok(()) => return Ok(Lock(file)),
            Err(TryLockError::WouldBlock) => {
                if !waiting {
                    progress(format!("Waiting for {resource} lock"));
                    waiting = true;
                }
                std::thread::sleep(budget.min(Duration::from_millis(50)));
            }
            Err(error) => return Err(lock_error(resource, error)),
        }
    }
}

fn lock_error(resource: &str, error: TryLockError) -> String {
    match error {
        TryLockError::WouldBlock => {
            format!("{resource} is busy in another operation; retry after it finishes")
        }
        TryLockError::Error(error) => format!("cannot lock {resource}: {error}"),
    }
}

fn open_lock(config: &Config, resource: &str) -> Result<File, String> {
    let mut options = OpenOptions::new();
    options.create(true).truncate(false).read(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600).custom_flags(libc::O_NOFOLLOW);
    }
    options
        .open(config.home.join("locks").join(resource))
        .map_err(|error| error.to_string())
}

pub(crate) fn ensure(config: &Config, progress: Progress, timeout: Duration) -> Result<(), String> {
    let deadline = Instant::now() + timeout;
    let _lock = lock_until(config, "gateway", deadline, &progress)?;
    let network = config.network();
    let mut inspect = docker();
    inspect.args([
        "network",
        "ls",
        "--filter",
        &format!("name=^{network}$"),
        "--format",
        "{{.Name}}",
    ]);
    if run(
        inspect,
        remaining(deadline)?.min(Duration::from_secs(15)),
        None,
    )?
    .lines()
    .any(|name| name == network)
    {
        let mut inspect = docker();
        inspect.args([
            "network",
            "inspect",
            &network,
            "--format",
            "{{json .Labels}}",
        ]);
        let labels: Value = serde_json::from_str(&run(
            inspect,
            remaining(deadline)?.min(Duration::from_secs(15)),
            None,
        )?)
        .map_err(|error| error.to_string())?;
        if labels[compose::NAMESPACE].as_str() != Some(&config.namespace) {
            return Err(format!(
                "network {network} exists but is not owned by this Tandem namespace"
            ));
        }
    } else {
        let mut create = docker();
        create.args([
            "network",
            "create",
            "--label",
            &format!("{}={}", compose::NAMESPACE, config.namespace),
            &network,
        ]);
        run(
            create,
            remaining(deadline)?.min(Duration::from_secs(15)),
            None,
        )?;
    }
    let ids = super::docker::project_ids(config, "gateway", deadline)?;
    if !ids.is_empty() {
        let mut inspect = docker();
        inspect.arg("inspect").args(&ids);
        let containers: Vec<Value> = serde_json::from_str(&run(
            inspect,
            remaining(deadline)?.min(Duration::from_secs(15)),
            None,
        )?)
        .map_err(|error| error.to_string())?;
        if containers.iter().any(|container| {
            container["Config"]["Labels"][compose::NAMESPACE] != config.namespace
                || container["Config"]["Labels"][compose::KIND] != "gateway"
                || container["Config"]["Labels"]["io.tandem.gateway-port"]
                    .as_str()
                    .and_then(|port| port.parse::<u16>().ok())
                    != Some(config.port)
        }) {
            return Err("gateway project exists with incompatible ownership or port; inspect it before changing configuration".into());
        }
        if containers
            .iter()
            .all(|container| container["State"]["Running"] == true)
        {
            return Ok(());
        }
    }
    progress("Starting shared loopback gateway".into());
    let directory = config.home.join("gateway");
    let path = directory.join("compose.json");
    let model = json!({
        "services": { "gateway": {
            "image": "traefik:v3.6", "restart": "unless-stopped",
            "command": ["--providers.docker=true", "--providers.docker.exposedbydefault=false",
                format!("--providers.docker.network={network}"),
                format!("--providers.docker.constraints=Label(`{}`,`{}`)", compose::NAMESPACE, config.namespace),
                "--entrypoints.web.address=:80", "--accesslog=true"],
            "ports": [{"target":80,"published":config.port.to_string(),"host_ip":"127.0.0.1","protocol":"tcp"}],
            "volumes": [{"type":"bind","source":"/var/run/docker.sock","target":"/var/run/docker.sock","read_only":true}],
            "networks": ["ingress"],
            "labels": { (compose::NAMESPACE):config.namespace, (compose::KIND):"gateway", "io.tandem.gateway-port":config.port.to_string() }
        }},
        "networks": {"ingress":{"name":network,"external":true}}
    });
    private_file(&path, false)
        .and_then(|mut file| file.write_all(model.to_string().as_bytes()))
        .map_err(|error| error.to_string())?;
    let mut command = compose::command(config, Path::new(&directory), &path, "gateway", None);
    command.args(["up", "--detach"]);
    run(command, remaining(deadline)?, Some(progress))?;
    Ok(())
}
