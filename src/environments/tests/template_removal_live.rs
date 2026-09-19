use std::{
    collections::BTreeSet,
    fs,
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use crate::environments::{
    command::{docker, run},
    compose,
    config::Config,
    removal, templates,
};

fn command(args: &[&str]) -> Result<String, String> {
    let mut command = docker();
    command.args(args);
    run(command, Duration::from_secs(30), None)
}

#[derive(Default)]
struct Cleanup {
    containers: Vec<String>,
    networks: Vec<String>,
    volumes: BTreeSet<String>,
}

impl Drop for Cleanup {
    fn drop(&mut self) {
        for name in &self.containers {
            let _ = command(&["rm", "--force", "--volumes", name]);
        }
        for name in &self.networks {
            let _ = command(&["network", "rm", name]);
        }
        for name in &self.volumes {
            let _ = command(&["volume", "rm", name]);
        }
    }
}

#[test]
#[ignore = "creates and deletes isolated Docker fixtures; requires local alpine/git:2.49.1"]
fn live_template_deletion_removes_its_containers_volumes_network_and_workspace() {
    command(&["image", "inspect", "alpine/git:2.49.1"]).unwrap();
    let home = tempfile::tempdir().unwrap();
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let namespace = format!("td-delete-{}-{nonce:x}", std::process::id());
    let config = Config::at(home.path().to_owned(), namespace.clone(), 9876).unwrap();
    let template = templates::create(&config, "website").unwrap();
    let other = templates::create(&config, "other").unwrap();
    let workspace = config.workspaces.join("review");
    fs::create_dir(&workspace).unwrap();
    fs::write(workspace.join("data"), "delete this").unwrap();
    fs::create_dir(config.workspaces.join("unrelated")).unwrap();
    fs::write(config.workspaces.join("unrelated/data"), "keep").unwrap();
    let project = config.project("review");
    let project_label = format!("com.docker.compose.project={project}");
    let network = format!("{project}-network");
    let volume = format!("{project}-data");
    let unrelated_volume = format!("{namespace}-unrelated-data");
    let container = format!("{project}-web");
    let mut cleanup = Cleanup::default();
    command(&["network", "create", "--label", &project_label, &network]).unwrap();
    cleanup.networks.push(network.clone());
    command(&["volume", "create", "--label", &project_label, &volume]).unwrap();
    cleanup.volumes.insert(volume.clone());
    command(&["volume", "create", &unrelated_volume]).unwrap();
    cleanup.volumes.insert(unrelated_volume.clone());
    cleanup.containers.push(container.clone());
    command(&[
        "run",
        "--detach",
        "--pull=never",
        "--name",
        &container,
        "--entrypoint",
        "sleep",
        "--network",
        &network,
        "--label",
        &project_label,
        "--label",
        "com.docker.compose.service=web",
        "--label",
        &format!("{}={namespace}", compose::NAMESPACE),
        "--label",
        &format!("{}=instance", compose::KIND),
        "--label",
        &format!("{}=review", compose::INSTANCE),
        "--label",
        &format!("{}=website", compose::TEMPLATE),
        "--label",
        &format!("{}={}", compose::DIRECTORY, template.directory),
        "--label",
        &format!("{}={}", compose::WORKSPACE, workspace.display()),
        "--mount",
        &format!("type=bind,src={},dst=/workspace", workspace.display()),
        "--mount",
        &format!("type=volume,src={volume},dst=/data"),
        "--volume",
        "/anonymous",
        "alpine/git:2.49.1",
        "300",
    ])
    .unwrap();
    let mounts = command(&[
        "inspect",
        "--format",
        "{{range .Mounts}}{{if eq .Type \"volume\"}}{{println .Name}}{{end}}{{end}}",
        &container,
    ])
    .unwrap();
    let volumes: Vec<_> = mounts.split_whitespace().map(str::to_owned).collect();
    cleanup.volumes.extend(volumes.iter().cloned());
    assert!(volumes.contains(&volume));
    assert!(volumes.iter().any(|name| name != &volume));
    command(&["pause", &container]).unwrap();

    removal::template(
        &config,
        "website",
        60,
        Arc::new(|_| {}),
        &Default::default(),
    )
    .unwrap();

    assert!(command(&["inspect", &container]).is_err());
    assert!(command(&["network", "inspect", &network]).is_err());
    for volume in &volumes {
        assert!(command(&["volume", "inspect", volume]).is_err());
    }
    assert!(!workspace.exists());
    assert!(!std::path::Path::new(&template.directory).exists());
    assert!(std::path::Path::new(&other.compose_file).is_file());
    assert_eq!(
        fs::read_to_string(config.workspaces.join("unrelated/data")).unwrap(),
        "keep"
    );
    assert!(command(&["volume", "inspect", &unrelated_volume]).is_ok());
}
