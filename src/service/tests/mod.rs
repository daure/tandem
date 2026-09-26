use super::super::AppService;
use crate::store::environments::Instance;

mod close_command;
mod instance_batch;
mod open_command;
mod opencode;

#[test]
fn mutations_require_confirmation_before_admission() {
    let service = AppService::for_tests();
    let error = service
        .submit_operation(
            "create_instance",
            "review",
            Some("website".into()),
            60,
            false,
        )
        .unwrap_err();
    assert!(error.contains("confirmation_required"));
    assert!(service.operations().is_empty());
    assert!(
        service
            .submit_operation("remove_template", "website", None, 60, false)
            .unwrap_err()
            .contains("confirmation_required")
    );
    assert!(service.operations().is_empty());
    assert!(
        service
            .submit_operation("stop_instance", "gateway", None, 60, true)
            .is_err()
    );
    let operation = service
        .submit_operation("stop_template", "website", None, 60, true)
        .unwrap();
    assert_eq!(operation.action, "stop_template");
    assert!(
        service
            .submit_operation(
                "create_instance",
                "review",
                Some("website".into()),
                901,
                true
            )
            .is_err()
    );
}

#[test]
fn description_updates_run_off_thread_and_publish_to_the_snapshot() {
    let service = AppService::for_tests();
    let operation = service.queue_instance_for_tests("review", "website");
    service.complete_instance_for_tests(
        &operation.id,
        Instance {
            name: "review".into(),
            description: "Original description".into(),
            template: "website".into(),
            template_directory: service
                .environments
                .config
                .templates
                .join("website")
                .display()
                .to_string(),
            workspace: service
                .environments
                .config
                .workspaces
                .join("review")
                .display()
                .to_string(),
            project: service.environments.config.project("review"),
            ..Default::default()
        },
    );

    let reply = service.update_instance_description("review".into(), "Updated".into());
    service.runtime.block_on(reply).unwrap().unwrap();

    assert_eq!(
        service.environment_snapshot().instances[0].description,
        "Updated"
    );
}

#[test]
fn new_instance_description_is_visible_while_creation_is_pending() {
    let service = AppService::for_tests();

    let outcome = service
        .submit_new_instance("review", "website".into(), "Review environment".into())
        .unwrap();

    assert!(matches!(
        outcome,
        crate::service::CreateInstanceOutcome::Started(_)
    ));
    let snapshot = service.environment_snapshot();
    let instance = snapshot
        .instances
        .iter()
        .find(|instance| instance.name == "review")
        .unwrap();
    assert!(instance.pending);
    assert_eq!(instance.description, "Review environment");
}
