use super::*;

fn parse(arguments: &[&str]) -> Result<Cli, clap::Error> {
    Cli::try_parse_from(normalize_arguments(arguments.iter().map(OsString::from)))
}

#[test]
fn new_instance_accepts_long_short_and_mixed_options() {
    for options in [
        vec!["review", "--template", "website"],
        vec!["review", "-t", "website", "--open-command"],
        vec!["-oc", "--template", "website", "review"],
        vec!["review", "-twebsite", "-oc"],
        vec!["--template=website", "-oc", "review"],
    ] {
        let mut arguments = vec!["tandem", "new-instance"];
        arguments.extend(&options);
        let Some(Commands::NewInstance {
            name,
            template,
            open_command,
        }) = parse(&arguments).unwrap().command
        else {
            panic!("expected new-instance");
        };
        assert_eq!(name, "review");
        assert_eq!(template, "website");
        assert_eq!(
            open_command,
            options.contains(&"-oc") || options.contains(&"--open-command")
        );
    }
}

#[test]
fn new_instance_requires_name_template_and_boolean_open_flag() {
    for arguments in [
        vec!["tandem", "new-instance", "review"],
        vec!["tandem", "new-instance", "-t", "website"],
        vec!["tandem", "new-instance", "review", "-t"],
        vec![
            "tandem",
            "new-instance",
            "review",
            "-t",
            "website",
            "--open-command=echo",
        ],
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
fn open_alias_preserves_values_and_other_commands() {
    for arguments in [
        vec!["tandem", "new-instance", "review", "-t", "-oc"],
        vec!["tandem", "new-instance", "--template=-oc", "review"],
        vec!["tandem", "new-instance", "-t", "website", "--", "-oc"],
        vec!["tandem", "serve", "-oc"],
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
