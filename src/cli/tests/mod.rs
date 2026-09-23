use super::*;

fn parse(arguments: &[&str]) -> Result<Cli, clap::Error> {
    Cli::try_parse_from(normalize_arguments(arguments.iter().map(OsString::from)))
}

#[test]
fn new_instance_accepts_long_short_and_mixed_options() {
    for (options, expected_open_command, expected_description) in [
        (vec!["review", "--template", "website"], None, None),
        (
            vec!["review", "-t", "website", "--open-command"],
            Some(None),
            None,
        ),
        (vec!["review", "-twebsite", "-oc"], Some(None), None),
        (
            vec!["review", "-t", "website", "-oc", "extra value"],
            Some(Some("extra value")),
            None,
        ),
        (
            vec![
                "review",
                "-t",
                "website",
                "--open-command=extra value",
                "-d",
                "Review environment",
            ],
            Some(Some("extra value")),
            Some("Review environment"),
        ),
    ] {
        let mut arguments = vec!["tandem", "new-instance"];
        arguments.extend(&options);
        let Some(Commands::NewInstance {
            name,
            template,
            open_command,
            description,
        }) = parse(&arguments).unwrap().command
        else {
            panic!("expected new-instance");
        };
        assert_eq!(name, "review");
        assert_eq!(template, "website");
        assert_eq!(
            open_command.as_ref().map(|extra| extra.as_deref()),
            expected_open_command
        );
        assert_eq!(description.as_deref(), expected_description);
    }
}

#[test]
fn new_instance_requires_name_and_template() {
    for arguments in [
        vec!["tandem", "new-instance", "review"],
        vec!["tandem", "new-instance", "-t", "website"],
        vec!["tandem", "new-instance", "review", "-t"],
        vec!["tandem", "new-instance", "review", "-t", "website", "-o"],
        vec![
            "tandem",
            "new-instance",
            "review",
            "-t",
            "website",
            "--",
            "-oc",
        ],
    ] {
        assert!(parse(&arguments).is_err(), "{arguments:?}");
    }
}

#[test]
fn delete_instance_requires_a_name() {
    assert!(parse(&["tandem", "delete-instance"]).is_err());
    let Some(Commands::DeleteInstance { name, headless, .. }) =
        parse(&["tandem", "delete-instance", "review", "-h"])
            .unwrap()
            .command
    else {
        panic!("expected delete-instance");
    };
    assert_eq!(name, "review");
    assert!(headless);
}

#[test]
fn delete_instance_close_command_is_an_optional_boolean_flag() {
    for options in [
        vec!["review"],
        vec!["review", "--close-command"],
        vec!["-cc", "review"],
        vec!["review", "-h", "-cc"],
        vec!["--headless", "--close-command", "review"],
    ] {
        let mut arguments = vec!["tandem", "delete-instance"];
        arguments.extend(&options);
        let Some(Commands::DeleteInstance {
            name,
            headless,
            close_command,
        }) = parse(&arguments).unwrap().command
        else {
            panic!("expected delete-instance");
        };
        assert_eq!(name, "review");
        assert_eq!(
            headless,
            options.contains(&"-h") || options.contains(&"--headless")
        );
        assert_eq!(
            close_command,
            options.contains(&"-cc") || options.contains(&"--close-command")
        );
    }
    for arguments in [
        vec!["tandem", "delete-instance", "--close-command"],
        vec![
            "tandem",
            "delete-instance",
            "review",
            "--close-command=echo",
        ],
        vec!["tandem", "delete-instance", "review", "-c"],
        vec!["tandem", "delete-instance", "review", "-oc"],
    ] {
        assert!(parse(&arguments).is_err(), "{arguments:?}");
    }
}

#[test]
fn command_aliases_preserve_values_boundaries_and_other_commands() {
    for arguments in [
        vec!["tandem", "new-instance", "review", "-t", "-oc"],
        vec!["tandem", "new-instance", "--template=-oc", "review"],
        vec!["tandem", "new-instance", "-t", "website", "--", "-oc"],
        vec!["tandem", "serve", "-oc"],
        vec!["tandem", "new-instance", "review", "-cc"],
        vec!["tandem", "delete-instance", "--", "-cc"],
        vec!["tandem", "serve", "-cc"],
    ] {
        assert_eq!(
            normalize_arguments(arguments.iter().map(OsString::from)),
            arguments.iter().map(OsString::from).collect::<Vec<_>>()
        );
    }
    assert!(parse(&["tandem"]).unwrap().command.is_none());
    assert!(matches!(
        parse(&["tandem", "mcp"]).unwrap().command,
        Some(Commands::Mcp)
    ));
}
