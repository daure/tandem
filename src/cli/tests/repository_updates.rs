use super::startup::Client;
use super::*;

#[test]
fn instance_repository_updates_return_partial_results_and_stay_within_the_bound_workspace() {
    let fixture = Fixture::new();
    fs::remove_file(fixture.home.join("templates/website/compose.yaml")).unwrap();
    fs::write(
        fixture.home.join("templates/website/tandem.json"),
        json!({"repositories": [
            {"source": fixture.source, "target": "app"},
            {"source": fixture.source, "target": "lib"}
        ]})
        .to_string(),
    )
    .unwrap();
    for name in ["review", "other"] {
        let output = fixture.run(&["new-instance", name, "-t", "website"]);
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    let workspace = fixture.home.join("workspaces/review");
    let app = workspace.join("app");
    for target in ["app", "lib"] {
        git(&workspace.join(target), &["switch", "trunk"]);
    }
    let before = git(&app, &["rev-parse", "HEAD"]);
    fs::write(workspace.join("lib/file.txt"), "local edits").unwrap();
    git(
        &workspace,
        &[
            "clone",
            "--",
            fixture.source.to_str().unwrap(),
            "undeclared",
        ],
    );
    fs::write(fixture.source.join("file.txt"), "latest upstream").unwrap();
    git(&fixture.source, &["add", "."]);
    git(&fixture.source, &["commit", "-m", "Upstream update"]);
    let latest = git(&fixture.source, &["rev-parse", "HEAD"]);
    let mut command = fixture.command(&["mcp-instance"]);
    command.current_dir(&app);
    let mut client = Client::with_command(command);
    client.tool("get_instructions", json!({}));
    let injection = client.request_raw(
        "tools/call",
        json!({"name": "update_repositories", "arguments": {"name": "other"}}),
    );
    assert_eq!(injection["error"]["code"], -32602);
    let results = client.tool("update_repositories", json!({}));
    assert_eq!(results["instance"], "review");
    let repositories = results["repositories"].as_array().unwrap();
    assert_eq!(repositories.len(), 2);
    assert_eq!(repositories[0]["target"], "app");
    assert_eq!(repositories[0]["status"], "updated");
    assert_eq!(repositories[0]["before"], before);
    assert_eq!(repositories[0]["after"], latest);
    assert_eq!(repositories[0]["upstream"], "refs/remotes/origin/trunk");
    assert_eq!(repositories[1]["target"], "lib");
    assert_eq!(repositories[1]["status"], "skipped");
    assert_eq!(repositories[1]["before"], before);
    assert_eq!(repositories[1]["after"], before);
    assert_eq!(
        fs::read_to_string(workspace.join("lib/file.txt")).unwrap(),
        "local edits"
    );
    assert_eq!(
        git(
            &fixture.home.join("workspaces/other/app"),
            &["rev-parse", "HEAD"]
        ),
        before
    );
    assert_eq!(
        git(&workspace.join("undeclared"), &["rev-parse", "HEAD"]),
        before
    );
    assert_eq!(
        client.tool("update_repositories", json!({}))["repositories"][0]["status"],
        "current"
    );
}
