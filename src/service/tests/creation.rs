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
        );
        assert!(matches!(result, Err(error) if error == "OpenCode integration is disabled"));
        assert!(service.operations().is_empty());
        assert!(service.environment_snapshot().instances.is_empty());
    }
}

#[test]
fn background_launch_waits_for_workspace_readiness_and_reports_launch_errors() {
    let service = AppService::for_tests();
    service
        .runtime
        .block_on(service.set_opencode_enabled(false).unwrap())
        .unwrap()
        .unwrap();
    let (sender, ready) = tokio::sync::oneshot::channel();
    let mut reply =
        service.schedule_instance_opencode(ready, "review".into(), Some("Explain".into()));
    assert_eq!(
        reply.try_recv(),
        Err(tokio::sync::oneshot::error::TryRecvError::Empty)
    );
    sender.send("/unused/workspace".into()).unwrap();
    assert_eq!(
        service.runtime.block_on(reply).unwrap(),
        Err("OpenCode integration is disabled".into())
    );
}

#[test]
fn failed_workspace_preparation_skips_the_background_launch() {
    let service = AppService::for_tests();
    service
        .runtime
        .block_on(service.set_opencode_enabled(false).unwrap())
        .unwrap()
        .unwrap();
    let (sender, ready) = tokio::sync::oneshot::channel();
    let reply = service.schedule_instance_opencode(ready, "review".into(), None);
    drop(sender);
    assert_eq!(service.runtime.block_on(reply).unwrap(), Ok(()));
}
