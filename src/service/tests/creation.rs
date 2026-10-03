use super::AppService;

#[test]
fn opencode_creation_is_rejected_before_admission_when_integration_is_disabled() {
    let service = AppService::for_tests();
    service
        .runtime
        .block_on(service.set_opencode_enabled(false).unwrap())
        .unwrap()
        .unwrap();
    for prompt in [None, Some("Explain this project".into())] {
        let result = service.submit_new_instance(
            "review",
            "website".into(),
            "Review\nenvironment".into(),
            Some(prompt),
            true,
        );
        assert!(matches!(result, Err(error) if error == "OpenCode integration is disabled"));
        assert!(service.operations().is_empty());
        assert!(service.environment_snapshot().instances.is_empty());
    }
}
