use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
    process::Command,
};

use super::*;
use crate::environments::{ownership, removal};
use crate::store::environments::Template;

#[derive(Default)]
struct Docker {
    containers: BTreeMap<String, serde_json::Value>,
    networks: BTreeMap<String, String>,
    volumes: BTreeMap<String, String>,
    anonymous: BTreeSet<String>,
    mutations: Vec<Vec<String>>,
    fail_volume_removal: bool,
    unavailable: bool,
}

impl Docker {
    fn instance(&mut self, config: &Config, template: &Template, name: &str, states: &[&str]) {
        let workspace = config.workspaces.join(name);
        fs::create_dir(&workspace).unwrap();
        fs::write(workspace.join("data"), "keep").unwrap();
        for (index, state) in states.iter().enumerate() {
            let id = format!("{name}-{index}");
            self.anonymous.insert(format!("{id}-anonymous"));
            self.containers.insert(
                id.clone(),
                json!({
                    "Id": id,
                    "Config": {"Labels": {
                        "com.docker.compose.project": config.project(name),
                        "com.docker.compose.service": format!("service-{index}"),
                        (compose::NAMESPACE): config.namespace,
                        (compose::KIND): "instance",
                        (compose::INSTANCE): name,
                        (compose::TEMPLATE): template.name,
                        (compose::DIRECTORY): template.directory,
                        (compose::WORKSPACE): workspace,
                    }},
                    "State": {"Status": state, "ExitCode": 0}
                }),
            );
        }
        self.networks
            .insert(format!("{name}-network"), config.project(name));
        self.volumes
            .insert(format!("{name}-volume"), config.project(name));
    }

    fn remove(&mut self, config: &Config, name: &str) -> Result<(), String> {
        removal::template_with(
            config,
            name,
            60,
            Arc::new(|_| {}),
            &Default::default(),
            |command, _, _| self.run(command),
        )
    }

    fn run(&mut self, command: Command) -> Result<String, String> {
        let args: Vec<String> = command
            .get_args()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect();
        if self.unavailable {
            return Err("Docker unavailable".into());
        }
        if args[0] == "ps" {
            let filters: Vec<_> = args
                .iter()
                .filter_map(|arg| {
                    arg.strip_prefix("label=")
                        .and_then(|label| label.split_once('='))
                })
                .collect();
            return Ok(self
                .containers
                .iter()
                .filter(|(_, container)| {
                    filters.iter().all(|(key, value)| {
                        container["Config"]["Labels"][*key].as_str() == Some(value)
                    })
                })
                .map(|(id, _)| id.clone())
                .collect::<Vec<_>>()
                .join("\n"));
        }
        if args[0] == "inspect" {
            return Ok(serde_json::to_string(
                &args[1..]
                    .iter()
                    .map(|id| &self.containers[id])
                    .collect::<Vec<_>>(),
            )
            .unwrap());
        }
        if args.get(1).map(String::as_str) == Some("ls") {
            let project = args
                .iter()
                .find_map(|arg| arg.strip_prefix("label=com.docker.compose.project="))
                .unwrap();
            let resources = if args[0] == "network" {
                &self.networks
            } else {
                &self.volumes
            };
            return Ok(resources
                .iter()
                .filter(|(_, owner)| owner.as_str() == project)
                .map(|(name, _)| name.clone())
                .collect::<Vec<_>>()
                .join("\n"));
        }
        self.mutations.push(args.clone());
        match args[0].as_str() {
            "unpause" => {
                for id in &args[1..] {
                    self.containers.get_mut(id).unwrap()["State"]["Status"] = json!("running");
                }
            }
            "stop" => {
                assert_eq!(&args[1..3], ["--time", "10"]);
                for id in &args[3..] {
                    self.containers.get_mut(id).unwrap()["State"]["Status"] = json!("exited");
                }
            }
            "rm" => {
                assert_eq!(&args[1..3], ["--force", "--volumes"]);
                for id in &args[3..] {
                    let container = self.containers.remove(id).unwrap();
                    assert_eq!(container["State"]["Status"], "exited");
                    self.anonymous.remove(&format!("{id}-anonymous"));
                }
            }
            "network" => {
                for name in &args[2..] {
                    self.networks.remove(name).unwrap();
                }
            }
            "volume" => {
                if self.fail_volume_removal {
                    self.fail_volume_removal = false;
                    return Err("volume removal failed".into());
                }
                for name in &args[2..] {
                    self.volumes.remove(name).unwrap();
                }
            }
            _ => panic!("unexpected Docker command: {args:?}"),
        }
        Ok(String::new())
    }
}

