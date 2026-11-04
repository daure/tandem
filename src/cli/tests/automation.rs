use super::*;

#[test]
fn automation_commands_require_opt_in_while_workspace_commands_remain_available() {
    let fixture = Fixture::new();
    fs::remove_file(fixture.home.join("config.json")).unwrap();
    for arguments in [
        vec!["list-providers"],
        vec!["provider", "stop", "sample"],
        vec!["providers-setup"],
        vec!["rules-setup", "--model", "openai/test"],
    ] {
        let output = fixture.run(&arguments);
        assert!(!output.status.success(), "{arguments:?}");
        assert_eq!(
            String::from_utf8(output.stderr).unwrap().trim(),
            "Error: \"Automation is disabled; set ui.automation to true in TANDEM_HOME/config.json and restart Tandem\""
        );
        assert!(output.stdout.is_empty());
    }
    assert!(fixture.run(&["list-templates", "--json"]).status.success());
    fs::write(
        fixture.home.join("config.json"),
        r#"{"ui":{"automation":true}}"#,
    )
    .unwrap();
    let output = fixture.run(&["list-providers"]);
    assert!(output.status.success(), "{output:?}");
    let providers: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(providers["providers"], json!([]));
}
