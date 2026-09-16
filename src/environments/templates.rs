use std::{fs, io::Write, path::Path};

use super::config::{Config, private_file, read_text};
use crate::store::environments::{Manifest, Template, validate_name};

pub(crate) fn list(config: &Config) -> Result<Vec<Template>, String> {
    let mut templates = Vec::new();
    for entry in fs::read_dir(&config.templates).map_err(|error| error.to_string())? {
        let entry = entry.map_err(|error| error.to_string())?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if validate_name(&name).is_ok() && entry.path().join("compose.yaml").is_file() {
            match get(config, &name) {
                Ok(template) => templates.push(template),
                Err(error) => templates.push(Template {
                    name,
                    directory: entry.path().display().to_string(),
                    compose_file: entry.path().join("compose.yaml").display().to_string(),
                    manifest_file: entry.path().join("tandem.json").display().to_string(),
                    compose_source: String::new(),
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
    let manifest: Manifest = if manifest_path.exists() {
        serde_json::from_str(&read_text(&manifest_path)?)
            .map_err(|error| format!("tandem.json: {error}"))?
    } else {
        Manifest::default()
    };
    validate_manifest(&manifest)?;
    Ok(Template {
        name: name.into(),
        directory: directory.display().to_string(),
        compose_file: directory.join("compose.yaml").display().to_string(),
        manifest_file: manifest_path.display().to_string(),
        compose_source: read_text(&directory.join("compose.yaml"))?,
        manifest,
        error: None,
    })
}

fn validate_manifest(manifest: &Manifest) -> Result<(), String> {
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
    let directory = config.templates.join(name);
    fs::create_dir(&directory).map_err(|error| format!("create template {name}: {error}"))?;
    write_new(
        &directory.join("compose.yaml"),
        include_str!("scaffold/compose.yaml"),
    )?;
    write_new(
        &directory.join("tandem.json"),
        include_str!("scaffold/tandem.json"),
    )?;
    fs::create_dir(directory.join("site")).map_err(|error| error.to_string())?;
    write_new(
        &directory.join("site/index.html"),
        include_str!("scaffold/index.html"),
    )?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(
            directory.join("site/index.html"),
            fs::Permissions::from_mode(0o644),
        )
        .map_err(|error| error.to_string())?;
    }
    write_new(&directory.join(".gitignore"), ".tandem-*.compose.json\n")?;
    get(config, name)
}

fn write_new(path: &Path, text: &str) -> Result<(), String> {
    private_file(path, true)
        .and_then(|mut file| file.write_all(text.as_bytes()))
        .map_err(|error| error.to_string())
}
