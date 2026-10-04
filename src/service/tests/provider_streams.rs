use super::*;
use crate::{
    environments::events::EventStore,
    store::providers::{Manifest, Status, StreamControl},
};
use crate::{environments::events::tests::event, events_http::router};
use axum::http::StatusCode;
use serde_json::json;

#[test]
fn stream_controls_require_current_provider_acknowledgments_and_isolate_sibling_ingestion() {
    let service = AppService::for_tests();
    let token = service.register_provider_for_tests("sample");
    let other = service.register_provider_for_tests("other");
    let store = EventStore::open(&service.environments.config).unwrap();
    let manifest: Manifest = serde_json::from_value(json!({
        "schema_version": 1, "name": "sample", "profile": "message",
        "description": "Fixture", "protocol": "tandem-events-v1",
        "streams": ["samples", "sibling"], "stream_control": true,
    }))
    .unwrap();
    store.prepare_streams(&manifest).unwrap();
    assert!(
        store
            .provider_streams(&manifest, Status::Running)
            .unwrap()
            .iter()
            .all(|stream| stream.status == Status::Starting)
    );
    let connection =
        rusqlite::Connection::open(service.environments.config.home.join("settings.sqlite3"))
            .unwrap();
    connection
        .execute(
            "UPDATE provider_streams SET requested_at = unixepoch() - 11",
            [],
        )
        .unwrap();
    assert!(
        store
            .provider_streams(&manifest, Status::Running)
            .unwrap()
            .iter()
            .all(|stream| stream.status == Status::Unknown)
    );
    store.prepare_streams(&manifest).unwrap();
    let runtime = tokio::runtime::Runtime::new().unwrap();
    runtime.block_on(async {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let origin = format!("http://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(axum::serve(listener, router(service.clone())).into_future());
        let client = reqwest::Client::new();
        let controls = format!("{origin}/v1/streams");
        assert_eq!(
            client.get(&controls).send().await.unwrap().status(),
            StatusCode::UNAUTHORIZED
        );
        let response = client
            .get(&controls)
            .bearer_auth(&token)
            .send()
            .await
            .unwrap();
        let payload: serde_json::Value =
            serde_json::from_str(&response.text().await.unwrap()).unwrap();
        assert_eq!(payload["streams"].as_array().unwrap().len(), 2);
        let mut control: StreamControl =
            serde_json::from_value(payload["streams"][0].clone()).unwrap();
        let sibling_control: StreamControl =
            serde_json::from_value(payload["streams"][1].clone()).unwrap();
        store.acknowledge_stream(&token, &sibling_control).unwrap();
        assert_eq!(
            store.provider_streams(&manifest, Status::Running).unwrap()[1].status,
            Status::Running
        );
        assert_eq!(store.snapshot().unwrap().total, 0);
        let revision = store.request_stream("sample", "samples", false).unwrap();
        let ack = format!("{controls}/ack");
        let send_ack = |credential: String, control: StreamControl| {
            let client = client.clone();
            let ack = ack.clone();
            async move {
                client
                    .post(ack)
                    .bearer_auth(credential)
                    .header("Content-Type", "application/json")
                    .body(serde_json::to_string(&control).unwrap())
                    .send()
                    .await
                    .unwrap()
                    .status()
            }
        };
        assert_eq!(
            send_ack(token.clone(), control.clone()).await,
            StatusCode::CONFLICT
        );
        assert!(!store.stream_applied("sample", "samples", revision).unwrap());
        assert_eq!(
            store.provider_streams(&manifest, Status::Running).unwrap()[0].status,
            Status::Unknown
        );
        control.enabled = false;
        control.revision = revision;
        assert_eq!(send_ack(other, control.clone()).await, StatusCode::CONFLICT);
        assert_eq!(
            send_ack(token.clone(), control.clone()).await,
            StatusCode::NO_CONTENT
        );
        assert!(store.stream_applied("sample", "samples", revision).unwrap());
        assert_eq!(
            store.provider_streams(&manifest, Status::Running).unwrap()[0].status,
            Status::Stopped
        );
        let mut sibling = event("sibling-one");
        sibling.stream = "sibling".into();
        let response = client
            .post(format!("{origin}/v1/events"))
            .bearer_auth(&token)
            .header("Content-Type", "application/json")
            .body(
                serde_json::to_string(&Batch {
                    events: vec![event("ignored"), sibling],
                })
                .unwrap(),
            )
            .send()
            .await
            .unwrap();
        let ingestion: serde_json::Value =
            serde_json::from_str(&response.text().await.unwrap()).unwrap();
        assert_eq!(ingestion["discarded"], json!(["ignored"]));
        assert_eq!(ingestion["receipts"][0]["event_id"], "sibling-one");
        let streams = store.provider_streams(&manifest, Status::Running).unwrap();
        assert_eq!((streams[0].total, streams[1].total), (0, 1));
        assert_eq!(store.snapshot().unwrap().total, 1);
        assert_eq!(store.notifications(&token).unwrap().len(), 1);
        let connection =
            rusqlite::Connection::open(service.environments.config.home.join("settings.sqlite3"))
                .unwrap();
        connection
            .execute(
                "UPDATE provider_streams SET applied_at = unixepoch() - 6",
                [],
            )
            .unwrap();
        connection
            .execute(
                "UPDATE provider_streams SET requested_at = unixepoch() - 11",
                [],
            )
            .unwrap();
        assert!(!store.stream_applied("sample", "samples", revision).unwrap());
        assert_eq!(
            store.provider_streams(&manifest, Status::Running).unwrap()[0].status,
            Status::Unknown
        );
        assert_eq!(
            send_ack(token.clone(), control.clone()).await,
            StatusCode::NO_CONTENT
        );
        assert_eq!(
            store.provider_streams(&manifest, Status::Stopped).unwrap()[0].status,
            Status::Stopped
        );
        store.request_stream("sample", "samples", true).unwrap();
        assert_eq!(
            send_ack(token.clone(), control.clone()).await,
            StatusCode::CONFLICT
        );
        store.prepare_streams(&manifest).unwrap();
        assert_eq!(
            send_ack(token.clone(), control.clone()).await,
            StatusCode::CONFLICT
        );
        let mut reduced = manifest.clone();
        reduced.streams = vec!["sibling".into()];
        store.prepare_streams(&reduced).unwrap();
        assert_eq!(store.stream_controls(&token).unwrap().len(), 1);
        assert_eq!(
            send_ack(token.clone(), control.clone()).await,
            StatusCode::CONFLICT
        );
        store.prepare_streams(&manifest).unwrap();
        assert_eq!(send_ack(token, control).await, StatusCode::CONFLICT);
        server.abort();
        let _ = server.await;
    });
}

#[test]
fn stream_handovers_count_distinct_dispatched_events_across_rules_and_replays() {
    use crate::{environments::rules::RuleStore, store::rules::Definition};

    let service = AppService::for_tests();
    let config = &service.environments.config;
    let token = service.register_provider_for_tests("sample");
    let events = EventStore::open(config).unwrap();
    let rules = RuleStore::open(config).unwrap();
    let manifest: Manifest = serde_json::from_value(json!({
        "schema_version": 1, "name": "sample", "profile": "message",
        "description": "Fixture", "protocol": "tandem-events-v1",
        "streams": ["samples", "sibling"], "stream_control": true,
    }))
    .unwrap();
    for name in ["first", "second"] {
        rules
            .save(
                Definition {
                    name: name.into(),
                    description: "Fixture".into(),
                    script: "fn matches(event) { true }".into(),
                    template: "blank".into(),
                    model: "openai/test".into(),
                    variant: None,
                    initial_prompt: "Inspect {{event.data.text}}".into(),
                    enabled: true,
                    start_instance: true,
                },
                None,
                "main".into(),
            )
            .unwrap();
    }
    let mut sibling = event("sibling");
    sibling.stream = "sibling".into();
    let sequence = events
        .ingest(
            &token,
            Batch {
                events: vec![event("sample"), sibling],
            },
        )
        .unwrap()
        .receipts[0]
        .sequence;
    rules.evaluate().unwrap();
    let streams = events.provider_streams(&manifest, Status::Running).unwrap();
    assert_eq!((streams[0].handovers, streams[1].handovers), (0, 0));
    for replay in [false, true] {
        if replay {
            events.replay(sequence, "additional-dispatch").unwrap();
            rules.evaluate().unwrap();
        }
        for mut acceptance in rules.snapshot().unwrap().acceptances {
            if acceptance.event_sequence == sequence {
                acceptance.operation_id = Some(format!("operation-{}", acceptance.id));
                rules.update(&acceptance, true).unwrap();
            }
        }
        let streams = events.provider_streams(&manifest, Status::Running).unwrap();
        assert_eq!((streams[0].total, streams[1].total), (1, 1));
        assert_eq!((streams[0].handovers, streams[1].handovers), (1, 0));
        assert_eq!(events.snapshot().unwrap().provider_handovers["sample"], 1);
    }
}
