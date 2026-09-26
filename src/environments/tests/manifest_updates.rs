use super::*;
use crate::store::environments::Manifest;

#[test]
fn manifest_updates_validate_before_replacing_the_file_and_repair_invalid_json() {
    let (_home, config) = fixture();
    let template = compose_template(&config, "website");
    let original = fs::read(&template.manifest_file).unwrap();
    for invalid in [
        json!({"routes":{"web":{"port":0,"readiness_path":"","readiness_contains":"ready"}}}),
        json!({"repositories":[{"source":"/source","target":"../outside"}]}),
        json!({"repositories":[{"source":"/source","target":"app"},{"source":"/other","target":"app/sub"}]}),
        json!({"repositories":[{"source":"/source","target":"app"}],"one_shots":["repo-sync"]}),
        json!({"routes":{"web":{"port":80,"readiness_path":"","readiness_contains":"ready"}},"one_shots":["web"]}),
        json!({"description": "x".repeat(262_144)}),
    ] {
        let manifest: Manifest = serde_json::from_value(invalid).unwrap();
        assert!(templates::update_manifest(&config, "website", manifest).is_err());
        assert_eq!(fs::read(&template.manifest_file).unwrap(), original);
    }
    fs::write(&template.manifest_file, "broken JSON").unwrap();
    let manifest: Manifest = serde_json::from_value(json!({
        "description":"Repaired", "repositories":[{"source":"/not-yet-available","target":"app"}]
    }))
    .unwrap();
    let updated = templates::update_manifest(&config, "website", manifest.clone()).unwrap();
    assert_eq!(updated.manifest, manifest);
    assert_eq!(templates::get(&config, "website").unwrap(), updated);
    assert_eq!(fs::read_dir(&config.workspaces).unwrap().count(), 0);
    assert!(fs::read_dir(&template.directory).unwrap().all(|entry| {
        !entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with(".tandem-manifest-")
    }));
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            fs::metadata(&template.manifest_file)
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
    }
    fs::remove_file(&template.manifest_file).unwrap();
    assert!(
        templates::update_manifest(&config, "website", manifest)
            .unwrap()
            .manifest_source
            .is_some()
    );
}

#[test]
fn manifest_updates_share_the_template_lock_and_reject_path_escape() {
    let (_home, config) = fixture();
    let template = templates::create(&config, "website").unwrap();
    let original = fs::read(&template.manifest_file).unwrap();
    let lock = gateway::lock(&config, "template-website").unwrap();
    assert!(templates::update_manifest(&config, "website", Manifest::default()).is_err());
    assert_eq!(fs::read(&template.manifest_file).unwrap(), original);
    drop(lock);
    for name in ["../website", "missing"] {
        assert!(templates::update_manifest(&config, name, Manifest::default()).is_err());
    }
    #[cfg(unix)]
    {
        let outside = tempfile::tempdir().unwrap();
        let path = outside.path().join("manifest.json");
        fs::write(&path, &original).unwrap();
        fs::remove_file(&template.manifest_file).unwrap();
        std::os::unix::fs::symlink(&path, &template.manifest_file).unwrap();
        assert!(templates::update_manifest(&config, "website", Manifest::default()).is_err());
        assert_eq!(fs::read(&path).unwrap(), original);
        std::os::unix::fs::symlink(&template.directory, config.templates.join("alias")).unwrap();
        assert!(templates::update_manifest(&config, "alias", Manifest::default()).is_err());
    }
}
