use super::*;
use std::{
    fs::OpenOptions, io::Write, os::unix::fs::OpenOptionsExt, path::Path, sync::mpsc, time::Instant,
};

use crate::environments::{instance_mcp, ownership, runtime_db};

fn owned_workspace(config: &Config, name: &str) {
    let template = templates::get(config, "blank").unwrap();
    journal::prepare(config, &template, name, None).unwrap();
    ownership::record(config, "blank", Path::new(&template.directory), name).unwrap();
    fs::create_dir_all(config.workspaces.join(name).join("app/src")).unwrap();
    journal::workspace_ready(config, name).unwrap();
}

fn assert_guidance_scope_error(
    environment: &Environments,
    scope: &instance_mcp::InstanceScope,
    config: &Config,
    expected: &str,
) {
    for locked in [false, true] {
        let _lock = locked.then(|| gateway::lock(config, "instance-review").unwrap());
        let error = environment.instance_instructions(scope).unwrap_err();
        assert!(error.contains(expected), "{error}");
    }
}

#[test]
fn instance_control_binds_nested_workspaces_and_holds_the_lifecycle_lock() {
    let (_directory, config) = fixture();
    templates::create(&config, "blank").unwrap();
    owned_workspace(&config, "review");
    owned_workspace(&config, "other");
    let environment = Environments::new(config.clone());
    let scope = environment
        .bind_instance_directory(&config.workspaces.join("review/app/src"))
        .unwrap();
    for start in [false, true] {
        let (instance, lock) = environment.admit_instance_self(&scope, start).unwrap();
        assert_eq!(instance.name, "review");
        assert!(gateway::lock(&config, "instance-review").is_err());
        assert!(gateway::lock(&config, "instance-other").is_ok());
        let error = environment.admit_instance_self(&scope, start).unwrap_err();
        assert!(error.contains("busy"));
        assert_eq!(
            environment.instance_instructions(&scope).unwrap().instance,
            "review"
        );
        drop(lock);
    }
    let (instance, lock) = environment.admit_instance_conclusion(&scope).unwrap();
    assert_eq!(instance.name, "review");
    assert!(gateway::lock(&config, "instance-review").is_err());
    assert!(
        environment
            .admit_instance_conclusion(&scope)
            .unwrap_err()
            .contains("busy")
    );
    for start in [false, true] {
        assert!(
            environment
                .admit_instance_self(&scope, start)
                .unwrap_err()
                .contains("busy")
        );
    }
    let guidance = "# Team guidance\nPreserve deliverables.\n";
    fs::write(config.instance_instructions(), guidance).unwrap();
    let instructions = environment.instance_instructions(&scope).unwrap();
    assert_eq!(instructions.instance, "review");
    assert_eq!(instructions.template, "blank");
    assert_eq!(instructions.namespace, config.namespace);
    assert_eq!(
        instructions.workspace,
        config.workspaces.join("review").display().to_string()
    );
    assert_eq!(
        instructions.file,
        config.instance_instructions().display().to_string()
    );
    assert_eq!(instructions.markdown, guidance);
    assert_eq!(
        instructions.status.instance,
        crate::store::environments::Status::WorkspaceReady
    );
    assert!(instructions.status.services.is_empty());
    assert!(instructions.status.topology_known);
    assert!(instructions.status.observed_at_unix_seconds.is_some());
    assert!(!instructions.status.stale);
    assert!(instructions.status.error.is_none());
    assert_eq!(
        instructions.core_guidance,
        include_str!("../../../instance-core-guidance.md")
    );
    assert!(
        environment
            .update_instance_repositories(&scope)
            .unwrap_err()
            .contains("busy")
    );
    drop(lock);
    let updated = environment.update_instance_repositories(&scope).unwrap();
    assert_eq!(updated.instance, "review");
    assert!(updated.repositories.is_empty());
    assert!(environment.bind_instance_directory(&config.home).is_err());
    assert!(
        environment
            .bind_instance_directory(&config.workspaces)
            .is_err()
    );
    fs::create_dir(config.workspaces.join("unowned")).unwrap();
    let unowned = config.workspaces.join("unowned");
    assert!(environment.bind_instance_directory(&unowned).is_err());
    let mut wrong_namespace = config.clone();
    wrong_namespace.namespace = "another".into();
    let wrong_environment = Environments::new(wrong_namespace.clone());
    let error = wrong_environment
        .admit_instance_self(&scope, false)
        .unwrap_err();
    assert!(error.contains("namespace"));
    assert_guidance_scope_error(&wrong_environment, &scope, &wrong_namespace, "namespace");
}

