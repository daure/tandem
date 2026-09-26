use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
};

use super::{
    config::{Config, private_file, read_text},
    gateway,
};
use crate::store::environments::{Manifest, Template, validate_name};

pub(crate) fn list(config: &Config) -> Result<Vec<Template>, String> {
    let mut templates = Vec::new();
    for entry in fs::read_dir(&config.templates).map_err(|error| error.to_string())? {
        let entry = entry.map_err(|error| error.to_string())?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if validate_name(&name).is_ok()
            && (entry.path().join("compose.yaml").is_file()
                || entry.path().join("tandem.json").is_file()
                || fs::symlink_metadata(entry.path().join("tandem-agents.md")).is_ok())
        {
            match read_template(config, &name) {
                Ok(template) => templates.push(template),
                Err(error) => templates.push(Template {
                    name,
                    directory: entry.path().display().to_string(),
                    compose_file: entry.path().join("compose.yaml").display().to_string(),
                    manifest_file: entry.path().join("tandem.json").display().to_string(),
                    guidance_file: entry.path().join("tandem-agents.md").display().to_string(),
                    compose_source: String::new(),
                    manifest_source: None,
                    guidance_source: None,
                    manifest: Manifest::default(),
                    error: Some(error),
                }),
            }
        }
    }
    templates.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(templates)
}

pub(crate) fn get(config: &Config, name: &str) -> Result<Template, String> {
    let template = read_template(config, name)?;
    if let Some(error) = &template.error {
        return Err(error.clone());
    }
    Ok(template)
}

pub(crate) fn update_manifest(
    config: &Config,
    name: &str,
    manifest: Manifest,
) -> Result<Template, String> {
    validate_name(name)?;
    validate_manifest(&manifest)?;
    let source = format!(
        "{}\n",
        serde_json::to_string_pretty(&manifest).map_err(|error| error.to_string())?
    );
    if source.len() > 262_144 {
        return Err("tandem.json exceeds 256 KiB".into());
    }
    let _lock = gateway::lock(config, &format!("template-{name}"))?;
    let directory = removal_directory(config, name)?;
    let path = directory.join("tandem.json");
    match fs::symlink_metadata(&path) {
        Ok(metadata) if !metadata.file_type().is_file() => {
            return Err("tandem.json must be a regular file, not a symlink".into());
        }
        Err(error) if error.kind() != std::io::ErrorKind::NotFound => return Err(error.to_string()),
        _ => {}
    }
    let mut template = read_template(config, name)?;
    validate_workspace_template(
        &manifest,
        !template.workspace_only(),
        true,
        template.guidance_source.is_some(),
    )?;
    let mut temporary = tempfile::Builder::new()
        .prefix(".tandem-manifest-")
        .tempfile_in(&directory)
        .map_err(|error| error.to_string())?;
    temporary
        .write_all(source.as_bytes())
        .map_err(|error| error.to_string())?;
    temporary
        .as_file()
        .sync_all()
        .map_err(|error| error.to_string())?;
    temporary
        .persist(&path)
        .map_err(|error| error.to_string())?;
    template.manifest = manifest;
    template.manifest_source = Some(source);
    template.error = None;
    Ok(template)
}

fn read_template(config: &Config, name: &str) -> Result<Template, String> {
    validate_name(name)?;
    let directory = fs::canonicalize(config.templates.join(name))
        .map_err(|error| format!("template {name}: {error}"))?;
    if !directory.starts_with(&config.templates) {
        return Err("template directory escapes templates root".into());
    }
    for file in ["compose.yaml", "tandem.json"] {
        let path = directory.join(file);
        if path.exists()
            && !fs::canonicalize(&path)
                .map_err(|error| error.to_string())?
                .starts_with(&directory)
        {
            return Err(format!("{file} escapes template directory"));
        }
    }
    let manifest_path = directory.join("tandem.json");
    let manifest_source = manifest_path
        .exists()
        .then(|| read_text(&manifest_path))
        .transpose()?;
    let manifest = manifest_source
        .as_deref()
        .map_or_else(
            || Ok(Manifest::default()),
            |source| {
                serde_json::from_str::<Manifest>(source)
                    .map_err(|error| format!("tandem.json: {error}"))
            },
        )
        .and_then(|manifest| {
            validate_manifest(&manifest)?;
            Ok(manifest)
        });
    let compose_path = directory.join("compose.yaml");
    let compose_source = compose_path
        .try_exists()
        .map_err(|error| error.to_string())?
        .then(|| read_text(&compose_path))
        .transpose()?;
    let guidance_source = read_guidance(&directory)?;
    let manifest = manifest.and_then(|manifest| {
        validate_workspace_template(
            &manifest,
            compose_source.is_some(),
            manifest_source.is_some(),
            guidance_source.is_some(),
        )?;
        Ok(manifest)
    });
    let error = manifest.as_ref().err().cloned();
    Ok(Template {
        name: name.into(),
        directory: directory.display().to_string(),
        compose_file: compose_source
            .as_ref()
            .map(|_| compose_path.display().to_string())
            .unwrap_or_default(),
        manifest_file: manifest_path.display().to_string(),
        guidance_file: directory.join("tandem-agents.md").display().to_string(),
        compose_source: compose_source.unwrap_or_default(),
        manifest_source,
        guidance_source,
        manifest: manifest.unwrap_or_default(),
        error,
    })
}

