use super::*;
use crate::store::rules::reports::ReportInput;
use crate::store::{events::Batch, rules::Definition};

#[test]
#[ignore = "entry point for detached test workers"]
fn worker_entry() {
    let Ok(id) = std::env::var("TANDEM_TEST_CONCLUSION_ID") else {
        return;
    };
    let descriptors: Vec<i32> = std::env::var("TANDEM_TEST_CONCLUSION_FDS")
        .unwrap()
        .split(',')
        .map(|part| part.parse().unwrap())
        .collect();
    AppService::run_conclusion_worker(
        id.parse().unwrap(),
        std::env::var("TANDEM_TEST_CONCLUSION_RECORD")
            .unwrap()
            .parse()
            .unwrap(),
        &std::env::var("TANDEM_TEST_CONCLUSION_NAME").unwrap(),
        descriptors.try_into().unwrap(),
    )
    .unwrap();
}

fn report() -> ReportInput {
    ReportInput {
        title: "Inspected the incident".into(),
        summary: "Verified the retry fix".into(),
        markdown: "# Investigation\n\nThe timeout was reproduced and the retry verified.\n".into(),
        ..Default::default()
    }
}

fn workspace(service: &AppService, name: &str) -> Arc<InstanceScope> {
    let operation = service
        .submit_operation("create_instance", name, Some("blank".into()), 60, true)
        .unwrap();
    let completed = service
        .runtime
        .block_on(service.wait_operation(&operation.id))
        .unwrap();
    assert_eq!(
        completed.state,
        OperationState::Succeeded,
        "{:?}",
        completed.error
    );
    Arc::new(
        service
            .environments
            .bind_instance_directory(std::path::Path::new(
                &service.environments.workspace(name).unwrap(),
            ))
            .unwrap(),
    )
}

#[test]
fn readiness_pings_follow_fresh_settings_and_preserve_the_workspace() {
    let service = AppService::for_tests();
    service
        .set_clear_opencode_history(false)
        .unwrap()
        .blocking_recv()
        .unwrap()
        .unwrap();
    service
        .runtime
        .block_on(service.create_template("blank".into()))
        .unwrap();
    let scope = workspace(&service, "review");
    let path = service.environments.config.workspaces.join("blank/review");
    std::fs::write(path.join("result.txt"), "deliverable").unwrap();
    let receipt = service
        .runtime
        .block_on(service.ping_instance(scope.clone(), None))
        .unwrap();
    assert_eq!(receipt.instance, "review");
    assert!(!receipt.sound_scheduled);
    assert_eq!(service.completion_sound_count_for_tests(), 0);

    let other = AppService::from_config(service.environments.config.clone()).unwrap();
    other
        .set_instance_ping_enabled(true)
        .unwrap()
        .blocking_recv()
        .unwrap()
        .unwrap();
    let receipt = service
        .runtime
        .block_on(service.ping_instance(scope.clone(), Some("ses_review".into())))
        .unwrap();
    assert!(receipt.sound_scheduled);
    assert_eq!(service.completion_sound_count_for_tests(), 1);
    other.environments.refresh_instances();
    let pings = other.environments.snapshot().session_pings;
    assert_eq!(pings.len(), 1);
    assert_eq!(pings[0].instance, "review");
    assert_eq!(pings[0].session_id, "ses_review");
    assert!(pings[0].revision > 0);
    other
        .set_instance_ping_enabled(false)
        .unwrap()
        .blocking_recv()
        .unwrap()
        .unwrap();
    assert!(
        !service
            .runtime
            .block_on(service.ping_instance(scope.clone(), Some("ses_muted".into())))
            .unwrap()
            .sound_scheduled
    );
    assert_eq!(service.completion_sound_count_for_tests(), 1);
    other.environments.refresh_instances();
    assert_eq!(other.environments.snapshot().session_pings, pings);
    assert_eq!(
        std::fs::read_to_string(path.join("result.txt")).unwrap(),
        "deliverable"
    );
    assert!(service.rules.store.snapshot().unwrap().reports.is_empty());
    std::fs::rename(&path, path.with_file_name("retained-review")).unwrap();
    std::fs::create_dir(&path).unwrap();
    assert!(
        service
            .runtime
            .block_on(service.ping_instance(scope, None))
            .unwrap_err()
            .contains("replaced")
    );
    assert_eq!(service.completion_sound_count_for_tests(), 1);
}

