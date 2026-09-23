use std::{fs, sync::Arc, time::Duration};

use serde_json::json;

use super::{Environments, compose, config::Config, docker, gateway, lifecycle, templates};
use crate::store::environments::{
    Instance, InstanceService, OperationState, StartupKind, validate_instance_name, validate_name,
};

mod concurrency;
mod guidance;
mod manifest_updates;
mod restart;
mod template_removal;
mod template_removal_live;
mod workspaces;

fn fixture() -> (tempfile::TempDir, Config) {
    let directory = tempfile::tempdir().unwrap();
    let config = Config::at(directory.path().to_path_buf(), "tandem-test".into(), 9876).unwrap();
    (directory, config)
}

#[test]
fn templates_have_editable_absolute_paths_and_preserve_existing_content() {
    let (_directory, config) = fixture();
    assert!(templates::list(&config).unwrap().is_empty());
    let template = templates::create(&config, "web-app").unwrap();
    assert!(std::path::Path::new(&template.directory).is_absolute());
    assert!(template.compose_source.contains("./site"));
    fs::write(&template.compose_file, "services: {}\n").unwrap();
    assert!(templates::create(&config, "web-app").is_err());
    assert_eq!(
        templates::get(&config, "web-app").unwrap().compose_source,
        "services: {}\n"
    );
    assert_eq!(templates::list(&config).unwrap().len(), 1);
}

#[test]
fn template_manifest_source_preserves_file_contents_for_inspection() {
    let (_directory, config) = fixture();
    let environment = Environments::new(config);
    let template = environment.create_template("website").unwrap();
    let source = "{\n    \"description\" : \"Example\", \"routes\": {}\n}\n";
    fs::write(&template.manifest_file, source).unwrap();
    assert_eq!(
        environment
            .get_template("website")
            .unwrap()
            .manifest_source
            .as_deref(),
        Some(source)
    );

    for invalid in ["{ invalid json", r#"{"one_shots":["../invalid"]}"#] {
        fs::write(&template.manifest_file, invalid).unwrap();
        assert!(environment.get_template("website").is_err());
        let listed = environment.list_templates().unwrap();
        assert!(listed[0].error.is_some());
        assert_eq!(listed[0].manifest_source.as_deref(), Some(invalid));
    }

    fs::remove_file(&template.manifest_file).unwrap();
    let template = environment.get_template("website").unwrap();
    assert!(template.manifest_source.is_none());
    assert!(template.error.is_none());
}

#[test]
fn instructions_are_read_from_the_editable_file_on_every_call() {
    let (_directory, config) = fixture();
    let environment = Environments::new(config.clone());
    assert_eq!(
        environment.instructions().unwrap().markdown,
        include_str!("../../../agent-instructions.md")
    );
    fs::write(
        &config.instructions,
        "# Team guidance\nUse the api template.\n",
    )
    .unwrap();
    assert_eq!(
        environment.instructions().unwrap().markdown,
        "# Team guidance\nUse the api template.\n"
    );
    let reopened = Config::at(config.home.clone(), config.namespace.clone(), config.port).unwrap();
    assert_eq!(
        fs::read_to_string(reopened.instructions).unwrap(),
        "# Team guidance\nUse the api template.\n"
    );
}

#[test]
fn invalid_names_and_manifests_are_rejected_without_hiding_other_templates() {
    let (_directory, config) = fixture();
    for name in [
        "",
        "../escape",
        "gateway",
        "Upper",
        "a/b",
        "x;pwd",
        "-first",
        ".hidden",
    ] {
        assert!(validate_name(name).is_err(), "{name}");
        assert!(templates::create(&config, name).is_err());
    }
    let good = templates::create(&config, "good").unwrap();
    let broken = templates::create(&config, "broken").unwrap();
    fs::write(
        broken.manifest_file,
        r#"{"routes":{"web":{"port":0,"readiness_path":"../secrets","readiness_contains":""}}}"#,
    )
    .unwrap();
    let listed = templates::list(&config).unwrap();
    assert!(
        listed
            .iter()
            .find(|template| template.name == "broken")
            .unwrap()
            .error
            .is_some()
    );
    assert_eq!(
        listed
            .iter()
            .find(|template| template.name == "good")
            .unwrap(),
        &good
    );
}

