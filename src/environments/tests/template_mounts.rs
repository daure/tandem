use std::{fs, path::Path};

use serde_json::{Value, json};

use super::{Config, GUIDANCE, validate, validate_build_caches};

fn fixture() -> (tempfile::TempDir, Config) {
    let directory = tempfile::tempdir().unwrap();
    let config = Config::at(directory.path().join("home"), "mount-test".into(), 9876).unwrap();
    (directory, config)
}

fn bind(source: &Path, read_only: bool) -> Value {
    json!({
        "type": "bind",
        "source": source,
        "target": "/assets",
        "read_only": read_only,
        "bind": {"create_host_path": true}
    })
}

fn model(mounts: Vec<Value>) -> Value {
    json!({"services": {"web": {"image": "test", "volumes": mounts}}})
}

fn rejection(config: &Config, mount: Value) -> String {
    let source = mount["source"].as_str().map(str::to_owned);
    let model = model(vec![mount]);
    let before = model.clone();
    let error = validate(config, &model).unwrap_err();
    assert!(error.starts_with("web: bind source "), "{error}");
    assert!(error.contains(GUIDANCE), "{error}");
    if let Some(source) = source {
        assert!(error.contains(&format!("{source:?}")), "{error}");
    }
    assert_eq!(model, before);
    error
}

#[test]
fn writable_roots_descendants_and_ancestors_expose_shared_templates() {
    let (_directory, config) = fixture();
    let assets = config.templates.join("team/static");
    fs::create_dir_all(&assets).unwrap();
    let file = assets.join("index.html");
    fs::write(&file, "asset").unwrap();
    for source in [
        config.templates.clone(),
        assets,
        file.clone(),
        config.home.clone(),
        config.home.parent().unwrap().to_path_buf(),
        Path::new("/").to_path_buf(),
    ] {
        let error = rejection(&config, bind(&source, false));
        assert!(error.contains("writable source"), "{error}");
        assert!(error.contains("exposes shared templates"), "{error}");
    }
    assert_eq!(fs::read_to_string(file).unwrap(), "asset");
}

#[test]
fn missing_protected_sources_are_rejected_even_read_only_with_creation_disabled() {
    let (_directory, config) = fixture();
    let source = config.templates.join("team/missing/nested");
    for read_only in [false, true] {
        for create_host_path in [None, Some(true), Some(false)] {
            let mut mount = bind(&source, read_only);
            if let Some(create_host_path) = create_host_path {
                mount["bind"]["create_host_path"] = json!(create_host_path);
            } else {
                mount.as_object_mut().unwrap().remove("bind");
            }
            let error = rejection(&config, mount);
            assert!(error.contains("missing source"), "{error}");
            assert!(!config.templates.join("team").exists());
        }
    }
}

#[test]
fn existing_template_assets_and_ancestors_allow_read_only_binds() {
    let (_directory, config) = fixture();
    let assets = config.templates.join("team");
    fs::create_dir(&assets).unwrap();
    let file = assets.join("config.json");
    fs::write(&file, "{}").unwrap();
    let model = model(
        [config.templates.clone(), config.home.clone(), assets, file]
            .iter()
            .map(|source| bind(source, true))
            .collect(),
    );
    let before = model.clone();
    validate(&config, &model).unwrap();
    assert_eq!(model, before);
}

#[test]
fn workspace_unrelated_and_similarly_named_paths_allow_writable_binds() {
    let (directory, config) = fixture();
    let workspace = config.workspaces.join("instance");
    let unrelated = directory.path().join("unrelated");
    let sibling = config.home.join("templates-other");
    fs::create_dir(&workspace).unwrap();
    fs::create_dir(&unrelated).unwrap();
    fs::create_dir(&sibling).unwrap();
    let missing = workspace.join("new/nested");
    validate(
        &config,
        &model(
            [&workspace, &unrelated, &sibling, &missing]
                .into_iter()
                .map(|source| bind(source, false))
                .collect(),
        ),
    )
    .unwrap();
    assert!(!workspace.join("new").exists());
}

#[test]
fn named_volumes_tmpfs_and_services_without_mounts_are_allowed() {
    let (_directory, config) = fixture();
    validate(
        &config,
        &json!({"services": {
            "web": {"volumes": [
                {"type": "volume", "source": "templates", "target": "/data"},
                {"type": "tmpfs", "target": "/tmp"}
            ]},
            "worker": {},
            "empty": {"volumes": []}
        }}),
    )
    .unwrap();
}

#[test]
fn bind_backed_volume_drivers_protect_templates_and_allow_workspace_caches() {
    let (_directory, config) = fixture();
    let mut model = json!({"services":{"web":{}},"volumes":{"cache":{"driver":"local","driver_opts":{"type":"none","o":"bind","device":config.templates}}}});
    assert!(
        validate(&config, &model)
            .unwrap_err()
            .contains("writable source")
    );
    model["volumes"]["cache"]["driver_opts"]["o"] = json!("bind,ro");
    validate(&config, &model).unwrap();
    model["volumes"]["cache"]["driver_opts"]["o"] = json!("bind");
    model["volumes"]["cache"]["driver_opts"]["device"] = json!(config.workspaces.join("cache"));
    validate(&config, &model).unwrap();
}

#[test]
fn local_build_cache_exports_use_workspace_or_external_destinations() {
    let (_directory, config) = fixture();
    let directory = config.templates.join("website");
    fs::create_dir(&directory).unwrap();
    for destination in [".cache".to_owned(), directory.display().to_string()] {
        let model = json!({"services":{"web":{"build":{"context":directory,"cache_to":[format!("type=local,dest={destination}")]}}}});
        assert!(validate_build_caches(&config, &directory, &model).is_err());
    }
    let model = json!({"services":{"web":{"build":{"context":directory,"cache_to":[format!("type=local,dest={}",config.workspaces.join("review/cache").display()),"type=registry,ref=cache"]}}}});
    validate_build_caches(&config, &directory, &model).unwrap();
    assert!(!directory.join(".cache").exists());
}