fn validate_workspace_template(
    manifest: &Manifest,
    has_compose: bool,
    has_manifest: bool,
    has_guidance: bool,
) -> Result<(), String> {
    if !has_compose {
        if !has_manifest && !has_guidance {
            return Err("templates require compose.yaml, tandem.json, or tandem-agents.md".into());
        }
        if !manifest.routes.is_empty() || !manifest.one_shots.is_empty() {
            return Err("routes and one_shots require compose.yaml".into());
        }
    }
    Ok(())
}

pub(super) fn read_guidance(directory: &Path) -> Result<Option<String>, String> {
    let path = directory.join("tandem-agents.md");
    match fs::symlink_metadata(&path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(format!("{}: {error}", path.display())),
        Ok(_) => {}
    }
    let directory = fs::canonicalize(directory).map_err(|error| error.to_string())?;
    let resolved =
        fs::canonicalize(&path).map_err(|error| format!("{}: {error}", path.display()))?;
    if !resolved.starts_with(&directory) {
        return Err("tandem-agents.md escapes template directory".into());
    }
    if !resolved.is_file() {
        return Err("tandem-agents.md must be a regular file".into());
    }
    read_text(&resolved)
        .map(Some)
        .map_err(|error| format!("{}: {error}", path.display()))
}

fn validate_manifest(manifest: &Manifest) -> Result<(), String> {
    super::repositories::validate(&manifest.repositories)?;
    if !manifest.repositories.is_empty()
        && manifest.one_shots.iter().any(|name| name == "repo-sync")
    {
        return Err(
            "declared repositories use Tandem provisioning; remove the repo-sync one-shot".into(),
        );
    }
    for (name, route) in &manifest.routes {
        validate_name(name)?;
        let path = &route.readiness_path;
        if route.port == 0
            || route.readiness_contains.trim().is_empty()
            || route.readiness_contains.len() > 4096
            || path.starts_with('/')
            || path.contains(['?', '#', '\\', '%'])
            || path.split('/').any(|part| part == "..")
            || path.chars().any(char::is_control)
        {
            return Err(format!(
                "invalid route {name}: use a nonzero port, relative readiness_path and nonempty readiness_contains"
            ));
        }
        if manifest.one_shots.contains(name) {
            return Err(format!("one-shot {name} cannot be a web route"));
        }
    }
    for name in &manifest.one_shots {
        validate_name(name)?;
    }
    Ok(())
}

pub(crate) fn create(config: &Config, name: &str) -> Result<Template, String> {
    validate_name(name)?;
    let _lock = gateway::lock(config, &format!("template-{name}"))?;
    let directory = config.templates.join(name);
    fs::create_dir(&directory).map_err(|error| format!("create template {name}: {error}"))?;
    write_new(&directory.join("tandem.json"), "{}\n")?;
    get(config, name)
}

pub(super) fn removal_directory(config: &Config, name: &str) -> Result<PathBuf, String> {
    validate_name(name)?;
    let root = fs::canonicalize(&config.templates).map_err(|error| error.to_string())?;
    if root != config.templates {
        return Err("templates root must be a real directory".into());
    }
    let directory = root.join(name);
    let metadata =
        fs::symlink_metadata(&directory).map_err(|error| format!("template {name}: {error}"))?;
    if !metadata.file_type().is_dir() {
        return Err("template must be a real directory, not a symlink".into());
    }
    let canonical = fs::canonicalize(&directory).map_err(|error| error.to_string())?;
    if canonical.parent() != Some(root.as_path()) {
        return Err("template directory escapes templates root".into());
    }
    Ok(directory)
}

fn write_new(path: &Path, text: &str) -> Result<(), String> {
    private_file(path, true)
        .and_then(|mut file| file.write_all(text.as_bytes()))
        .map_err(|error| error.to_string())
}