#[test]
fn instance_names_preserve_capital_letters() {
    assert!(validate_instance_name("Feature-Branch").is_ok());
    assert!(validate_instance_name("Feature Branch").is_err());
    let (_directory, config) = fixture();
    assert_eq!(
        config.project("Feature-Branch"),
        "tandem-test-feature-branch"
    );
    let environment = Environments::new(config);
    assert!(
        environment
            .begin("create_instance", "Feature-Branch", Some("website".into()))
            .is_ok()
    );
    assert!(
        environment
            .begin("create_template", "Feature-Branch", None)
            .is_err()
    );
}

#[test]
fn pending_instance_creation_is_visible_before_containers_exist() {
    let (_directory, config) = fixture();
    let environment = Environments::new(config.clone());
    templates::create(&config, "website").unwrap();

    environment
        .begin("create_instance", "review", Some("website".into()))
        .unwrap();

    let snapshot = environment.snapshot();
    assert_eq!(snapshot.instances.len(), 1);
    assert_eq!(snapshot.instances[0].name, "review");
    assert!(snapshot.instances[0].pending);
    assert_eq!(
        snapshot.instances[0].workspace,
        config.workspaces.join("review").display().to_string()
    );
}

#[test]
fn startup_kind_distinguishes_new_and_existing_instances() {
    let (_directory, config) = fixture();
    let environment = Environments::new(config);
    let cold = environment
        .begin("create_instance", "review", Some("website".into()))
        .unwrap();
    assert_eq!(environment.startup_kind(&cold.id), Some(StartupKind::Cold));

    let ready = environment.snapshot().instances[0].clone();
    environment.complete_instance_for_tests(&cold.id, ready);
    let hot = environment
        .begin("create_instance", "review", Some("website".into()))
        .unwrap();
    assert_eq!(environment.startup_kind(&hot.id), Some(StartupKind::Hot));
}

#[test]
fn completed_stop_remains_projected_until_fresh_inventory_arrives() {
    let (_directory, config) = fixture();
    let environment = Environments::new(config);
    let instance = Instance {
        name: "review".into(),
        template: "website".into(),
        template_directory: "/tmp/templates/website".into(),
        workspace: "/tmp/workspaces/review".into(),
        ..Default::default()
    };
    environment
        .snapshot
        .lock()
        .unwrap()
        .instances
        .push(instance.clone());
    let operation = environment.begin("stop_instance", "review", None).unwrap();

    environment.finish_operation(&operation.id, Ok(None));

    let waiting = environment.snapshot();
    assert_eq!(
        waiting.instances[0]
            .runtime
            .activity
            .as_ref()
            .map(|activity| activity.status()),
        Some(crate::store::environments::Status::Stopping)
    );
    environment.publish_instances(Ok(vec![instance.clone()]), 0);
    assert_eq!(
        environment.snapshot().instances[0]
            .runtime
            .activity
            .as_ref()
            .map(|activity| activity.status()),
        Some(crate::store::environments::Status::Stopping)
    );
    let mut stopped = instance;
    stopped.runtime.whole_stop = true;
    environment.publish_instances(Ok(vec![stopped]), 1);
    assert!(
        environment.snapshot().instances[0]
            .runtime
            .activity
            .is_none()
    );
}

#[test]
fn pending_instance_shows_expected_services_before_docker_discovers_containers() {
    let (_directory, config) = fixture();
    let environment = Environments::new(config.clone());
    templates::create(&config, "website").unwrap();
    let operation = environment
        .begin("create_instance", "review", Some("website".into()))
        .unwrap();
    let service = InstanceService {
        name: "web".into(),
        container_id: String::new(),
        status: "created".into(),
        one_shot: false,
        image: Some("nginx".into()),
        health: None,
        restart_policy: None,
        restart_count: 0,
        created_at: None,
        started_at: None,
        port: Some(80),
        url: Some("http://localhost:9876/review/web/".into()),
        usage: None,
        memory_limit_bytes: None,
        volumes: Vec::new(),
        ..Default::default()
    };
    let mut database = service.clone();
    database.name = "db".into();
    database.url = None;
    database.port = None;
    environment.set_pending_services(&operation.id, vec![service, database]);

    let snapshot = environment.snapshot();
    assert_eq!(snapshot.instances[0].services[0].name, "web");
    assert_eq!(snapshot.instances[0].services[0].status, "created");
    let mut discovered = snapshot.instances.clone();
    discovered[0].pending = false;
    discovered[0].services.truncate(1);
    discovered[0].services[0].status = "boot".into();
    super::merge_pending_instances(&mut discovered, &snapshot.instances);
    assert_eq!(discovered[0].services[0].status, "boot");
    assert!(
        discovered[0]
            .services
            .iter()
            .any(|service| service.name == "db")
    );
}

