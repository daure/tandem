use super::*;
use crate::store::providers::{Provider, RuntimeObservation};
use crate::{environments::events::EventStore, store::events::Batch};

#[path = "provider_streams.rs"]
mod streams;

#[test]
fn sidecar_acknowledges_discarded_batches_without_receipts_or_feedback() {
    let service = AppService::for_tests();
    let token = service.register_provider_for_tests("sample");
    let store = EventStore::open(&service.environments.config).unwrap();
    store.set_ingestion_enabled("sample", false).unwrap();
    let runtime = tokio::runtime::Runtime::new().unwrap();
    runtime.block_on(async {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let origin = format!("http://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(
            axum::serve(listener, crate::events_http::router(service.clone())).into_future(),
        );
        let batch = Batch {
            events: vec![crate::environments::events::tests::event("discarded")],
        };
        let response = reqwest::Client::builder()
            .timeout(Duration::from_secs(5))
            .build()
            .unwrap()
            .post(format!("{origin}/v1/events"))
            .bearer_auth(&token)
            .header("Content-Type", "application/json")
            .body(serde_json::to_string(&batch).unwrap())
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), axum::http::StatusCode::OK);
        let payload: serde_json::Value =
            serde_json::from_str(&response.text().await.unwrap()).unwrap();
        assert_eq!(
            payload,
            serde_json::json!({"receipts": [], "discarded": ["discarded"]})
        );
        server.abort();
        let _ = server.await;
    });
    assert_eq!(store.snapshot().unwrap().total, 0);
    assert!(store.notifications(&token).unwrap().is_empty());
}

fn provider(name: &str, status: Status) -> Provider {
    Provider {
        name: name.into(),
        directory: format!("/providers/{name}"),
        manifest: None,
        available: true,
        status,
        container_id: Some(format!("{name}-id")),
        error: Some("prior failure".into()),
        operation: None,
        streams: vec![],
    }
}

#[test]
fn verified_completion_updates_only_its_provider_and_preserves_other_pending_actions() {
    let service = AppService::for_tests();
    let integration = &service.providers;
    *integration.snapshot.lock().unwrap() = Snapshot {
        providers: vec![
            provider("message", Status::Running),
            provider("ticket", Status::Running),
        ],
        error: None,
    };
    *integration.operations.lock().unwrap() = [
        ("message".into(), Action::Stop),
        ("ticket".into(), Action::Stop),
    ]
    .into();
    integration.complete_action(
        "message",
        Action::Stop,
        &Ok(ActionOutcome {
            message: "stopped".into(),
            runtime: Some(RuntimeObservation {
                status: Status::Stopped,
                container_id: "message-id".into(),
            }),
        }),
    );
    let snapshot = service.provider_snapshot();
    assert_eq!(snapshot.providers[0].status, Status::Stopped);
    assert_eq!(
        snapshot.providers[0].container_id.as_deref(),
        Some("message-id")
    );
    assert_eq!(snapshot.providers[0].error, None);
    assert_eq!(snapshot.providers[0].operation, None);
    assert_eq!(snapshot.providers[1].status, Status::Running);
    assert_eq!(snapshot.providers[1].operation, Some(Action::Stop));
    assert_eq!(
        snapshot.providers[1].error.as_deref(),
        Some("prior failure")
    );
}

#[test]
fn observations_started_before_completion_cannot_overwrite_verified_runtime_state() {
    let service = AppService::for_tests();
    let integration = &service.providers;
    let stale = Snapshot {
        providers: vec![provider("message", Status::Running)],
        error: None,
    };
    *integration.snapshot.lock().unwrap() = stale.clone();
    let revision = integration.revision.load(Ordering::Acquire);
    integration.complete_action(
        "message",
        Action::Stop,
        &Ok(ActionOutcome {
            message: "stopped".into(),
            runtime: Some(RuntimeObservation {
                status: Status::Stopped,
                container_id: "message-id".into(),
            }),
        }),
    );
    assert!(!integration.publish_observation(revision, Ok(stale)));
    assert_eq!(
        service.provider_snapshot().providers[0].status,
        Status::Stopped
    );
    let revision = integration.revision.load(Ordering::Acquire);
    let fresh = Snapshot {
        providers: vec![provider("message", Status::Stopped)],
        error: None,
    };
    assert!(integration.publish_observation(revision, Ok(fresh)));
    assert_eq!(
        service.provider_snapshot().providers[0].status,
        Status::Stopped
    );
}

#[test]
fn failed_lifecycle_completion_stays_unverified_and_logs_preserve_runtime_state() {
    let service = AppService::for_tests();
    let integration = &service.providers;
    *integration.snapshot.lock().unwrap() = Snapshot {
        providers: vec![provider("message", Status::Running)],
        error: None,
    };
    for action in [Action::Logs, Action::Stop] {
        let error = if action == Action::Logs {
            ActionError::Failed("logs unavailable".into())
        } else {
            ActionError::Unavailable("runtime changed")
        };
        integration.complete_action("message", action, &Err(error));
        assert_eq!(
            service.provider_snapshot().providers[0].status,
            Status::Running
        );
        assert_eq!(
            service.provider_snapshot().providers[0].error.as_deref(),
            Some("prior failure")
        );
    }
    integration.complete_action(
        "message",
        Action::Stop,
        &Err(ActionError::Failed("Docker failed".into())),
    );
    let snapshot = service.provider_snapshot();
    assert_eq!(snapshot.providers[0].status, Status::Unknown);
    assert_eq!(
        snapshot.providers[0].error.as_deref(),
        Some("Docker failed")
    );
}
