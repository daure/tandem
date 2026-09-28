use super::*;
use crate::store::environments::{ContainerState, ServiceRuntime, Status};

#[test]
fn service_preview_combines_yaml_names_and_manifest_roles_without_resolving_compose() {
    let (_directory, config) = fixture();
    let mut template = compose_template(&config, "website");
    template.compose_source = r#"
x-services: &shared
  db: {image: postgres}
services:
  <<: *shared
  web:
    extends: {file: '${TANDEM_WORKSPACE}/app/compose.yaml', service: web}
    deploy: {replicas: '${WEB_REPLICAS:-2}'}
  disabled: {image: alpine, scale: 0}
include: ['${TANDEM_WORKSPACE}/app/infra.yaml']
"#
    .into();
    template.manifest.one_shots = vec!["migrate".into()];

    let services = compose::preview_services(&config, &template, "review");

    assert_eq!(
        services
            .iter()
            .map(|service| service.name.as_str())
            .collect::<Vec<_>>(),
        ["db", "disabled", "migrate", "web"]
    );
    assert!(services.iter().all(|service| {
        service.container_id.is_empty()
            && service.runtime.replica == 1
            && service.status_summary().status == Status::Waiting
            && !service.ready()
            && !service.can_start()
            && !service.can_stop()
            && !service.can_restart()
    }));
    assert!(services[2].one_shot);
    assert_eq!(services[3].port, Some(80));
    assert_eq!(
        services[3].url.as_deref(),
        Some("http://localhost:9876/review/web/")
    );
    assert!(!config.workspaces.join("review").exists());
}

#[test]
fn service_preview_accepts_json_and_keeps_manifest_hints_when_yaml_cannot_be_read() {
    let (_directory, config) = fixture();
    let mut template = compose_template(&config, "website");
    template.manifest.one_shots = vec!["migrate".into()];
    for source in [
        r#"{"services":{"web":{"image":"nginx"},"migrate":{"image":"alpine"}}}"#,
        "include: [./repo/compose.yaml]",
        "services: [invalid",
        "services: {<<: invalid}",
    ] {
        template.compose_source = source.into();
        let services = compose::preview_services(&config, &template, "review");
        assert_eq!(
            services
                .iter()
                .map(|service| service.name.as_str())
                .collect::<Vec<_>>(),
            ["migrate", "web"],
            "{source}"
        );
    }
    let workspace = templates::create(&config, "workspace").unwrap();
    assert!(compose::preview_services(&config, &workspace, "review").is_empty());
}

#[test]
fn startup_publishes_service_preview_before_external_commands_and_workspace_preparation() {
    let (_directory, config) = fixture();
    compose_template(&config, "website");
    let environment = Environments::new(config.clone());
    let operation = environment
        .begin("create_instance", "review", Some("website".into()))
        .unwrap();
    let mut publications = 0;

    // An expired deadline prevents external commands; local discovery still supplies the tree.
    let result = lifecycle::start(
        &config,
        "website",
        "review",
        Default::default(),
        0,
        Arc::new(|_| {}),
        |services| {
            publications += 1;
            environment.set_pending_services(&operation.id, services);
        },
    );

    assert_eq!(result.unwrap_err(), "operation deadline reached");
    assert_eq!(publications, 1);
    let snapshot = environment.snapshot();
    let instance = &snapshot.instances[0];
    assert_eq!(instance.services.len(), 1);
    assert_eq!(instance.services[0].name, "web");
    assert_eq!(instance.services[0].summary.status, Status::Waiting);
    assert!(!config.workspaces.join("review").exists());
    assert!(journal::recorded(&config, "review").unwrap().is_none());
}

#[test]
fn resolved_services_replace_preview_slots_preserve_containers_and_reject_stale_refreshes() {
    let (_directory, config) = fixture();
    let mut template = compose_template(&config, "website");
    template.compose_source = "services: {web: {}, disabled: {scale: 0}}".into();
    let environment = Environments::new(config);
    let operation = environment
        .begin("create_instance", "review", Some("website".into()))
        .unwrap();
    let preview = compose::preview_services(&environment.config, &template, "review");
    environment.set_pending_services(&operation.id, preview);
    let revision = environment
        .instance_revision
        .load(std::sync::atomic::Ordering::SeqCst);
    environment.publish_instances(Ok(Vec::new()), revision);
    let mut observed = environment.snapshot().instances[0].clone();
    observed.pending = false;
    let mut web = observed
        .services
        .iter()
        .find(|service| service.name == "web")
        .unwrap()
        .clone();
    web.container_id = "web-id".into();
    web.status = "running".into();
    web.runtime.state = ContainerState::Running;
    observed.services = vec![web];
    environment.publish_instances(Ok(vec![observed.clone()]), revision);

    let resolved = [1, 2].map(|replica| InstanceService {
        name: "web".into(),
        status: "created".into(),
        runtime: ServiceRuntime {
            replica,
            state: ContainerState::Missing,
            waiting: true,
            ..Default::default()
        },
        ..Default::default()
    });
    environment.set_pending_services(&operation.id, resolved.into());
    let snapshot = environment.snapshot();
    let services = &snapshot.instances[0].services;
    assert_eq!(services.len(), 2);
    assert_eq!(services[0].container_id, "web-id");
    assert_eq!(services[0].state(), ContainerState::Running);
    assert_eq!(services[1].name, "web");
    assert_eq!(services[1].runtime.replica, 2);
    assert_eq!(services[1].summary.status, Status::Waiting);

    environment.publish_instances(Ok(Vec::new()), revision);
    assert_eq!(environment.snapshot().instances[0].services, *services);
    let revision = environment
        .instance_revision
        .load(std::sync::atomic::Ordering::SeqCst);
    environment.publish_instances(Ok(vec![observed]), revision);
    assert_eq!(environment.snapshot().instances[0].services, *services);
}
