use std::{
    collections::BTreeMap,
    fs,
    io::Write,
    path::Path,
    time::{Duration, Instant},
};

use super::{
    Environments,
    config::{Config, read_text},
    lifecycle, templates,
};
use crate::store::environments::{Instance, Repository};

mod repositories;

impl Environments {
    pub fn prepare_workspace_open(&self, workspace: &str, name: &str) -> Result<(), String> {
        if existing(Path::new(workspace))? {
            return Ok(());
        }
        let instance = match self
            .snapshot()
            .instances
            .into_iter()
            .find(|item| item.name == name)
        {
            Some(instance) => instance,
            None => {
                lifecycle::managed_instance(
                    &self.config,
                    name,
                    Instant::now() + Duration::from_secs(30),
                )?
                .0
            }
        };
        if instance.workspace != workspace {
            return Err("instance workspace does not match the requested path".into());
        }
        // Opening an existing instance must also work after its template is removed.
        let repositories = templates::get(&self.config, &instance.template)
            .map(|template| template.manifest.repositories)
            .unwrap_or_default();
        generate(&self.config, &instance, &repositories)
    }
}

fn existing(workspace: &Path) -> Result<bool, String> {
    if !fs::symlink_metadata(workspace)
        .map_err(|error| format!("workspace {}: {error}", workspace.display()))?
        .file_type()
        .is_dir()
    {
        return Err("workspace must be a real directory".into());
    }
    let path = workspace.join("AGENTS.md");
    match fs::symlink_metadata(&path) {
        Ok(metadata) if metadata.file_type().is_file() => Ok(true),
        Ok(_) => Err(format!("{} must be a regular file", path.display())),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(format!("{}: {error}", path.display())),
    }
}

pub(super) fn generate(
    config: &Config,
    instance: &Instance,
    repositories: &[Repository],
) -> Result<(), String> {
    let workspace = Path::new(&instance.workspace);
    if existing(workspace)? {
        return Ok(());
    }
    let compose_file = Path::new(&instance.template_directory).join(format!(
        ".tandem-{}-{}.compose.json",
        config.namespace, instance.name
    ));
    let command = format!(
        "docker compose --project-name {} --project-directory {} --file {}",
        shell_quote(&instance.project),
        shell_quote(&instance.template_directory),
        shell_quote(&compose_file.display().to_string()),
    );
    let (repository_lines, has_repository_guidance) =
        repositories::table(config, instance, repositories, &compose_file)?;
    let http_urls = http_url_lines(instance);
    let (http_guidance, http_section) = if instance
        .services
        .iter()
        .any(|service| service.url.is_some())
    {
        (
            "Use the exposed URLs for API and browser testing against the running application.\n"
                .into(),
            format!(
                "\n\n## HTTP URLs\n\n{http_urls}\n\nThese URLs use a shared gateway; container ports are internal."
            ),
        )
    } else {
        (String::new(), String::new())
    };
    let values = BTreeMap::from([
        ("instance", instance.name.clone()),
        ("template", code(&instance.template)),
        ("project", code(&instance.project)),
        ("repositories", repository_lines),
        (
            "repository_guidance",
            if has_repository_guidance {
                "Read the agents.md files listed above before starting any work.\n".into()
            } else {
                String::new()
            },
        ),
        ("services", service_lines(instance)),
        ("compose_command", command),
        (
            "compose_project_command",
            format!("docker compose -p {}", shell_quote(&instance.project)),
        ),
        (
            "docker_discovery_command",
            format!(
                "docker ps -a --filter {}",
                shell_quote(&format!(
                    "label=com.docker.compose.project={}",
                    instance.project
                ))
            ),
        ),
        ("http_urls", http_urls),
        ("http_guidance", http_guidance),
        ("http_section", http_section),
    ]);
    let source = read_text(&config.workspace_agents_template())?;
    let mut markdown = render(&source, &values)?;
    if let Some(guidance) = templates::read_guidance(Path::new(&instance.template_directory))?
        && !guidance.is_empty()
    {
        if !markdown.ends_with('\n') {
            markdown.push('\n');
        }
        markdown.push('\n');
        markdown.push_str(&guidance);
    }
    let mut file = tempfile::NamedTempFile::new_in(workspace).map_err(|error| error.to_string())?;
    file.write_all(markdown.as_bytes())
        .map_err(|error| error.to_string())?;
    file.as_file()
        .sync_all()
        .map_err(|error| error.to_string())?;
    match file.persist_noclobber(workspace.join("AGENTS.md")) {
        Ok(_) => Ok(()),
        Err(error) if error.error.kind() == std::io::ErrorKind::AlreadyExists => {
            if existing(workspace)? {
                Ok(())
            } else {
                Err("workspace AGENTS.md disappeared during generation".into())
            }
        }
        Err(error) => Err(format!("cannot write workspace AGENTS.md: {error}")),
    }
}

fn http_url_lines(instance: &Instance) -> String {
    let urls: BTreeMap<_, _> = instance
        .services
        .iter()
        .filter_map(|service| service.url.as_ref().map(|url| (&service.name, url)))
        .collect();
    if urls.is_empty() {
        return "No HTTP routes configured.".into();
    }
    urls.iter()
        .map(|(name, url)| format!("- {}: {}", code(name), code(url)))
        .collect::<Vec<_>>()
        .join("\n")
}

fn service_lines(instance: &Instance) -> String {
    let services: BTreeMap<_, _> = instance
        .services
        .iter()
        .map(|service| (&service.name, service))
        .collect();
    services
        .values()
        .map(|service| {
            let mut properties = vec![if service.one_shot {
                "one-shot setup".into()
            } else {
                "service".into()
            }];
            if let Some(image) = &service.image {
                properties.push(format!("image {}", code(image)));
            }
            if let Some(url) = &service.url {
                properties.push(code(url));
            }
            if let Some(port) = service.port {
                properties.push(format!("container port {port}"));
            }
            if service.url.is_none() {
                properties.push("no host HTTP route".into());
            }
            format!("- {} — {}", code(&service.name), properties.join("; "))
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn render(source: &str, values: &BTreeMap<&str, String>) -> Result<String, String> {
    let mut result = String::new();
    let mut remaining = source;
    while let Some((before, after)) = remaining.split_once("{{") {
        result.push_str(before);
        let (key, rest) = after
            .split_once("}}")
            .ok_or("unclosed workspace AGENTS.md template placeholder")?;
        result.push_str(
            values.get(key.trim()).ok_or_else(|| {
                format!("unknown workspace AGENTS.md template placeholder: {key}")
            })?,
        );
        remaining = rest;
    }
    result.push_str(remaining);
    if result.trim().is_empty() {
        return Err("workspace AGENTS.md template must not be empty".into());
    }
    Ok(result)
}

fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

fn code(value: &str) -> String {
    let value = value.replace(['\n', '\r'], " ");
    if !value.contains('`') && !value.starts_with(' ') && !value.ends_with(' ') {
        return format!("`{value}`");
    }
    let fence = "`".repeat(
        value
            .split(|character| character != '`')
            .map(str::len)
            .max()
            .unwrap_or(0)
            + 1,
    );
    format!("{fence} {value} {fence}")
}

#[cfg(test)]
#[path = "tests/workspace_agents.rs"]
mod tests;
