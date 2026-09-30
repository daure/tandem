use super::*;

fn success(output: Output) -> String {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap()
}

#[test]
fn template_lists_include_configuration_errors_and_workspace_templates() {
    let fixture = Fixture::new();
    fs::create_dir(fixture.home.join("templates/blank")).unwrap();
    fs::write(fixture.home.join("templates/blank/tandem.json"), "{}").unwrap();
    fs::create_dir(fixture.home.join("templates/broken")).unwrap();
    fs::write(fixture.home.join("templates/broken/tandem.json"), "{").unwrap();

    let text = success(fixture.run(&["list-templates"]));
    assert!(text.contains("NAME"), "{text}");
    assert!(text.contains("Workspace"), "{text}");
    assert!(text.contains("Compose"), "{text}");
    assert!(text.contains("Invalid: tandem.json:"), "{text}");

    let listing: serde_json::Value =
        serde_json::from_str(&success(fixture.run(&["list-templates", "--json"]))).unwrap();
    let templates = listing["templates"].as_array().unwrap();
    assert_eq!(templates.len(), 3);
    assert_eq!(templates[0]["name"], "blank");
    assert_eq!(templates[1]["name"], "broken");
    assert!(
        templates[1]["error"]
            .as_str()
            .unwrap()
            .contains("tandem.json")
    );
    assert_eq!(templates[2]["name"], "website");
    assert_eq!(templates[2]["manifest"]["repositories"][0]["target"], "app");
}

#[test]
fn template_inspection_exposes_sources_and_configured_paths_without_startup() {
    let fixture = Fixture::new();
    let template_directory = fixture.home.join("templates/website");
    let compose = "services:\n  web:\n    image: nginx\n  setup:\n    image: alpine\n";
    let guidance = "## Website\nRun the application tests.\n";
    fs::write(template_directory.join("compose.yaml"), compose).unwrap();
    fs::write(template_directory.join("tandem-agents.md"), guidance).unwrap();
    let files = template_directory.join("tandem-files");
    fs::create_dir_all(files.join("config")).unwrap();
    fs::write(files.join("config/settings.json"), "secret seed content").unwrap();
    fs::write(
        template_directory.join("tandem.json"),
        json!({
            "description": "Website fixture",
            "repositories": [{"source": fixture.source, "target": "app"}],
            "routes": {"web": {
                "port": 80, "readiness_path": "", "readiness_contains": "Welcome"
            }},
            "one_shots": ["setup"]
        })
        .to_string(),
    )
    .unwrap();

    let text = success(fixture.run(&["inspect-template", "website"]));
    for expected in [
        "Template: website",
        "Type: Compose",
        "Website fixture",
        "Configured gateway URL: http://localhost:9876",
        "\"repositories\"",
        "\"routes\"",
        "\"setup\"",
        compose,
        guidance,
        "Files directory:",
        "tandem-files",
        "Files (copied into workspace root):\nconfig/\n  settings.json",
    ] {
        assert!(text.contains(expected), "missing {expected:?}: {text}");
    }
    let inspection: serde_json::Value = serde_json::from_str(&success(fixture.run(&[
        "inspect-template",
        "website",
        "--json",
    ])))
    .unwrap();
    assert_eq!(inspection["gateway_origin"], "http://localhost:9876");
    assert_eq!(
        inspection["templates_root"],
        fixture.home.join("templates").display().to_string()
    );
    assert_eq!(
        inspection["template"]["directory"],
        template_directory.display().to_string()
    );
    assert_eq!(inspection["template"]["compose_source"], compose);
    assert_eq!(inspection["template"]["guidance_source"], guidance);
    assert_eq!(
        inspection["template"]["files_directory"],
        files.display().to_string()
    );
    assert_eq!(inspection["template"]["files"][0]["path"], "config");
    assert_eq!(inspection["template"]["files"][0]["directory"], true);
    assert_eq!(
        inspection["template"]["files"][1]["path"],
        "config/settings.json"
    );
    assert_eq!(inspection["template"]["files"][1]["directory"], false);
    assert!(!text.contains("secret seed content"));
    assert_eq!(
        inspection["template"]["manifest"]["routes"]["web"]["port"],
        80
    );
    for path in ["configured", "started", "opened", "workspaces/review"] {
        assert!(
            !fixture.home.join(path).exists(),
            "inspection created {path}"
        );
    }
}

