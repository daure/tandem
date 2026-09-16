use std::{
    fs::{File, OpenOptions},
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

pub(crate) fn lock(config: &Config, resource: &str) -> Result<File, String> {
    let mut options = OpenOptions::new();
    options.create(true).truncate(false).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600).custom_flags(libc::O_NOFOLLOW);
    }
    let file = options
        .open(config.home.join("locks").join(resource))
        .map_err(|error| error.to_string())?;
    file.try_lock()
        .map_err(|_| format!("{resource} is busy in another operation; retry after it finishes"))?;
    Ok(file)
}

pub(crate) fn ensure(config: &Config, progress: Progress, timeout: Duration) -> Result<(), String> {
    let deadline = Instant::now() + timeout;
    let _lock = lock(config, "gateway")?;
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
    let mut command = compose::command(config, Path::new(&directory), &path, "gateway");
    command.args(["up", "--detach"]);
    run(command, remaining(deadline)?, Some(progress))?;
    Ok(())
}
