use std::{io::Write, path::Path, process::Command, time::Duration};

use serde_json::{Value, json};

use super::{
    command::{docker, run},
    config::{Config, private_file},
};
use crate::store::environments::{Template, validate_name};

pub(crate) const NAMESPACE: &str = "io.tandem.namespace";
pub(crate) const TEMPLATE: &str = "io.tandem.template";
pub(crate) const DIRECTORY: &str = "io.tandem.template-directory";
pub(crate) const WORKSPACE: &str = "io.tandem.workspace";
pub(crate) const ROLE: &str = "io.tandem.role";
pub(crate) const URL: &str = "io.tandem.url";
pub(crate) const KIND: &str = "io.tandem.kind";

pub(crate) fn command(config: &Config, directory: &Path, compose: &Path, name: &str) -> Command {
    let mut cmd = docker();
    cmd.args([
        "compose",
        "--ansi",
        "never",
        "--progress",
        "plain",
        "--project-name",
        &config.project(name),
        "--project-directory",
    ])
    .arg(directory)
    .arg("--file")
    .arg(compose)
    .current_dir(directory)
    .env("TANDEM_INSTANCE", name)
    .env("TANDEM_WORKSPACE", config.workspaces.join(name))
    .env("TANDEM_ORIGIN", config.origin());
    #[cfg(unix)]
    {
        cmd.env("TANDEM_UID", unsafe { libc::getuid() }.to_string());
        cmd.env("TANDEM_GID", unsafe { libc::getgid() }.to_string());
    }
    cmd
}

pub(crate) fn render(
    config: &Config,
    template: &Template,
    name: &str,
    timeout: Duration,
) -> Result<std::path::PathBuf, String> {
    let directory = Path::new(&template.directory);
    let mut cmd = command(config, directory, Path::new(&template.compose_file), name);
    cmd.args(["config", "--format", "json"]);
    let mut model: Value =
        serde_json::from_str(&run(cmd, timeout, None)?).map_err(|error| error.to_string())?;
    decorate(config, template, name, &mut model)?;
    let rendered = directory.join(format!(".tandem-{}-{name}.compose.json", config.namespace));
    // Compose will interpolate this resolved model once more when launching it.
    let text = serde_json::to_string_pretty(&model)
        .map_err(|error| error.to_string())?
        .replace('$', "$$");
    private_file(&rendered, false)
        .and_then(|mut file| file.write_all(text.as_bytes()))
        .map_err(|error| error.to_string())?;
    Ok(rendered)
}

pub(crate) fn decorate(
    config: &Config,
    template: &Template,
    name: &str,
    model: &mut Value,
) -> Result<(), String> {
    validate_name(name)?;
    let services = model
        .get_mut("services")
        .and_then(Value::as_object_mut)
        .ok_or("Compose must declare services")?;
    if services.is_empty() {
        return Err("Compose must declare at least one service".into());
    }
    for expected in template
        .manifest
        .routes
        .keys()
        .chain(template.manifest.one_shots.iter())
    {
        if !services.contains_key(expected) {
            return Err(format!("metadata references unknown service {expected}"));
        }
    }
    for (service_name, service) in services.iter_mut() {
        validate_name(service_name)?;
        let service = service.as_object_mut().ok_or("invalid Compose service")?;
        for forbidden in ["container_name", "profiles"] {
            if service.contains_key(forbidden) {
                return Err(format!("{service_name}: {forbidden} is not supported"));
            }
        }
        if service
            .get("ports")
            .and_then(Value::as_array)
            .is_some_and(|ports| !ports.is_empty())
        {
            return Err(format!(
                "{service_name}: only the gateway may publish ports; use tandem.json routes"
            ));
        }
        if let Some(mode) = service.get("network_mode").and_then(Value::as_str)
            && !mode.starts_with("service:")
        {
            return Err(format!(
                "{service_name}: only service: network_mode is supported"
            ));
        }
        let labels = service
            .entry("labels")
            .or_insert_with(|| json!({}))
            .as_object_mut()
            .ok_or("invalid service labels")?;
        if labels
            .keys()
            .any(|key| key.starts_with("traefik.") || key.starts_with("io.tandem."))
        {
            return Err(format!(
                "{service_name}: Tandem owns traefik.* and io.tandem.* labels"
            ));
        }
        for (key, value) in [
            (NAMESPACE, config.namespace.clone()),
            (KIND, "instance".into()),
            (TEMPLATE, template.name.clone()),
            (DIRECTORY, template.directory.clone()),
            (
                WORKSPACE,
                config.workspaces.join(name).display().to_string(),
            ),
            (
                ROLE,
                if template.manifest.one_shots.contains(service_name) {
                    "oneshot"
                } else {
                    "service"
                }
                .into(),
            ),
        ] {
            labels.insert(key.into(), json!(value));
        }
        if let Some(route) = template.manifest.routes.get(service_name) {
            let prefix = format!("/{name}/{service_name}");
            // Dots delimit Traefik label fields; length tags disambiguate hyphenated names.
            let router = format!(
                "{}-i{}-{name}-s{}-{service_name}",
                config.namespace,
                name.len(),
                service_name.len()
            );
            labels.insert(URL.into(), json!(format!("{}{prefix}/", config.origin())));
            labels.insert("traefik.enable".into(), json!("true"));
            labels.insert("traefik.docker.network".into(), json!(config.network()));
            labels.insert(
                format!("traefik.http.routers.{router}.rule"),
                json!(format!("PathPrefix(`{prefix}/`)")),
            );
            labels.insert(
                format!("traefik.http.routers.{router}.entrypoints"),
                json!("web"),
            );
            labels.insert(
                format!("traefik.http.routers.{router}.service"),
                json!(router),
            );
            labels.insert(
                format!("traefik.http.services.{router}.loadbalancer.server.port"),
                json!(route.port.to_string()),
            );
            if route.strip_prefix {
                labels.insert(
                    format!("traefik.http.middlewares.{router}.stripprefix.prefixes"),
                    json!(prefix),
                );
                labels.insert(
                    format!("traefik.http.routers.{router}.middlewares"),
                    json!(router),
                );
            }
            if service.contains_key("network_mode") {
                return Err(format!(
                    "{service_name}: route the host service of a shared network namespace"
                ));
            }
            let networks = service
                .entry("networks")
                .or_insert_with(|| json!({"default": {}}))
                .as_object_mut()
                .ok_or("invalid service networks")?;
            if networks.contains_key("tandem-ingress") {
                return Err("tandem-ingress is reserved".into());
            }
            networks.insert("tandem-ingress".into(), json!({}));
        }
    }
    let networks = model
        .as_object_mut()
        .ok_or("invalid Compose model")?
        .entry("networks")
        .or_insert_with(|| json!({}))
        .as_object_mut()
        .ok_or("invalid networks")?;
    if networks.contains_key("tandem-ingress") {
        return Err("tandem-ingress is reserved".into());
    }
    if !template.manifest.routes.is_empty() {
        networks.insert(
            "tandem-ingress".into(),
            json!({"name": config.network(), "external": true}),
        );
    }
    Ok(())
}