#[test]
fn template_inspection_reports_missing_and_invalid_templates() {
    let fixture = Fixture::new();
    let missing = fixture.run(&["inspect-template", "missing", "--json"]);
    assert!(!missing.status.success());
    assert!(String::from_utf8_lossy(&missing.stderr).contains("template missing:"));
    assert!(missing.stdout.is_empty());

    fs::write(fixture.home.join("templates/website/tandem.json"), "{").unwrap();
    let invalid = fixture.run(&["inspect-template", "website"]);
    assert!(!invalid.status.success());
    assert!(String::from_utf8_lossy(&invalid.stderr).contains("tandem.json:"));
    assert!(invalid.stdout.is_empty());
}

#[test]
fn instance_lists_expose_runtime_evidence_and_workspace_paths() {
    let fixture = Fixture::new();
    fs::write(fixture.home.join("started"), "").unwrap();
    let text = success(fixture.run(&["list-instances"]));
    for expected in ["NAME", "STATUS", "review", "website", "workspaces/review"] {
        assert!(text.contains(expected), "{text}");
    }
    let inventory: serde_json::Value =
        serde_json::from_str(&success(fixture.run(&["list-instances", "--json"]))).unwrap();
    assert_eq!(inventory["instances"].as_array().unwrap().len(), 1);
    let instance = &inventory["instances"][0];
    assert_eq!(instance["name"], "review");
    assert_eq!(instance["template"], "website");
    assert_eq!(
        instance["workspace"],
        fixture.home.join("workspaces/review").display().to_string()
    );
    assert_eq!(instance["services"][0]["runtime"]["state"], "running");
    assert!(instance["summary"]["label"].is_string());
    assert!(inventory["runtime_error"].is_null());
}

#[test]
fn empty_inventory_lists_are_explicit_in_text_and_json() {
    let fixture = Fixture::new();
    fs::remove_file(fixture.home.join("templates/website/compose.yaml")).unwrap();
    fs::remove_file(fixture.home.join("templates/website/tandem.json")).unwrap();
    assert_eq!(
        success(fixture.run(&["list-templates"])),
        "No templates found.\n"
    );
    assert_eq!(
        success(fixture.run(&["list-instances"])),
        "No instances found.\n"
    );
    let templates: serde_json::Value =
        serde_json::from_str(&success(fixture.run(&["list-templates", "--json"]))).unwrap();
    let instances: serde_json::Value =
        serde_json::from_str(&success(fixture.run(&["list-instances", "--json"]))).unwrap();
    assert_eq!(templates["templates"], json!([]));
    assert_eq!(instances["instances"], json!([]));
}

#[test]
fn partial_instance_inventory_preserves_workspace_rows_and_exits_nonzero() {
    let fixture = Fixture::new();
    fs::remove_file(fixture.home.join("templates/website/compose.yaml")).unwrap();
    fs::write(fixture.home.join("templates/website/tandem.json"), "{}").unwrap();
    success(fixture.run(&["new-instance", "scratch", "-t", "website"]));
    fs::write(
        fixture.bin.join("docker"),
        "#!/bin/sh\nprintf 'Docker unavailable\\n' >&2\nexit 99\n",
    )
    .unwrap();

    for arguments in [vec!["list-instances"], vec!["list-instances", "--json"]] {
        let output = fixture.run(&arguments);
        assert!(!output.status.success());
        let error = String::from_utf8_lossy(&output.stderr);
        assert!(error.contains("runtime inventory incomplete"), "{error}");
        assert!(error.contains("Docker unavailable"), "{error}");
        let text = String::from_utf8(output.stdout).unwrap();
        if arguments.contains(&"--json") {
            let inventory: serde_json::Value = serde_json::from_str(&text).unwrap();
            assert_eq!(inventory["instances"][0]["name"], "scratch");
            assert_eq!(inventory["instances"][0]["workspace_only"], true);
            assert!(
                inventory["runtime_error"]
                    .as_str()
                    .unwrap()
                    .contains("Docker unavailable")
            );
        } else {
            assert!(text.contains("scratch"), "{text}");
            assert!(text.contains("Workspace ready"), "{text}");
        }
    }
}

