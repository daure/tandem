use super::*;
use crate::environments::Startup;
use std::{
    os::unix::fs::{PermissionsExt, symlink},
    path::Path,
};

fn start(config: &Config, startup: Startup) -> Result<Instance, String> {
    lifecycle::start(
        config,
        "local",
        "review",
        startup,
        30,
        Arc::new(|_| {}),
        |_| {},
    )
}

#[test]
fn files_only_templates_copy_nested_hidden_binary_and_executable_files_and_preserve_edits() {
    let (_root, config) = fixture();
    let files = config.templates.join("local/tandem-files");
    fs::create_dir_all(files.join("nested/empty")).unwrap();
    fs::write(files.join(".settings"), "hidden").unwrap();
    fs::write(files.join("nested/binary.bin"), [0, 255, 128]).unwrap();
    fs::write(files.join("run.sh"), "#!/bin/sh\nexit 0\n").unwrap();
    fs::set_permissions(files.join("run.sh"), fs::Permissions::from_mode(0o751)).unwrap();
    let template = templates::get(&config, "local").unwrap();
    assert!(template.workspace_only());
    assert_eq!(
        templates::list(&config).unwrap(),
        std::slice::from_ref(&template)
    );
    assert_eq!(
        template
            .files
            .iter()
            .map(|file| file.path.as_str())
            .collect::<Vec<_>>(),
        [
            ".settings",
            "nested",
            "nested/binary.bin",
            "nested/empty",
            "run.sh"
        ]
    );

    let instance = start(&config, Startup::default()).unwrap();
    let workspace = Path::new(&instance.workspace);
    assert!(instance.runtime.workspace_ready);
    assert_eq!(
        fs::read_to_string(workspace.join(".settings")).unwrap(),
        "hidden"
    );
    assert_eq!(
        fs::read(workspace.join("nested/binary.bin")).unwrap(),
        [0, 255, 128]
    );
    assert_eq!(
        fs::metadata(workspace.join("run.sh"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o751
    );
    assert!(workspace.join("nested/empty").is_dir());
    assert!(workspace.join("AGENTS.md").is_file());
    assert!(!workspace.join("tandem-files").exists());

    fs::write(workspace.join(".settings"), "user edits").unwrap();
    fs::write(files.join(".settings"), "template edits").unwrap();
    fs::write(files.join("new.txt"), "new seed").unwrap();
    start(&config, Startup::default()).unwrap();
    assert_eq!(
        fs::read_to_string(workspace.join(".settings")).unwrap(),
        "user edits"
    );
    assert_eq!(
        fs::read_to_string(workspace.join("new.txt")).unwrap(),
        "new seed"
    );
}

#[test]
fn unsafe_seed_entries_are_reported_as_invalid_templates() {
    for entry in [
        "root-link",
        "file-link",
        "directory-link",
        "fifo",
        "AGENTS.md",
        ".tandem-owner",
        ".git",
    ] {
        let (_root, config) = fixture();
        let template = templates::create(&config, "local").unwrap();
        let files = Path::new(&template.directory).join("tandem-files");
        let outside = tempfile::tempdir().unwrap();
        fs::write(outside.path().join("source"), "untouched").unwrap();
        if entry == "root-link" {
            symlink(outside.path(), &files).unwrap();
        } else {
            fs::create_dir(&files).unwrap();
            match entry {
                "file-link" => symlink(outside.path().join("source"), files.join("link")).unwrap(),
                "directory-link" => symlink(outside.path(), files.join("link")).unwrap(),
                "fifo" => {
                    let path =
                        std::ffi::CString::new(files.join("pipe").as_os_str().as_encoded_bytes())
                            .unwrap();
                    assert_eq!(unsafe { libc::mkfifo(path.as_ptr(), 0o600) }, 0);
                }
                name => fs::write(files.join(name), "reserved").unwrap(),
            }
        }
        assert!(templates::get(&config, "local").is_err(), "{entry}");
        assert!(
            templates::list(&config).unwrap()[0].error.is_some(),
            "{entry}"
        );
        assert!(start(&config, Startup::default()).is_err(), "{entry}");
        assert!(!config.workspaces.join("review").exists(), "{entry}");
        assert_eq!(
            fs::read_to_string(outside.path().join("source")).unwrap(),
            "untouched"
        );
    }
}

#[test]
fn incompatible_seed_destinations_block_the_whole_copy() {
    for destination in [
        "symlink-parent",
        "symlink-file",
        "file-parent",
        "directory-file",
    ] {
        let (_root, config) = fixture();
        let template = templates::create(&config, "local").unwrap();
        let files = Path::new(&template.directory).join("tandem-files");
        fs::create_dir_all(files.join("nested")).unwrap();
        fs::write(files.join("first.txt"), "seed").unwrap();
        fs::write(files.join("nested/value.txt"), "seed").unwrap();
        let workspace = config.workspaces.join("review");
        let outside = tempfile::tempdir().unwrap();
        fs::write(outside.path().join("value.txt"), "untouched").unwrap();
        fs::create_dir(&workspace).unwrap();
        match destination {
            "symlink-parent" => symlink(outside.path(), workspace.join("nested")).unwrap(),
            "file-parent" => fs::write(workspace.join("nested"), "untouched").unwrap(),
            name => {
                fs::create_dir(workspace.join("nested")).unwrap();
                if name == "symlink-file" {
                    symlink(
                        outside.path().join("value.txt"),
                        workspace.join("nested/value.txt"),
                    )
                    .unwrap();
                } else {
                    fs::create_dir(workspace.join("nested/value.txt")).unwrap();
                }
            }
        }
        let error = start(&config, Startup::default()).unwrap_err();
        assert!(
            error.contains("tandem-files destination"),
            "{destination}: {error}"
        );
        assert!(!workspace.join("first.txt").exists());
        assert!(!workspace.join("AGENTS.md").exists());
        assert_eq!(
            fs::read_to_string(outside.path().join("value.txt")).unwrap(),
            "untouched"
        );
    }
}

#[test]
fn seed_paths_keep_repository_targets_clear_and_allow_shared_parent_directories() {
    for path in ["sources", "sources/app", "sources/app/config"] {
        let (_root, config) = fixture();
        let template = templates::create(&config, "local").unwrap();
        let files = Path::new(&template.directory).join("tandem-files");
        let file = files.join(path);
        fs::create_dir_all(file.parent().unwrap()).unwrap();
        fs::write(&file, "conflict").unwrap();
        let manifest = serde_json::from_value(json!({
            "repositories": [{"source": "/source", "target": "sources/app"}]
        }))
        .unwrap();
        let error = templates::update_manifest(&config, "local", manifest).unwrap_err();
        assert!(
            error.contains("conflicts with repository target sources/app"),
            "{error}"
        );
        assert_eq!(fs::read_to_string(&template.manifest_file).unwrap(), "{}\n");
        fs::remove_dir_all(&files).unwrap();
        fs::create_dir_all(files.join("sources/config")).unwrap();
        fs::write(files.join("sources/config/settings"), "valid").unwrap();
        templates::update_manifest(
            &config,
            "local",
            serde_json::from_value(json!({
                "repositories": [{"source": "/source", "target": "sources/app"}]
            }))
            .unwrap(),
        )
        .unwrap();
    }
}
