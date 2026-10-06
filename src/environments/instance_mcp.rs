use std::{
    fs::{self, File, OpenOptions},
    io::Write,
    os::unix::{fs::MetadataExt, fs::OpenOptionsExt},
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

use super::{
    Environments, config::Config, gateway, journal, ownership, removal, repositories, templates,
};
use crate::store::environments::{Instance, InstanceInstructions, RepositoryUpdates};

#[derive(Debug)]
pub(crate) struct InstanceScope {
    home: PathBuf,
    namespace: String,
    name: String,
    template: String,
    workspace: File,
}

impl Environments {
    pub(crate) fn update_instance_repositories(
        &self,
        scope: &InstanceScope,
    ) -> Result<RepositoryUpdates, String> {
        let _lock = gateway::lock(&self.config, &format!("instance-{}", scope.name))?;
        let instance = self.verify_instance_scope(scope)?;
        let template = templates::get(&self.config, &instance.template)?;
        if instance.template_directory != template.directory {
            return Err("instance belongs to a different template directory".into());
        }
        Ok(RepositoryUpdates {
            instance: instance.name,
            repositories: repositories::update(
                Path::new(&instance.workspace),
                &template.manifest.repositories,
                Instant::now() + Duration::from_secs(600),
            )?,
        })
    }

    pub(crate) fn instance_instructions(
        &self,
        scope: &InstanceScope,
    ) -> Result<InstanceInstructions, String> {
        let _lock = gateway::lock(&self.config, &format!("instance-{}", scope.name))?;
        let instance = self.verify_instance_scope(scope)?;
        let file = self.config.instance_instructions();
        Ok(InstanceInstructions {
            file: file.display().to_string(),
            core_guidance: include_str!("../../instance-core-guidance.md").into(),
            markdown: super::config::read_text(&file)?,
            instance: instance.name,
            template: instance.template,
            workspace: instance.workspace,
            namespace: self.config.namespace.clone(),
        })
    }

    pub(crate) fn bind_instance_workspace(&self) -> Result<InstanceScope, String> {
        self.bind_instance_directory(&std::env::current_dir().map_err(|error| error.to_string())?)
    }

    pub(crate) fn bind_instance_directory(
        &self,
        directory: &Path,
    ) -> Result<InstanceScope, String> {
        let directory = fs::canonicalize(directory).map_err(|error| error.to_string())?;
        let relative = directory
            .strip_prefix(&self.config.workspaces)
            .map_err(|_| "MCP directory is outside Tandem's instance workspaces")?;
        let name = relative
            .components()
            .next()
            .and_then(|part| part.as_os_str().to_str())
            .ok_or("MCP directory must belong to an instance workspace")?;
        crate::store::environments::validate_instance_name(name)?;
        let instance =
            journal::recorded(&self.config, name)?.ok_or("instance ownership is missing")?;
        ownership::verify(&self.config, &instance)?;
        removal::validate_workspace(&self.config, name)?;
        // Holding the directory open pins its inode across deletion and name reuse.
        let workspace = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open(&instance.workspace)
            .map_err(|error| format!("cannot pin instance workspace: {error}"))?;
        let scope = InstanceScope {
            home: self.config.home.clone(),
            namespace: self.config.namespace.clone(),
            name: name.into(),
            template: instance.template,
            workspace,
        };
        self.verify_instance_scope(&scope)?;
        Ok(scope)
    }

    pub(crate) fn verify_instance_scope(&self, scope: &InstanceScope) -> Result<Instance, String> {
        if scope.home != self.config.home || scope.namespace != self.config.namespace {
            return Err("instance MCP scope belongs to another Tandem home or namespace".into());
        }
        let instance = journal::recorded(&self.config, &scope.name)?
            .ok_or("instance MCP ownership is missing; reconnect in a prepared workspace")?;
        ownership::verify(&self.config, &instance)?;
        removal::validate_workspace(&self.config, &scope.name)?;
        let current =
            fs::symlink_metadata(&instance.workspace).map_err(|error| error.to_string())?;
        let pinned = scope
            .workspace
            .metadata()
            .map_err(|error| error.to_string())?;
        if instance.template != scope.template
            || !current.is_dir()
            || (current.dev(), current.ino()) != (pinned.dev(), pinned.ino())
        {
            return Err(
                "instance MCP workspace was replaced; reconnect before controlling it".into(),
            );
        }
        Ok(instance)
    }

    pub(crate) fn admit_instance_self(
        &self,
        scope: &InstanceScope,
        start: bool,
    ) -> Result<(Instance, gateway::Lock), String> {
        let lock = gateway::lock(&self.config, &format!("instance-{}", scope.name))?;
        self.verify_instance_scope(scope)?;
        let (instance, _) = super::lifecycle::managed_instance(
            &self.config,
            &scope.name,
            Instant::now() + Duration::from_secs(30),
        )?;
        ownership::verify(&self.config, &instance)?;
        if start {
            let template = templates::get(&self.config, &instance.template)?;
            if instance.template_directory != template.directory
                || instance.workspace_only != template.workspace_only()
            {
                return Err("instance belongs to a different template or execution kind".into());
            }
            if instance.services.iter().any(|service| {
                service.state() == crate::store::environments::ContainerState::Paused
            }) {
                return Err("unpause the instance containers before start".into());
            }
        }
        Ok((instance, lock))
    }

    pub(crate) fn admit_instance_conclusion(
        &self,
        scope: &InstanceScope,
    ) -> Result<(Instance, gateway::Lock), String> {
        let lock = gateway::lock(&self.config, &format!("instance-{}", scope.name))?;
        Ok((self.verify_instance_scope(scope)?, lock))
    }

    pub(crate) fn admit_instance_observation(
        &self,
        scope: &InstanceScope,
    ) -> Result<Option<(Instance, gateway::Lock)>, String> {
        let Some(lock) = gateway::try_lock(&self.config, &format!("instance-{}", scope.name))?
        else {
            return Ok(None);
        };
        Ok(Some((self.verify_instance_scope(scope)?, lock)))
    }
}

pub(super) fn prepare_config(config: &Config, workspace: &Path) -> Result<(), String> {
    // Project configuration is user-owned, including JSONC and template seed files.
    for relative in [
        "opencode.json",
        "opencode.jsonc",
        ".opencode/opencode.json",
        ".opencode/opencode.jsonc",
    ] {
        match fs::symlink_metadata(workspace.join(relative)) {
            Ok(_) => return Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.to_string()),
        }
    }
    let directory = workspace.join(".opencode");
    match fs::symlink_metadata(&directory) {
        Ok(metadata) if metadata.file_type().is_dir() => {}
        Ok(_) => return Err("workspace .opencode must be a real directory".into()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            fs::create_dir(&directory).map_err(|error| error.to_string())?;
        }
        Err(error) => return Err(error.to_string()),
    }
    let executable = std::env::current_exe().map_err(|error| error.to_string())?;
    // V2 normalizes this shared syntax; native V2-only keys would break V1 workspaces.
    let content = serde_json::json!({
        "$schema": "https://opencode.ai/config.json",
        "mcp": {"tandem-instance": {
            "type": "local",
            "command": [executable, "mcp-instance"],
            "cwd": ".",
            "timeout": 660_000,
            "environment": {
                "TANDEM_HOME": config.home,
                "TANDEM_NAMESPACE": config.namespace,
                "TANDEM_GATEWAY_PORT": config.port.to_string()
            }
        }},
        "permission": {
            "tandem-instance_get_instructions": "allow",
            "tandem-instance_start_self": "allow",
            "tandem-instance_stop_self": "allow",
            "tandem-instance_update_repositories": "allow",
            "tandem-instance_conclude": "allow",
            "tandem-instance_search_events": "allow",
            "tandem-instance_get_event_report": "allow"
        }
    });
    let mut file =
        tempfile::NamedTempFile::new_in(&directory).map_err(|error| error.to_string())?;
    writeln!(
        file,
        "{}",
        serde_json::to_string_pretty(&content).map_err(|error| error.to_string())?
    )
    .map_err(|error| error.to_string())?;
    file.as_file()
        .sync_all()
        .map_err(|error| error.to_string())?;
    match file.persist_noclobber(directory.join("opencode.json")) {
        Ok(_) => Ok(()),
        Err(error) if error.error.kind() == std::io::ErrorKind::AlreadyExists => Ok(()),
        Err(error) => Err(error.to_string()),
    }
}
