use super::AppService;

#[test]
fn instance_batches_require_confirmation_and_allow_only_stop_or_delete() {
    let service = AppService::for_tests();
    for action in ["stop_instance", "delete_instance"] {
        assert!(
            service
                .submit_instance_batch(action, &["review".into()], false)
                .unwrap_err()
                .contains("confirmation_required")
        );
    }
    assert!(
        service
            .submit_instance_batch("remove_template", &["website".into()], true)
            .is_err()
    );
    assert!(service.operations().is_empty());
}

#[test]
fn instance_batches_continue_after_admission_failures_and_deduplicate_targets() {
    for action in ["stop_instance", "delete_instance"] {
        let service = AppService::for_tests();
        service.queue_instance_for_tests("busy", "website");
        let batch = service
            .submit_instance_batch(
                action,
                &[
                    "busy".into(),
                    "gateway".into(),
                    "other".into(),
                    "other".into(),
                ],
                true,
            )
            .unwrap();
        assert_eq!(batch.operations.len(), 1);
        assert_eq!(batch.operations[0].name, "other");
        assert_eq!(batch.operations[0].action, action);
        assert_eq!(batch.errors.len(), 2);
        assert!(batch.errors.iter().any(|error| error.starts_with("busy:")));
        assert!(
            batch
                .errors
                .iter()
                .any(|error| error.starts_with("gateway:"))
        );
    }
}