#[test]
fn gateway_labels_use_instance_service_boundaries_and_opt_in_prefix_stripping() {
    let (_directory, config) = fixture();
    let mut template = templates::create(&config, "web-app").unwrap();
    let mut model = json!({"services":{"web":{"image":"nginx","volumes":[{"type":"bind","source":"/template/site","target":"/srv"}]}},"networks":{"default":{}}});
    compose::decorate(
        &config,
        &template,
        "review",
        "Review environment",
        &mut model,
    )
    .unwrap();
    let labels = &model["services"]["web"]["labels"];
    assert_eq!(labels[compose::URL], "http://localhost:9876/review/web/");
    assert_eq!(labels[compose::PORT], "80");
    assert_eq!(labels[compose::DESCRIPTION], "Review environment");
    assert_eq!(
        labels["traefik.http.routers.tandem-test-i6-review-s3-web.rule"],
        "PathPrefix(`/review/web/`)"
    );
    assert_eq!(
        labels["traefik.http.middlewares.tandem-test-i6-review-s3-web.stripprefix.prefixes"],
        "/review/web"
    );
    assert_eq!(labels[compose::DIRECTORY], template.directory);
    assert_eq!(model["networks"]["tandem-ingress"]["external"], true);
    assert_eq!(
        model["services"]["web"]["volumes"][0]["source"],
        "/template/site"
    );
    template
        .manifest
        .routes
        .get_mut("web")
        .unwrap()
        .strip_prefix = false;
    let mut model = json!({"services":{"web":{"image":"nginx"}}});
    compose::decorate(&config, &template, "review", "", &mut model).unwrap();
    assert!(
        model["services"]["web"]["labels"]
            .get("traefik.http.routers.tandem-test-i6-review-s3-web.middlewares")
            .is_none()
    );
}

#[test]
fn compose_rejects_host_ports_conflicting_labels_and_unknown_routes() {
    let (_directory, config) = fixture();
    let template = templates::create(&config, "web-app").unwrap();
    for service in [
        json!({"ports":[{"published":"8000","target":80}]}),
        json!({"network_mode":"host"}),
        json!({"container_name":"shared"}),
        json!({"profiles":["optional"]}),
        json!({"labels":{"traefik.enable":"true"}}),
    ] {
        let mut model = json!({"services":{"web":service}});
        assert!(compose::decorate(&config, &template, "review", "", &mut model).is_err());
    }
    assert!(
        compose::decorate(
            &config,
            &template,
            "review",
            "",
            &mut json!({"services":{"database":{}}})
        )
        .is_err()
    );
}

#[test]
fn runtime_status_distinguishes_missing_probes_and_failed_one_shots() {
    assert_eq!(
        docker::service_status(&json!({"Status":"running"}), false),
        "up"
    );
    assert_eq!(
        docker::service_status(
            &json!({"Status":"running","Health":{"Status":"healthy"}}),
            false
        ),
        "healthy"
    );
    assert_eq!(
        docker::service_status(
            &json!({"Status":"running","Health":{"Status":"starting"}}),
            false
        ),
        "boot"
    );
    assert_eq!(
        docker::service_status(&json!({"Status":"exited","ExitCode":0}), true),
        "exited 0"
    );
    assert_eq!(
        docker::service_status(&json!({"Status":"exited","ExitCode":1}), true),
        "down (exit 1)"
    );
    assert_eq!(
        docker::service_status(&json!({"Status":"exited","ExitCode":0}), false),
        "down (exit 0)"
    );
}