#[test]
fn template_deletion_cleans_running_stopped_paused_and_orphaned_instances() {
    let (_directory, config) = fixture();
    let template = compose_template(&config, "website");
    let other = compose_template(&config, "other");
    let mut docker = Docker::default();
    docker.instance(&config, &template, "running", &["running", "paused"]);
    docker.instance(&config, &template, "stopped", &["exited"]);
    docker.instance(&config, &template, "orphan", &[]);
    docker.instance(&config, &template, "failed-start", &[]);
    docker.instance(&config, &other, "unrelated", &["running"]);
    ownership::record(
        &config,
        "website",
        Path::new(&template.directory),
        "failed-start",
    )
    .unwrap();
    let mut model = json!({"services": {"web": {"image": "nginx"}}});
    compose::decorate(&config, &template, "orphan", "", &mut model).unwrap();
    fs::write(
        Path::new(&template.directory).join(".tandem-tandem-test-orphan.compose.json"),
        serde_json::to_vec(&model).unwrap(),
    )
    .unwrap();
    fs::write(&template.manifest_file, "invalid manifest").unwrap();

    let warnings = Arc::new(std::sync::Mutex::new(Vec::new()));
    let received = warnings.clone();
    let close = crate::environments::close_command::CloseCommand::new(
        Ok(
            "test -f data || exit 12; printf '%s\\n' \"$TANDEM_INSTANCE\" >> ../closed; exit 23"
                .into(),
        ),
        Arc::new(move |warning| received.lock().unwrap().push(warning)),
    );
    removal::template_with(
        &config,
        "website",
        60,
        Arc::new(|_| {}),
        &close,
        |command, _, _| docker.run(command),
    )
    .unwrap();

    assert_eq!(
        fs::read_to_string(config.workspaces.join("closed")).unwrap(),
        "failed-start\norphan\nrunning\nstopped\n"
    );
    assert_eq!(warnings.lock().unwrap().len(), 4);

    assert!(!Path::new(&template.directory).exists());
    for name in ["running", "stopped", "orphan", "failed-start"] {
        assert!(!config.workspaces.join(name).exists());
    }
    assert_eq!(
        docker.containers.keys().cloned().collect::<Vec<_>>(),
        ["unrelated-0"]
    );
    assert_eq!(
        docker.networks.keys().cloned().collect::<Vec<_>>(),
        ["unrelated-network"]
    );
    assert_eq!(
        docker.volumes.keys().cloned().collect::<Vec<_>>(),
        ["unrelated-volume"]
    );
    assert_eq!(
        docker.anonymous.into_iter().collect::<Vec<_>>(),
        ["unrelated-0-anonymous"]
    );
    assert_eq!(
        fs::read_to_string(config.workspaces.join("unrelated/data")).unwrap(),
        "keep"
    );
    assert!(Path::new(&other.compose_file).is_file());
    let first_remove = docker
        .mutations
        .iter()
        .position(|args| args[0] == "rm")
        .unwrap();
    assert!(
        docker
            .mutations
            .iter()
            .skip(first_remove)
            .all(|args| args[0] != "stop")
    );
}