#[test]
fn instance_control_rejects_deleted_recreated_and_symlinked_workspaces() {
    let (_directory, config) = fixture();
    templates::create(&config, "blank").unwrap();
    owned_workspace(&config, "review");
    let environment = Environments::new(config.clone());
    let path = config.workspaces.join("review");
    let scope = environment.bind_instance_directory(&path).unwrap();
    runtime_db::remove(&config, "review", &[runtime_db::Kind::Ownership]).unwrap();
    assert_guidance_scope_error(&environment, &scope, &config, "ownership record missing");
    let template = templates::get(&config, "blank").unwrap();
    ownership::record(&config, "blank", Path::new(&template.directory), "review").unwrap();
    fs::remove_dir_all(&path).unwrap();
    assert_guidance_scope_error(&environment, &scope, &config, "No such file or directory");
    journal::forget(&config, "review").unwrap();
    assert_guidance_scope_error(&environment, &scope, &config, "ownership is missing");
    assert!(environment.admit_instance_self(&scope, false).is_err());
    assert!(environment.admit_instance_conclusion(&scope).is_err());
    assert!(environment.update_instance_repositories(&scope).is_err());
    owned_workspace(&config, "review");
    for start in [false, true] {
        let error = environment.admit_instance_self(&scope, start).unwrap_err();
        assert!(error.contains("replaced"));
    }
    let error = environment.admit_instance_conclusion(&scope).unwrap_err();
    assert!(error.contains("replaced"));
    assert!(
        environment
            .update_instance_repositories(&scope)
            .unwrap_err()
            .contains("replaced")
    );
    assert_guidance_scope_error(&environment, &scope, &config, "replaced");
    let fresh = environment.bind_instance_directory(&path).unwrap();
    assert!(environment.admit_instance_self(&fresh, false).is_ok());
    fs::remove_dir_all(&path).unwrap();
    let outside = tempfile::tempdir().unwrap();
    std::os::unix::fs::symlink(outside.path(), &path).unwrap();
    assert!(environment.bind_instance_directory(&path).is_err());
    assert!(environment.admit_instance_self(&fresh, false).is_err());
    assert!(environment.admit_instance_conclusion(&fresh).is_err());
    assert!(environment.update_instance_repositories(&fresh).is_err());
    assert_guidance_scope_error(&environment, &fresh, &config, "must be a real directory");
}

