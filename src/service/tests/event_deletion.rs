use super::*;
use crate::store::{
    events::Deletion,
    rules::{Acceptance, Definition},
};

fn delete(service: &AppService, target: Deletion) -> Result<i64, Error> {
    service.delete_events(target).blocking_recv().unwrap()
}

fn accepted_events(service: &AppService) -> (String, Vec<Acceptance>) {
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
                variant: None,
                initial_prompt: "Inspect {{event.summary}}".into(),
                enabled: true,
                start_instance: false,
                focus_pane: true,
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
            token.clone(),
            Batch {
                events: vec![
                    crate::environments::events::tests::event("first"),
                    crate::environments::events::tests::event("second"),
                ],
            },
        ))
        .unwrap();
    service.rules.store.evaluate().unwrap();
    let acceptances = service.rules.store.snapshot().unwrap().acceptances;
    (token, acceptances)
}

#[test]
fn event_deletion_proceeds_while_unrelated_rule_work_holds_the_scheduler_and_event_leases() {
    for by_acceptance in [false, true] {
        let service = AppService::for_tests();
        let (_token, acceptances) = accepted_events(&service);
        let busy = &acceptances[0];
        let removable = &acceptances[1];
        let store = &service.rules.store;
        let _scheduler = store.lease().unwrap().unwrap();
        let _event = store.event_lease(busy.event_sequence).unwrap().unwrap();
        let target = if by_acceptance {
            Deletion::Acceptance(removable.id)
        } else {
            Deletion::Event(removable.event_sequence)
        };
        assert_eq!(delete(&service, target).unwrap(), 1);
        assert_eq!(store.acceptance(busy.id).unwrap(), *busy);
        assert!(matches!(
            store.acceptance(removable.id),
            Err(Error::NotFound)
        ));
    }
}

#[test]
fn ignored_event_deletion_proceeds_while_the_rule_worker_holds_its_leases() {
    let service = AppService::for_tests();
    let (token, acceptances) = accepted_events(&service);
    let receipt = service
        .runtime
        .block_on(service.ingest_events(
            token.clone(),
            Batch {
                events: vec![crate::environments::events::tests::event("pending")],
            },
        ))
        .unwrap();
    let store = &service.rules.store;
    let _scheduler = store.lease().unwrap().unwrap();
    let _event = store
        .event_lease(acceptances[0].event_sequence)
        .unwrap()
        .unwrap();
    assert_eq!(delete(&service, Deletion::Ignored).unwrap(), 1);
    assert_eq!(store.snapshot().unwrap().acceptances, acceptances);
    assert!(matches!(
        store.event(receipt.receipts[0].sequence),
        Err(Error::NotFound)
    ));
    store.evaluate().unwrap();
    assert_eq!(store.snapshot().unwrap().acceptances.len(), 2);
    let snapshot = service.runtime.block_on(service.list_events()).unwrap();
    assert_eq!(snapshot.total, 2);
    let notifications = service
        .runtime
        .block_on(service.provider_notifications(token))
        .unwrap();
    assert_eq!(notifications.len(), 2);
}

#[test]
fn event_deletion_refuses_only_the_affected_busy_event_and_preserves_bulk_atomicity() {
    let service = AppService::for_tests();
    let (_token, acceptances) = accepted_events(&service);
    let busy = &acceptances[0];
    let _event = service
        .rules
        .store
        .event_lease(busy.event_sequence)
        .unwrap()
        .unwrap();
    let before = service.runtime.block_on(service.list_events()).unwrap();
    for target in [
        Deletion::Event(busy.event_sequence),
        Deletion::Acceptance(busy.id),
        Deletion::All,
    ] {
        let error = delete(&service, target).unwrap_err();
        assert!(matches!(error, Error::Conflict(_)));
        assert_eq!(
            error.to_string(),
            format!(
                "event #{} is busy; retry when its operation finishes",
                busy.event_sequence
            )
        );
        assert_eq!(
            service.runtime.block_on(service.list_events()).unwrap(),
            before
        );
    }
    let other = &acceptances[1];
    assert_eq!(
        delete(&service, Deletion::Event(other.event_sequence)).unwrap(),
        1
    );
}

#[test]
fn event_deletion_preserves_acceptance_history_during_conclusion_cleanup() {
    let service = AppService::for_tests();
    let (_token, acceptances) = accepted_events(&service);
    let busy = &acceptances[0];
    let _conclusion = crate::environments::conclusion::reserve(
        &service.environments.config,
        busy.id,
        &busy.instance,
    )
    .unwrap();
    service
        .rules
        .store
        .save_report(
            busy.id,
            &crate::store::rules::reports::ReportInput {
                title: "Completed inspection".into(),
                summary: "Retained evidence".into(),
                markdown: "# Evidence\nThe inspection is complete.\n".into(),
            },
        )
        .unwrap();
    for target in [
        Deletion::Event(busy.event_sequence),
        Deletion::Acceptance(busy.id),
        Deletion::All,
    ] {
        let error = delete(&service, target).unwrap_err();
        assert!(matches!(error, Error::Conflict(_)));
        assert!(error.to_string().contains("conclusion is busy"));
        assert_eq!(service.rules.store.acceptance(busy.id).unwrap(), *busy);
        assert!(service.rules.store.event_report(busy.id).is_ok());
    }
    assert_eq!(
        delete(&service, Deletion::Event(acceptances[1].event_sequence)).unwrap(),
        1
    );
}

#[test]
fn reported_work_can_delete_history_after_cleanup_even_when_dispatch_is_historically_launching() {
    let service = AppService::for_tests();
    let (_token, mut acceptances) = accepted_events(&service);
    let acceptance = &mut acceptances[0];
    acceptance.status = crate::store::rules::DispatchStatus::Launching;
    service.rules.store.update(acceptance, false).unwrap();
    service
        .rules
        .store
        .save_report(
            acceptance.id,
            &crate::store::rules::reports::ReportInput {
                title: "Inspection complete".into(),
                summary: "Verified work".into(),
                markdown: "# Evidence\nVerified".into(),
            },
        )
        .unwrap();
    service
        .rules
        .store
        .set_report_cleanup(
            acceptance.id,
            crate::store::rules::reports::CleanupState::Purged,
            None,
        )
        .unwrap();
    assert_eq!(
        delete(&service, Deletion::Acceptance(acceptance.id)).unwrap(),
        1
    );
}