#[test]
fn cleanup_failure_keeps_ownership_for_a_complete_retry() {
    let (_directory, config) = fixture();
    let template = compose_template(&config, "website");
    let mut docker = Docker::default();
    docker.instance(&config, &template, "review", &["running"]);
    docker.fail_volume_removal = true;
    assert!(
        docker
            .remove(&config, "website")
            .unwrap_err()
            .contains("volume removal failed")
    );
    assert!(docker.containers.is_empty());
    assert!(Path::new(&template.directory).is_dir());
    assert!(config.workspaces.join("review/data").is_file());
    assert_eq!(
        ownership::instances(&config, "website", Path::new(&template.directory))
            .unwrap()
            .into_iter()
            .collect::<Vec<_>>(),
        ["review"]
    );
    docker.remove(&config, "website").unwrap();
    assert!(docker.volumes.is_empty());
    assert!(!config.workspaces.join("review").exists());
    assert!(!Path::new(&template.directory).exists());
}

#[test]
fn deletion_requires_verified_ownership_and_available_docker() {
    let (_directory, config) = fixture();
    let template = compose_template(&config, "website");
    let mut docker = Docker::default();
    docker.instance(&config, &template, "review", &["running"]);
    let mut foreign = docker.containers["review-0"].clone();
    foreign["Id"] = json!("foreign");
    foreign["Config"]["Labels"][compose::NAMESPACE] = json!("other");
    docker.containers.insert("foreign".into(), foreign);
    assert!(
        docker
            .remove(&config, "website")
            .unwrap_err()
            .contains("unmanaged")
    );
    assert!(docker.mutations.is_empty());
    docker.unavailable = true;
    assert!(
        docker
            .remove(&config, "website")
            .unwrap_err()
            .contains("Docker unavailable")
    );
    assert!(Path::new(&template.compose_file).is_file());
    assert!(config.workspaces.join("review/data").is_file());
}

#[test]
fn template_removal_rejects_unsafe_names_and_busy_templates() {
    let (_directory, config) = fixture();
    let template = compose_template(&config, "website");
    let mut docker = Docker::default();
    for name in ["", "..", "../website", "/tmp/website", "gateway"] {
        assert!(docker.remove(&config, name).is_err());
    }
    let held = gateway::lock(&config, "template-website").unwrap();
    assert!(
        docker
            .remove(&config, "website")
            .unwrap_err()
            .contains("busy")
    );
    assert!(
        templates::create(&config, "website")
            .unwrap_err()
            .contains("busy")
    );
    assert!(
        lifecycle::start(
            &config,
            "website",
            "review",
            crate::environments::Startup::default(),
            5,
            Arc::new(|_| {}),
            |_| {},
        )
        .unwrap_err()
        .contains("busy")
    );
    drop(held);
    docker.remove(&config, "website").unwrap();
    assert!(!Path::new(&template.directory).exists());
}

#[cfg(unix)]
#[test]
fn template_and_workspace_symlinks_cannot_delete_unrelated_data() {
    use std::os::unix::fs::symlink;
    let (_directory, config) = fixture();
    let outside = tempfile::tempdir().unwrap();
    fs::write(outside.path().join("keep"), "data").unwrap();
    symlink(outside.path(), config.templates.join("alias")).unwrap();
    let mut docker = Docker::default();
    assert!(docker.remove(&config, "alias").is_err());
    let template = compose_template(&config, "website");
    docker.instance(&config, &template, "review", &["running"]);
    fs::remove_dir_all(config.workspaces.join("review")).unwrap();
    symlink(outside.path(), config.workspaces.join("review")).unwrap();
    assert!(docker.remove(&config, "website").is_err());
    assert!(docker.mutations.is_empty());
    fs::remove_file(config.workspaces.join("review")).unwrap();
    symlink(
        outside.path(),
        Path::new(&template.directory).join("linked-data"),
    )
    .unwrap();
    docker.remove(&config, "website").unwrap();
    fs::rename(&config.templates, config.home.join("saved-templates")).unwrap();
    symlink(outside.path(), &config.templates).unwrap();
    fs::create_dir(outside.path().join("website")).unwrap();
    assert!(docker.remove(&config, "website").is_err());
    assert!(outside.path().join("website").is_dir());
    assert_eq!(
        fs::read_to_string(outside.path().join("keep")).unwrap(),
        "data"
    );
}
