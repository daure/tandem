use super::*;

#[test]
fn template_guidance_is_optional_and_preserves_markdown_for_inspection() {
    let (_directory, config) = fixture();
    let environment = Environments::new(config);
    let template = environment.create_template("website").unwrap();
    assert!(template.guidance_source.is_none());
    assert_eq!(
        std::path::Path::new(&template.guidance_file),
        std::path::Path::new(&template.directory).join("tandem-agents.md")
    );
    for source in ["", "# Development\n\nUse `{{literal}}` for fixtures.\n"] {
        fs::write(&template.guidance_file, source).unwrap();
        let loaded = environment.get_template("website").unwrap();
        assert_eq!(loaded.guidance_source.as_deref(), Some(source));
        assert_eq!(
            environment.list_templates().unwrap()[0]
                .guidance_source
                .as_deref(),
            Some(source)
        );
        let serialized = serde_json::to_value(&loaded).unwrap();
        assert_eq!(serialized["guidance_source"], source);
        assert_eq!(serialized["guidance_file"], template.guidance_file);
    }
    fs::remove_file(&template.guidance_file).unwrap();
    assert!(
        environment
            .get_template("website")
            .unwrap()
            .guidance_source
            .is_none()
    );
}

#[test]
fn invalid_template_guidance_is_reported_without_hiding_the_template() {
    let (_directory, config) = fixture();
    let template = templates::create(&config, "website").unwrap();
    for source in [vec![0xff], vec![b'a'; 262_145]] {
        fs::write(&template.guidance_file, source).unwrap();
        assert!(
            templates::get(&config, "website")
                .unwrap_err()
                .contains("tandem-agents.md")
        );
        assert!(templates::list(&config).unwrap()[0].error.is_some());
    }
    fs::remove_file(&template.guidance_file).unwrap();
    fs::create_dir(&template.guidance_file).unwrap();
    assert!(
        templates::get(&config, "website")
            .unwrap_err()
            .contains("regular file")
    );
}

#[cfg(unix)]
#[test]
fn template_guidance_must_resolve_within_the_template_directory() {
    let (directory, config) = fixture();
    let template = templates::create(&config, "website").unwrap();
    let outside = directory.path().join("outside.md");
    fs::write(&outside, "Outside guidance").unwrap();
    std::os::unix::fs::symlink(&outside, &template.guidance_file).unwrap();
    assert_eq!(
        templates::get(&config, "website").unwrap_err(),
        "tandem-agents.md escapes template directory"
    );
}