#[test]
fn omitted_read_only_defaults_to_writable() {
    let (_directory, config) = fixture();
    let mut mount = bind(&config.templates, false);
    mount.as_object_mut().unwrap().remove("read_only");
    assert!(rejection(&config, mount).contains("writable source"));
}

#[test]
fn relative_and_malformed_bind_paths_fail_closed() {
    let (_directory, config) = fixture();
    for source in ["", "./static", "../templates", "/invalid\0path"] {
        rejection(&config, bind(Path::new(source), true));
    }
    for (field, value) in [
        ("source", Value::Null),
        ("source", json!(123)),
        ("read_only", json!("true")),
        ("bind", json!(false)),
        ("bind", json!({"create_host_path": "false"})),
    ] {
        let mut mount = bind(&config.templates, true);
        mount[field] = value;
        rejection(&config, mount);
    }
    for volumes in [json!("./static:/assets"), json!(["./static:/assets"])] {
        let error =
            validate(&config, &json!({"services": {"web": {"volumes": volumes}}})).unwrap_err();
        assert!(error.contains("expected resolved"), "{error}");
        assert!(error.contains(GUIDANCE), "{error}");
    }
}

#[test]
fn dotdot_uses_existing_parents_and_rejects_unresolvable_traversals() {
    let (_directory, config) = fixture();
    fs::create_dir(config.templates.join("team")).unwrap();
    let protected = config
        .workspaces
        .join("../templates/team/../missing/nested");
    assert!(rejection(&config, bind(&protected, true)).contains("missing source"));
    let existing = config.templates.join("team/..");
    assert!(rejection(&config, bind(&existing, false)).contains("writable source"));
    validate(
        &config,
        &model(vec![bind(&config.templates.join("../workspaces"), false)]),
    )
    .unwrap();
    let missing_parent = config.workspaces.join("absent/../other");
    assert!(rejection(&config, bind(&missing_parent, false)).contains("missing directory"));
    assert!(!config.workspaces.join("absent").exists());
    let file = config.workspaces.join("file");
    fs::write(&file, "keep").unwrap();
    assert!(rejection(&config, bind(&file.join("../other"), false)).contains("not a directory"));
}

#[cfg(unix)]
#[test]
fn aliases_to_templates_descendants_ancestors_and_missing_children_are_protected() {
    use std::os::unix::fs::symlink;

    let (directory, config) = fixture();
    let assets = config.templates.join("team");
    fs::create_dir(&assets).unwrap();
    for (name, target) in [
        ("root-alias", &config.templates),
        ("child-alias", &assets),
        ("ancestor-alias", &config.home),
    ] {
        let alias = directory.path().join(name);
        symlink(target, &alias).unwrap();
        assert!(rejection(&config, bind(&alias, false)).contains("writable source"));
        validate(&config, &model(vec![bind(&alias, true)])).unwrap();
    }
    let missing = directory.path().join("root-alias/missing/nested");
    assert!(rejection(&config, bind(&missing, true)).contains("missing source"));
    assert!(!config.templates.join("missing").exists());
}

#[cfg(unix)]
#[test]
fn dotdot_after_symlinks_follows_the_target_parent() {
    use std::os::unix::fs::symlink;

    let (directory, config) = fixture();
    let assets = config.templates.join("team");
    fs::create_dir(&assets).unwrap();
    let alias = directory.path().join("alias");
    symlink(&assets, &alias).unwrap();
    assert!(rejection(&config, bind(&alias.join(".."), false)).contains("writable source"));
    assert!(rejection(&config, bind(&alias.join("../missing"), true)).contains("missing source"));
    let outside = directory.path().join("outside/child");
    fs::create_dir_all(&outside).unwrap();
    let outbound = config.templates.join("outbound");
    symlink(&outside, &outbound).unwrap();
    validate(&config, &model(vec![bind(&outbound.join(".."), false)])).unwrap();
}

#[cfg(unix)]
#[test]
fn dangling_and_looping_symlinks_fail_closed() {
    use std::os::unix::fs::symlink;

    let (directory, config) = fixture();
    let dangling = directory.path().join("dangling");
    symlink(config.templates.join("missing"), &dangling).unwrap();
    assert!(rejection(&config, bind(&dangling, true)).contains("cannot resolve"));
    let looping = directory.path().join("loop");
    symlink(&looping, &looping).unwrap();
    assert!(rejection(&config, bind(&looping, true)).contains("cannot resolve"));
    assert!(!config.templates.join("missing").exists());
}

#[cfg(unix)]
#[test]
fn inaccessible_paths_fail_closed_when_permission_checks_apply() {
    use std::os::unix::fs::PermissionsExt;

    if unsafe { libc::geteuid() } == 0 {
        return;
    }
    let (directory, config) = fixture();
    let inaccessible = directory.path().join("inaccessible");
    fs::create_dir(&inaccessible).unwrap();
    fs::set_permissions(&inaccessible, fs::Permissions::from_mode(0o000)).unwrap();
    let result = validate(
        &config,
        &model(vec![bind(&inaccessible.join("child"), true)]),
    );
    fs::set_permissions(&inaccessible, fs::Permissions::from_mode(0o700)).unwrap();
    let error = result.unwrap_err();
    assert!(error.contains("web: bind source"), "{error}");
    assert!(error.contains("cannot inspect"), "{error}");
    assert!(error.contains(GUIDANCE), "{error}");
}
