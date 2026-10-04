use super::*;
use std::path::Path;

use crate::environments::{instance_mcp, ownership};

fn owned_workspace(config: &Config, name: &str) {
    let template = templates::get(config, "blank").unwrap();
    journal::prepare(config, &template, name, None).unwrap();
    ownership::record(config, "blank", Path::new(&template.directory), name).unwrap();
    fs::create_dir_all(config.workspaces.join(name).join("app/src")).unwrap();
    journal::workspace_ready(config, name).unwrap();
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
        drop(lock);
    }
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
    let error = Environments::new(wrong_namespace)
        .admit_instance_self(&scope, false)
        .unwrap_err();
    assert!(error.contains("namespace"));
}

#[test]
fn instance_control_rejects_deleted_recreated_and_symlinked_workspaces() {
    let (_directory, config) = fixture();
    templates::create(&config, "blank").unwrap();
    owned_workspace(&config, "review");
    let environment = Environments::new(config.clone());
    let path = config.workspaces.join("review");
    let scope = environment.bind_instance_directory(&path).unwrap();
    fs::remove_dir_all(&path).unwrap();
    journal::forget(&config, "review").unwrap();
    assert!(environment.admit_instance_self(&scope, false).is_err());
    owned_workspace(&config, "review");
    for start in [false, true] {
        let error = environment.admit_instance_self(&scope, start).unwrap_err();
        assert!(error.contains("replaced"));
    }
    let fresh = environment.bind_instance_directory(&path).unwrap();
    assert!(environment.admit_instance_self(&fresh, false).is_ok());
    fs::remove_dir_all(&path).unwrap();
    let outside = tempfile::tempdir().unwrap();
    std::os::unix::fs::symlink(outside.path(), &path).unwrap();
    assert!(environment.bind_instance_directory(&path).is_err());
    assert!(environment.admit_instance_self(&fresh, false).is_err());
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
    assert_eq!(value["permission"]["tandem-instance_stop_self"], "allow");
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
