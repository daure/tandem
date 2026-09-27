use super::*;

fn parse(arguments: &[&str]) -> Result<Cli, clap::Error> {
    Cli::try_parse_from(arguments)
}

#[test]
fn new_instance_accepts_long_short_and_mixed_options() {
    for (options, expected_opencode, expected_description) in [
        (vec!["review", "--template", "website"], None, None),
        (
            vec!["review", "-t", "website", "--opencode"],
            Some(None),
            None,
        ),
        (vec!["review", "-twebsite", "-o"], Some(None), None),
        (
            vec!["review", "-t", "website", "-o", "extra value"],
            Some(Some("extra value")),
            None,
        ),
        (
            vec![
                "review",
                "-t",
                "website",
                "--opencode=extra value",
                "-d",
                "Review environment",
            ],
            Some(Some("extra value")),
            Some("Review environment"),
        ),
        (
            vec!["review", "-t", "website", "--opencode="],
            Some(Some("")),
            None,
        ),
        (
            vec!["review", "-t", "website", "--opencode=--help"],
            Some(Some("--help")),
            None,
        ),
    ] {
        let mut arguments = vec!["tandem", "new-instance"];
        arguments.extend(&options);
        let Some(Commands::NewInstance {
            name,
            template,
            opencode,
            description,
        }) = parse(&arguments).unwrap().command
        else {
            panic!("expected new-instance");
        };
        assert_eq!(name, "review");
        assert_eq!(template, "website");
        assert_eq!(
            opencode.as_ref().map(|prompt| prompt.as_deref()),
            expected_opencode
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
        vec![
            "tandem",
            "new-instance",
            "review",
            "-t",
            "website",
            "--",
            "-o",
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
