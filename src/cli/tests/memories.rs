use super::*;

#[test]
fn checkpoints_survive_independent_purge_for_accepted_and_manual_instances() {
    let fixture = fixture();
    fs::write(fixture.home.join("release-close"), "").unwrap();
    let mut management = Client::with_command(fixture.command(&["mcp"]));
    for name in ["review", "reader"] {
        let mut agent = client(&fixture, name);
        let checkpoint = agent.tool(
            "remember",
            json!({
                "title":"Retry diagnosis", "summary":"Pool exhaustion verified",
                "fields":{"hypothesis":null, "verified":true, "findings":["Retry amplification"]},
                "labels":["incident"], "links":[{"label":"PR", "url":"https://example.com/pull/42"}]
            }),
        );
        assert_eq!(checkpoint["instance"], name);
        assert_eq!(
            checkpoint["acceptance_id"],
            if name == "review" {
                json!(1)
            } else {
                json!(null)
            }
        );
        let workspace = fixture
            .home
            .join("workspaces")
            .join(if name == "reader" { "blank" } else { "website" })
            .join(name);
        assert!(workspace.is_dir());
        assert!(
            management.tool("list_rules", json!({}))["reports"]
                .as_object()
                .unwrap()
                .is_empty()
        );
        let receipt = agent.tool("purge", json!({}));
        assert!(receipt["memory_id"].is_null());
        agent.close(Some(libc::SIGKILL));
        wait_for(|| {
            management.tool(
                "get_conclusion",
                json!({"conclusion_id":receipt["conclusion_id"]}),
            )["cleanup_state"]
                == "purged"
        });
        assert!(!workspace.exists());
        let memory = management.tool("get_memory", json!({"memory_id":checkpoint["memory_id"]}));
        assert_eq!(memory["fields"]["findings"], json!(["Retry amplification"]));
        let matches = management.tool(
            "search_memories",
            json!({"query":"retr diagnos", "instance":name, "fields":{"verified":true}}),
        );
        assert_eq!(matches["memories"].as_array().unwrap().len(), 1);
        assert_eq!(matches["memories"][0]["memory_id"], checkpoint["memory_id"]);
    }
}

#[test]
fn purge_retries_preserve_one_structured_checkpoint() {
    let fixture = fixture();
    fs::write(fixture.home.join("release-close"), "").unwrap();
    fs::write(fixture.home.join("fail-close"), "").unwrap();
    let mut agent = client(&fixture, "review");
    let mut reader = client(&fixture, "reader");
    let report = json!({"title":"Pool fix", "summary":"Bounded retries", "fields":{"verified":true}, "labels":["backend"]});
    let checkpoint = agent.tool("remember", report.clone());
    let receipt = agent.tool("purge", json!({}));
    wait_for(|| {
        reader.tool(
            "get_conclusion",
            json!({"conclusion_id":receipt["conclusion_id"]}),
        )["cleanup_state"]
            == "failed"
    });
    let memory_id = checkpoint["memory_id"].clone();
    assert!(memory_id.as_i64().unwrap() > 0);
    assert_eq!(
        reader.tool("get_memory", json!({"memory_id":memory_id}))["fields"],
        report["fields"]
    );
    fs::remove_file(fixture.home.join("fail-close")).unwrap();
    let retry = agent.tool("purge", json!({}));
    assert_eq!(retry["conclusion_id"], receipt["conclusion_id"]);
    assert!(retry["memory_id"].is_null());
    agent.close(Some(libc::SIGKILL));
    wait_for(|| {
        reader.tool(
            "get_conclusion",
            json!({"conclusion_id":receipt["conclusion_id"]}),
        )["cleanup_state"]
            == "purged"
    });
    let results = reader.tool(
        "search_memories",
        json!({"query":"pool", "labels":["backend"]}),
    );
    assert_eq!(results["memories"].as_array().unwrap().len(), 1);
    assert_eq!(results["memories"][0]["memory_id"], memory_id);
}