#[test]
fn instance_guidance_rejects_workspace_replacement_during_the_read() {
    for locked in [false, true] {
        let (_directory, config) = fixture();
        templates::create(&config, "blank").unwrap();
        owned_workspace(&config, "review");
        let environment = Environments::new(config.clone());
        let path = config.workspaces.join("review");
        let scope = environment.bind_instance_directory(&path).unwrap();
        let file = config.instance_instructions();
        fs::remove_file(&file).unwrap();
        assert!(
            std::process::Command::new("mkfifo")
                .arg(&file)
                .status()
                .unwrap()
                .success()
        );
        let _lock = locked.then(|| gateway::lock(&config, "instance-review").unwrap());
        let (finished, result) = mpsc::channel();
        let reader = std::thread::spawn(move || {
            finished
                .send(environment.instance_instructions(&scope))
                .unwrap();
        });
        let deadline = Instant::now() + Duration::from_secs(5);
        let opened = loop {
            match OpenOptions::new()
                .write(true)
                .custom_flags(libc::O_NONBLOCK)
                .open(&file)
            {
                Err(error)
                    if error.raw_os_error() == Some(libc::ENXIO)
                        && !reader.is_finished()
                        && Instant::now() < deadline =>
                {
                    std::thread::yield_now();
                }
                outcome => break outcome,
            }
        };
        let rendezvous = opened.is_ok();
        // An open writer proves the reader passed its first scope check.
        if rendezvous {
            fs::remove_dir_all(&path).unwrap();
            journal::forget(&config, "review").unwrap();
            owned_workspace(&config, "review");
        }
        // On timeout, a read/write descriptor releases a reader blocked opening the FIFO.
        let mut writer = opened.unwrap_or_else(|_| {
            OpenOptions::new()
                .read(true)
                .write(true)
                .custom_flags(libc::O_NONBLOCK)
                .open(&file)
                .unwrap()
        });
        let written = writer.write_all(b"# Team guidance\n");
        drop(writer);
        let outcome = result.recv_timeout(Duration::from_secs(5));
        if outcome.is_ok() {
            reader.join().unwrap();
        }
        assert!(
            rendezvous,
            "guidance reader did not open the FIFO before the deadline"
        );
        written.unwrap();
        let error = outcome.unwrap().unwrap_err();
        assert!(error.contains("replaced"), "{error}");
    }
}

#[test]
fn workspace_mcp_configuration_pins_environment_and_preserves_user_configuration() {
    let (_directory, config) = fixture();
    let workspace = config.workspaces.join("review");
    fs::create_dir(&workspace).unwrap();
    let root_config = workspace.join("opencode.json");
    fs::write(&root_config, "{\"model\":\"custom/model\"}").unwrap();
    instance_mcp::prepare_config(&config, &workspace).unwrap();
    assert_eq!(
        fs::read_to_string(&root_config).unwrap(),
        "{\"model\":\"custom/model\"}"
    );
    let path = workspace.join(".opencode/opencode.json");
    assert!(!path.exists());
    fs::remove_file(root_config).unwrap();
    instance_mcp::prepare_config(&config, &workspace).unwrap();
    let value: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    let server = &value["mcp"]["tandem-instance"];
    assert_eq!(server["command"][1], "mcp-instance");
    assert_eq!(server["cwd"], ".");
    assert_eq!(
        server["environment"]["TANDEM_HOME"],
        config.home.display().to_string()
    );
    assert_eq!(server["environment"]["TANDEM_NAMESPACE"], config.namespace);
    assert_eq!(server["timeout"], 660_000);
    assert_eq!(value["permission"]["tandem-instance_start_self"], "allow");
    assert_eq!(
        value["permission"]["tandem-instance_get_instructions"],
        "allow"
    );
    assert_eq!(value["permission"]["tandem-instance_stop_self"], "allow");
    assert_eq!(
        value["permission"]["tandem-instance_update_repositories"],
        "allow"
    );
    assert_eq!(value["permission"]["tandem-instance_conclude"], "allow");
    assert_eq!(
        value["permission"]["tandem-instance_search_events"],
        "allow"
    );
    assert_eq!(
        value["permission"]["tandem-instance_get_event_report"],
        "allow"
    );
    let custom = "// custom settings\n{\"model\":\"custom/model\"}\n";
    fs::remove_file(&path).unwrap();
    fs::write(workspace.join(".opencode/opencode.jsonc"), custom).unwrap();
    instance_mcp::prepare_config(&config, &workspace).unwrap();
    assert!(!path.exists());
    assert_eq!(
        fs::read_to_string(workspace.join(".opencode/opencode.jsonc")).unwrap(),
        custom
    );
    fs::remove_file(workspace.join(".opencode/opencode.jsonc")).unwrap();
    fs::write(&path, "{\"custom\":true}").unwrap();
    instance_mcp::prepare_config(&config, &workspace).unwrap();
    assert_eq!(fs::read_to_string(&path).unwrap(), "{\"custom\":true}");
}
