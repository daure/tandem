use super::*;
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
            .bind_instance_directory(&service.environments.config.workspaces.join(name))
            .unwrap(),
    )
}

#[test]
fn conclusion_saves_acceptance_reports_before_detached_purge_and_rejects_unlinked_workspaces() {
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
    let error = service
        .runtime
        .block_on(service.conclude_instance(unlinked, report()))
        .unwrap_err();
    assert!(error.contains("acceptance"), "{error}");
    assert!(
        service
            .environments
            .config
            .workspaces
            .join("manual")
            .is_dir()
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
                model: "openai/test".into(),
                initial_prompt: "Inspect {{event.summary}}".into(),
                enabled: true,
                start_instance: false,
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
    let receipt = service
        .runtime
        .block_on(service.conclude_instance(scope.clone(), report()))
        .unwrap();
    assert_eq!(receipt.acceptance_id, acceptance.id);
    assert_eq!(receipt.event_sequence, acceptance.event_sequence);
    assert_eq!(receipt.cleanup_state, CleanupState::Pending);
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
            .join(&acceptance.instance)
            .exists()
    );
    assert!(
        service
            .environments
            .config
            .workspaces
            .join("manual")
            .is_dir()
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
            .block_on(service.conclude_instance(scope, report()))
            .is_err()
    );
    let replacement = workspace(&service, &acceptance.instance);
    let error = service
        .runtime
        .block_on(service.conclude_instance(replacement, report()))
        .unwrap_err();
    assert!(error.contains("another acceptance lineage"), "{error}");
    assert!(
        service
            .environments
            .config
            .workspaces
            .join(&acceptance.instance)
            .exists()
    );
}