#[test]
fn inventory_uses_container_labels_and_excludes_the_gateway() {
    let (_directory, config) = fixture();
    let container = |project: &str| {
        json!({"Id":"container-id","Config":{"Labels":{
        "com.docker.compose.project":project,"com.docker.compose.service":"web",
        (compose::NAMESPACE):config.namespace,(compose::INSTANCE):"review",(compose::TEMPLATE):"website",(compose::DIRECTORY):"/stale/template",
        (compose::WORKSPACE):"/workspace/review",(compose::ROLE):"service",(compose::URL):"http://localhost:9876/review/web/",
        "traefik.http.services.review-web.loadbalancer.server.port":"8080"
    },"Image":"nginx:latest"},"HostConfig":{"Memory":536870912,"RestartPolicy":{"Name":"unless-stopped"}},"Mounts":[{"Type":"volume","Name":"db-data"},{"Type":"bind","Source":"/workspace"}],"RestartCount":2,"Created":"2026-09-16T12:00:00Z","State":{"Status":"running","StartedAt":"2026-09-16T12:01:00Z","Health":{"Status":"healthy"}}})
    };
    let instances = docker::instances(
        &config,
        &[
            container("tandem-test-review"),
            container("tandem-test-gateway"),
            container("other-review"),
        ],
    )
    .unwrap();
    assert_eq!(instances.len(), 1);
    assert_eq!(instances[0].template_directory, "/stale/template");
    assert_eq!(instances[0].services[0].status, "healthy");
    assert_eq!(instances[0].services[0].port, Some(8080));
    assert_eq!(
        instances[0].services[0].image.as_deref(),
        Some("nginx:latest")
    );
    assert_eq!(instances[0].services[0].health.as_deref(), Some("healthy"));
    assert_eq!(instances[0].services[0].restart_count, 2);
    assert_eq!(instances[0].services[0].memory_limit_bytes, Some(536870912));
    assert_eq!(instances[0].services[0].volumes, ["db-data"]);
    let mut capitalized = container("tandem-test-review");
    capitalized["Config"]["Labels"][compose::INSTANCE] = json!("Review");
    assert_eq!(
        docker::instances(&config, &[capitalized]).unwrap()[0].name,
        "Review"
    );
    let mut uncapped = container("tandem-test-review");
    uncapped["HostConfig"]["Memory"] = json!(0);
    assert_eq!(
        docker::instances(&config, &[uncapped]).unwrap()[0].services[0].memory_limit_bytes,
        None
    );
}

#[test]
fn inventory_retries_when_a_container_disappears_between_listing_and_inspection() {
    let (_directory, config) = fixture();
    let mut responses = std::collections::VecDeque::from([
        Ok("removed-container".into()),
        Err("Docker exited 1: Error: No such object: removed-container".into()),
        Ok(String::new()),
    ]);

    let instances = docker::inspect_with(
        &config,
        std::time::Instant::now() + Duration::from_secs(1),
        &mut |_, _, _| responses.pop_front().unwrap(),
    )
    .unwrap();

    assert!(instances.is_empty());
    assert!(responses.is_empty());
}

#[test]
fn operation_admission_rejects_duplicates_and_locks_release_on_drop() {
    let (_directory, config) = fixture();
    let environment = Arc::new(Environments::new(config.clone()));
    let operation = environment
        .begin("create_template", "website", None)
        .unwrap();
    assert!(
        environment
            .begin("create_template", "website", None)
            .is_err()
    );
    environment.execute(
        operation.clone(),
        60,
        super::Startup::default(),
        Ok(String::new()),
    );
    assert_eq!(
        environment.operation(&operation.id).unwrap().state,
        OperationState::Succeeded
    );
    let held = gateway::lock(&config, "instance-review").unwrap();
    assert!(gateway::lock(&config, "instance-review").is_err());
    drop(held);
    assert!(gateway::lock(&config, "instance-review").is_ok());
}

#[test]
fn compose_commands_expose_the_instance_name_as_the_branch_when_requested() {
    let (_directory, config) = fixture();
    let command = compose::command(
        &config,
        std::path::Path::new("/template"),
        std::path::Path::new("/template/compose.yaml"),
        "Feature-Branch",
        Some("Feature-Branch"),
    );
    let branch = command.get_envs().find_map(|(key, value)| {
        (key == std::ffi::OsStr::new("TANDEM_BRANCH"))
            .then(|| value.map(|value| value.to_string_lossy().into_owned()))
            .flatten()
    });

    assert_eq!(branch.as_deref(), Some("Feature-Branch"));
}

