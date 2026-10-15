use super::*;

#[test]
fn instance_closure_survives_disconnect_and_preserves_services_workspace_and_history() {
    for arguments in [
        json!({"sessions":"current"}),
        json!({"sessions":"all"}),
        json!({"zellij_tab":true}),
    ] {
        let fixture = fixture();
        let mut agent = client(&fixture, "review");
        let result = agent.request("tools/call", json!({"name":"close", "arguments": arguments, "_meta":{"ai.opencode/sessionID":"ses_incident"}}));
        assert_ne!(result["isError"], true, "{result}");
        assert_eq!(result["structuredContent"]["state"], "pending", "{result}");
        wait_for(|| fixture.home.join("closing").exists());
        assert!(
            fixture
                .home
                .join("workspaces/website/review/app/file.txt")
                .is_file()
        );
        agent.close(Some(libc::SIGKILL));
        fs::write(fixture.home.join("release-close"), "").unwrap();
        wait_for(|| fixture.home.join("closed").exists());
        let mut reader = client(&fixture, "reader");
        assert!(
            fixture
                .home
                .join("workspaces/website/review/app/file.txt")
                .is_file()
        );
        assert!(fixture.runtime_record("review", "journal").is_some());
        assert!(
            reader.tool("search_events", json!({"search_strings":["incident"]}))["reports"]
                .as_array()
                .unwrap()
                .is_empty()
        );
        let calls = fs::read_to_string(fixture.home.join("docker-calls")).unwrap();
        assert!(!calls.contains("rm --force"), "{calls}");
        assert!(!calls.contains("stop --"), "{calls}");
    }
}

#[test]
fn close_rejects_noops_missing_metadata_and_callers_from_other_instances() {
    let fixture = fixture();
    let mut agent = client(&fixture, "review");
    for arguments in [json!({}), json!({"sessions":"none", "zellij_tab":false})] {
        assert_eq!(
            agent.request("tools/call", json!({"name":"close", "arguments":arguments}))["isError"],
            true
        );
    }
    assert_eq!(
        agent.request(
            "tools/call",
            json!({"name":"close", "arguments":{"sessions":"current"}})
        )["isError"],
        true
    );
    let mut reader = client(&fixture, "reader");
    assert_eq!(reader.request("tools/call", json!({"name":"close", "arguments":{"sessions":"all"}, "_meta":{"ai.opencode/sessionID":"ses_incident"}}))["isError"], true);
    assert!(!fixture.home.join("closing").exists());
    assert!(
        fixture
            .home
            .join("workspaces/website/review/app/file.txt")
            .is_file()
    );
}
