use super::super::AppService;

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
            .submit_operation("stop_instance", "gateway", None, 60, true)
            .is_err()
    );
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
