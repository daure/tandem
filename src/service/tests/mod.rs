use super::super::AppService;

mod open_command;
mod instance_batch;

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