#[test]
fn purge_preserves_checkpoints_and_reuses_historical_acceptance_reports() {
    let service = AppService::for_tests();
    service
        .runtime
        .block_on(service.set_clear_opencode_history(false).unwrap())
        .unwrap()
        .unwrap();
    service
        .runtime
        .block_on(service.create_template("blank".into()))
        .unwrap();
    let unlinked = workspace(&service, "manual");
    let manual_memory = service
        .runtime
        .block_on(service.remember(unlinked.clone(), report()))
        .unwrap();
    let receipt = service
        .runtime
        .block_on(service.purge_instance(unlinked))
        .unwrap();
    assert_eq!(receipt.instance, "manual");
    assert_eq!(receipt.acceptance_id, None);
    assert_eq!(receipt.cleanup_state, CleanupState::Pending);
    let manual_id = receipt.conclusion_id;
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
    while service
        .environments
        .config
        .workspaces
        .join("blank/manual")
        .exists()
    {
        assert!(
            std::time::Instant::now() < deadline,
            "conclusion purge timed out"
        );
        std::thread::sleep(std::time::Duration::from_millis(30));
    }
    assert!(service.rules.store.snapshot().unwrap().reports.is_empty());
    let saved = service
        .runtime
        .block_on(service.get_conclusion(manual_id))
        .unwrap();
    assert!(saved.report.is_none());
    assert_eq!(
        service
            .runtime
            .block_on(service.get_memory(manual_memory.memory_id))
            .unwrap()
            .markdown,
        report().markdown
    );
    service
        .rules
        .store
        .save(
            Definition {
                name: "inspect".into(),
                description: String::new(),
                script: "fn matches(event) { true }".into(),
                template: "blank".into(),
                target_instance: None,
                session_name: None,
                delivery_mode: Default::default(),
                model: "openai/test".into(),
                variant: None,
                initial_prompt: "Inspect {{event.summary}}".into(),
                follow_up_prompt: None,
                enabled: true,
                start_instance: false,
                focus_pane: true,
                open_client: true,
                throttle_seconds: 0,
                trigger_at_end: false,
            },
            None,
            "main".into(),
        )
        .unwrap();
    let token = service.register_provider_for_tests("sample");
    service
        .runtime
        .block_on(service.ingest_events(
            token,
            Batch {
                events: vec![crate::environments::events::tests::event("one")],
            },
        ))
        .unwrap();
    service.rules.store.evaluate().unwrap();
    let mut acceptance = service
        .rules
        .store
        .snapshot()
        .unwrap()
        .acceptances
        .remove(0);
    let scope = workspace(&service, &acceptance.instance);
    acceptance.operation_id = Some(
        crate::environments::startup::read(&service.environments.config, &acceptance.instance)
            .unwrap()
            .unwrap()
            .operation
            .id,
    );
    service.rules.store.update(&acceptance, false).unwrap();
    let checkpoint = service
        .runtime
        .block_on(service.remember(scope.clone(), report()))
        .unwrap();
    assert_eq!(checkpoint.acceptance_id, Some(acceptance.id));
    assert_eq!(
        checkpoint.workspace_identity,
        scope.identity(&service.environments.config).unwrap()
    );
    assert_eq!(
        service.rules.store.snapshot().unwrap().acceptances,
        vec![acceptance.clone()]
    );
    assert!(service.rules.store.snapshot().unwrap().reports.is_empty());
    assert!(
        !service
            .rules
            .store
            .reported_acceptances()
            .unwrap()
            .contains(&acceptance.id)
    );
    let historical = service
        .rules
        .store
        .save_conclusion(
            &acceptance.instance,
            &scope.identity(&service.environments.config).unwrap(),
            Some(acceptance.id),
            Some(&report()),
        )
        .unwrap();
    let receipt = service
        .runtime
        .block_on(service.purge_instance(scope.clone()))
        .unwrap();
    assert_eq!(receipt.conclusion_id, historical.conclusion_id);
    assert_eq!(receipt.memory_id, historical.memory_id);
    assert_eq!(receipt.acceptance_id, Some(acceptance.id));
    assert_eq!(receipt.cleanup_state, CleanupState::Pending);
    let final_memory = receipt.memory_id.unwrap();
    assert_ne!(final_memory, checkpoint.memory_id);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
    loop {
        let saved = service
            .runtime
            .block_on(service.get_event_report(acceptance.id))
            .unwrap();
        assert_eq!(saved.markdown, report().markdown);
        if saved.details.cleanup_state == CleanupState::Purged {
            break;
        }
        assert_ne!(
            saved.details.cleanup_state,
            CleanupState::Failed,
            "{:?}",
            saved.details.cleanup_error
        );
        assert!(
            std::time::Instant::now() < deadline,
            "conclusion purge timed out"
        );
        std::thread::sleep(std::time::Duration::from_millis(30));
    }
    assert!(
        !service
            .environments
            .config
            .workspaces
            .join("blank")
            .join(&acceptance.instance)
            .exists()
    );
    assert_eq!(
        service.rules.store.snapshot().unwrap().acceptances,
        vec![acceptance.clone()]
    );
    assert_eq!(
        service
            .runtime
            .block_on(service.search_event_reports(vec!["TIMEOUT".into()]))
            .unwrap()[0]
            .acceptance_id,
        acceptance.id
    );
    assert!(
        service
            .runtime
            .block_on(service.purge_instance(scope))
            .is_err()
    );
    let replacement = workspace(&service, &acceptance.instance);
    let error = service
        .runtime
        .block_on(service.purge_instance(replacement.clone()))
        .unwrap_err();
    assert!(error.contains("another acceptance lineage"), "{error}");
    assert!(
        service
            .runtime
            .block_on(service.remember(replacement, report()))
            .is_err()
    );
    service
        .delete_events(crate::store::events::Deletion::Acceptance(acceptance.id))
        .blocking_recv()
        .unwrap()
        .unwrap();
    for id in [checkpoint.memory_id, final_memory] {
        let memory = service.runtime.block_on(service.get_memory(id)).unwrap();
        assert_eq!(memory.details.acceptance_id, Some(acceptance.id));
        assert_eq!(memory.markdown, report().markdown);
    }
    assert!(
        service
            .environments
            .config
            .workspaces
            .join("blank")
            .join(&acceptance.instance)
            .exists()
    );
}