#[test]
fn instance_inspection_shows_runtime_details_urls_and_retained_startup_progress() {
    let fixture = Fixture::new();
    success(fixture.run(&["new-instance", "review", "-t", "website"]));
    let mut containers: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(fixture.home.join("inspect.json")).unwrap())
            .unwrap();
    containers[0]["Config"]["Labels"]["io.tandem.url"] = json!("http://localhost:9876/review/app/");
    containers[0]["State"]["Health"]["Status"] = json!("healthy");
    fs::write(fixture.home.join("inspect.json"), containers.to_string()).unwrap();

    let text = success(fixture.run(&["inspect-instance", "review"]));
    for expected in [
        "Instance: review",
        "Template: website",
        "Service: app",
        "Healthcheck: healthy",
        "URL: http://localhost:9876/review/app/",
        "Readiness: Not recorded",
        "Latest startup:",
        "Outcome: succeeded",
        "Waiting for service health and gateway content assertions",
    ] {
        assert!(text.contains(expected), "missing {expected:?}: {text}");
    }
    let inspection: serde_json::Value = serde_json::from_str(&success(fixture.run(&[
        "inspect-instance",
        "review",
        "--json",
    ])))
    .unwrap();
    assert_eq!(inspection["name"], "review");
    assert_eq!(
        inspection["instance"]["services"][0]["runtime"]["health"],
        "healthy"
    );
    assert_eq!(inspection["startup"]["state"], "succeeded");
    assert!(inspection["startup"]["progress"].as_array().unwrap().len() > 1);
}

#[test]
fn instance_inspection_keeps_workspace_evidence_when_docker_is_unavailable() {
    let fixture = Fixture::new();
    fs::remove_file(fixture.home.join("templates/website/compose.yaml")).unwrap();
    fs::write(fixture.home.join("templates/website/tandem.json"), "{}").unwrap();
    success(fixture.run(&["new-instance", "scratch", "-t", "website"]));
    fs::write(
        fixture.bin.join("docker"),
        "#!/bin/sh\nprintf 'Docker unavailable\\n' >&2\nexit 99\n",
    )
    .unwrap();

    let output = fixture.run(&["inspect-instance", "scratch", "--json"]);
    assert!(!output.status.success());
    let inspection: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(inspection["instance"]["name"], "scratch");
    assert_eq!(inspection["instance"]["workspace_only"], true);
    assert!(
        inspection["runtime_error"]
            .as_str()
            .unwrap()
            .contains("Docker unavailable")
    );
    assert!(String::from_utf8_lossy(&output.stderr).contains("runtime inventory incomplete"));
}

#[test]
fn instance_inspection_exposes_retained_lifecycle_failures_without_runtime_containers() {
    let fixture = Fixture::new();
    let stopped = fixture.run(&["stop-instance", "review"]);
    assert!(!stopped.status.success());
    let text = success(fixture.run(&["inspect-instance", "review"]));
    assert!(text.contains("Runtime instance: None observed"), "{text}");
    assert!(
        text.contains("Activity error: instance not found"),
        "{text}"
    );
    let inspection: serde_json::Value = serde_json::from_str(&success(fixture.run(&[
        "inspect-instance",
        "review",
        "--json",
    ])))
    .unwrap();
    assert!(inspection["instance"].is_null());
    assert_eq!(inspection["activities"][0]["action"], "stop_instance");
    assert_eq!(inspection["activities"][0]["error"], "instance not found");
}
