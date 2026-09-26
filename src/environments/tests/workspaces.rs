use super::*;
use crate::environments::{journal, ownership, removal};
use crate::store::environments::{RepositoryCheckout, Status, Template};

fn workspace_template(config: &Config) -> Template {
    let directory = config.templates.join("local");
    fs::create_dir(&directory).unwrap();
    fs::write(
        directory.join("tandem.json"),
        json!({"repositories": [{"source": "/source", "target": "app"}]}).to_string(),
    )
    .unwrap();
    templates::get(config, "local").unwrap()
}

fn ready_workspace(config: &Config, template: &Template, name: &str) {
    journal::prepare(config, template, name, None).unwrap();
    ownership::record(
        config,
        &template.name,
        std::path::Path::new(&template.directory),
        name,
    )
    .unwrap();
    fs::create_dir_all(config.workspaces.join(name).join("app")).unwrap();
    journal::checkout(
        config,
        name,
        RepositoryCheckout {
            target: "app".into(),
            path: config
                .workspaces
                .join(name)
                .join("app")
                .display()
                .to_string(),
            cloned: true,
        },
    )
    .unwrap();
    journal::workspace_ready(config, name).unwrap();
}

#[test]
fn workspace_templates_accept_empty_manifests_and_reject_service_metadata_atomically() {
    let (_directory, config) = fixture();
    let template = workspace_template(&config);
    assert!(template.workspace_only());
    assert_eq!(templates::list(&config).unwrap().len(), 1);
    let blank = templates::update_manifest(&config, "local", Default::default()).unwrap();
    assert!(blank.workspace_only());
    assert!(blank.manifest.repositories.is_empty());
    assert!(blank.guidance_source.is_none());
    assert_eq!(templates::get(&config, "local").unwrap(), blank);
    let original = fs::read(&template.manifest_file).unwrap();
    for value in [
        json!({"repositories": template.manifest.repositories, "one_shots": ["setup"]}),
        json!({"repositories": template.manifest.repositories, "routes": {"web": {"port": 80, "readiness_path": "", "readiness_contains": "ok"}}}),
    ] {
        assert!(
            templates::update_manifest(&config, "local", serde_json::from_value(value).unwrap())
                .is_err()
        );
        assert_eq!(fs::read(&template.manifest_file).unwrap(), original);
    }
}

#[test]
fn guidance_only_templates_are_discovered_and_accept_optional_manifests() {
    let (_directory, config) = fixture();
    let directory = config.templates.join("notes");
    fs::create_dir(&directory).unwrap();
    assert!(templates::get(&config, "notes").is_err());
    let guidance = "## Notes\n\nKeep {{literal}} intact.\n";
    fs::write(directory.join("tandem-agents.md"), guidance).unwrap();
    let template = templates::get(&config, "notes").unwrap();
    assert!(template.workspace_only());
    assert!(template.manifest_source.is_none());
    assert_eq!(template.guidance_source.as_deref(), Some(guidance));
    assert_eq!(templates::list(&config).unwrap().len(), 1);
    let manifest = crate::store::environments::Manifest::default();
    templates::update_manifest(&config, "notes", manifest.clone()).unwrap();
    for invalid in [
        json!({"one_shots": ["setup"]}),
        json!({"routes": {"web": {"port": 80, "readiness_path": "", "readiness_contains": "ok"}}}),
    ] {
        assert!(
            templates::update_manifest(&config, "notes", serde_json::from_value(invalid).unwrap())
                .is_err()
        );
        assert_eq!(templates::get(&config, "notes").unwrap().manifest, manifest);
    }
    fs::remove_file(directory.join("tandem.json")).unwrap();
    fs::write(directory.join("tandem-agents.md"), [0xff]).unwrap();
    assert!(templates::list(&config).unwrap()[0].error.is_some());
    assert!(templates::get(&config, "notes").is_err());
}

#[test]
fn workspace_inventory_survives_reconnection_and_docker_failure_without_becoming_stale() {
    let (_directory, config) = fixture();
    let template = workspace_template(&config);
    ready_workspace(&config, &template, "Feature-1");
    let environment = Environments::new(config.clone());
    environment.publish_instances(Err("offline".into()), 0);
    let snapshot = environment.snapshot();
    assert!(snapshot.runtime_error.is_some());
    assert_eq!(snapshot.instances.len(), 1);
    let instance = &snapshot.instances[0];
    assert_eq!(instance.summary.status, Status::WorkspaceReady);
    assert_eq!(instance.repositories.len(), 1);
    assert!(!instance.can_start() && !instance.can_stop() && !instance.can_restart());
    assert!(
        environment
            .admit_new_instance("Feature-1", "other")
            .is_err()
    );
    assert!(
        environment
            .admit_new_instance("feature-1", "local")
            .is_err()
    );
    assert!(
        environment
            .admit_new_instance("Feature-1", "local")
            .unwrap()
            .0
            .is_some()
    );
    fs::remove_dir_all(config.workspaces.join("Feature-1")).unwrap();
    let missing = journal::workspace_instance(&config, "Feature-1")
        .unwrap()
        .unwrap();
    assert_eq!(missing.status_summary().status, Status::Failed);
}

#[test]
fn workspace_template_removal_and_purges_release_identity_without_docker() {
    let (_directory, config) = fixture();
    let template = workspace_template(&config);
    ready_workspace(&config, &template, "one");
    ready_workspace(&config, &template, "two");
    lifecycle::delete_template(&config, "local", Arc::new(|_| {}), &Default::default()).unwrap();
    assert!(journal::workspaces(&config).unwrap().is_empty());
    assert!(std::path::Path::new(&template.directory).exists());
    ready_workspace(&config, &template, "three");
    removal::template_with(
        &config,
        "local",
        30,
        Arc::new(|_| {}),
        &Default::default(),
        |_, _, _| panic!("workspace removal must not call Docker"),
    )
    .unwrap();
    assert!(journal::workspaces(&config).unwrap().is_empty());
    assert!(!std::path::Path::new(&template.directory).exists());
    assert!(!config.workspaces.join("three").exists());
}

#[test]
fn launch_kind_changes_and_forged_workspace_ownership_are_rejected() {
    let (_directory, config) = fixture();
    let template = workspace_template(&config);
    ready_workspace(&config, &template, "review");
    fs::write(
        std::path::Path::new(&template.directory).join("compose.yaml"),
        "services: {}\n",
    )
    .unwrap();
    let compose = templates::get(&config, "local").unwrap();
    assert!(journal::prepare(&config, &compose, "review", None).is_err());
    let record_path = config
        .home
        .join("runtime")
        .join(&config.namespace)
        .join("review.json");
    let mut record: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&record_path).unwrap()).unwrap();
    record["expected"]["workspace"] = "/outside".into();
    fs::write(record_path, record.to_string()).unwrap();
    assert!(journal::workspace_instance(&config, "review").is_err());
    assert!(lifecycle::delete(&config, "review", Arc::new(|_| {}), &Default::default()).is_err());
}