#[test]
fn operation_history_is_chronological_and_evicts_the_oldest_finished_job() {
    let (_directory, config) = fixture();
    let environment = Environments::new(config);
    let start = std::time::Instant::now() - Duration::from_secs(200);
    let mut admitted = Vec::new();
    for number in 0..100 {
        let operation = environment
            .begin("create_template", &format!("template-{number}"), None)
            .unwrap();
        let mut jobs = environment.operations.lock().unwrap();
        let job = jobs.get_mut(&operation.id).unwrap();
        job.started = start + Duration::from_secs(number);
        job.operation.state = OperationState::Succeeded;
        admitted.push(operation.id);
    }
    assert_eq!(
        environment
            .operations()
            .iter()
            .map(|operation| operation.id.clone())
            .collect::<Vec<_>>(),
        admitted
    );
    environment.begin("create_template", "next", None).unwrap();
    assert!(environment.operation(&admitted[0]).is_err());
    environment.begin("create_template", "last", None).unwrap();
    assert!(environment.operation(&admitted[1]).is_err());
    assert!(environment.operation(&admitted[9]).is_ok());
}

#[test]
fn instance_deletion_removes_only_its_workspace() {
    let (_directory, config) = fixture();
    let workspace = config.workspaces.join("review");
    fs::create_dir_all(&workspace).unwrap();
    fs::write(workspace.join("data.txt"), "instance data").unwrap();

    lifecycle::remove_workspace(&config, "review", Arc::new(|_| {}), &Default::default()).unwrap();

    assert!(!workspace.exists());
}

#[test]
fn instance_deletion_removes_its_rendered_compose_file() {
    let (_directory, config) = fixture();
    let template = templates::create(&config, "website").unwrap();
    let rendered =
        std::path::Path::new(&template.directory).join(".tandem-tandem-test-review.compose.json");
    fs::write(&rendered, "{}").unwrap();

    lifecycle::remove_rendered_compose(&config, "website", "review", Arc::new(|_| {})).unwrap();

    assert!(!rendered.exists());
}

#[cfg(unix)]
#[test]
fn instance_deletion_rejects_a_workspace_symlink_that_escapes_its_root() {
    let (_directory, config) = fixture();
    let outside = tempfile::tempdir().unwrap();
    fs::write(outside.path().join("data.txt"), "keep").unwrap();
    std::os::unix::fs::symlink(outside.path(), config.workspaces.join("review")).unwrap();

    assert!(
        lifecycle::remove_workspace(&config, "review", Arc::new(|_| {}), &Default::default())
            .is_err()
    );
    assert!(outside.path().join("data.txt").exists());
}

#[cfg(unix)]
#[test]
fn command_deadline_covers_descendants_holding_output_pipes_open() {
    let mut command = std::process::Command::new("sh");
    command.args(["-c", "sleep 2 & exit 0"]);
    let start = std::time::Instant::now();
    let result = super::command::run(command, Duration::from_millis(100), None);
    assert!(result.unwrap_err().contains("timed out"));
    assert!(start.elapsed() < Duration::from_secs(1));
}

#[cfg(unix)]
#[test]
fn verbose_command_progress_is_bounded_without_failing_a_successful_command() {
    let mut command = std::process::Command::new("sh");
    command.args(["-c", "i=0; while [ $i -lt 9000 ]; do printf 'build step done\\n' >&2; i=$((i + 1)); done; printf ready"]);
    let result = super::command::run(command, Duration::from_secs(5), Some(Arc::new(|_| {})));
    assert_eq!(result.unwrap(), "ready");
}

#[cfg(unix)]
#[test]
fn symlinked_templates_cannot_escape_the_template_root() {
    let (_directory, config) = fixture();
    let outside = tempfile::tempdir().unwrap();
    fs::write(outside.path().join("compose.yaml"), "services: {}\n").unwrap();
    std::os::unix::fs::symlink(outside.path(), config.templates.join("escape")).unwrap();
    assert!(templates::get(&config, "escape").is_err());
    assert!(templates::list(&config).unwrap()[0].error.is_some());
}
