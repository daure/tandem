use super::*;
use crate::environments::events::tests::event;

#[test]
fn sidecar_authenticates_ingestion_and_delivers_acknowledgeable_provider_feedback() {
    let service = AppService::for_tests();
    let token = service.register_provider_for_tests("sample");
    let other = service.register_provider_for_tests("other");
    let runtime = tokio::runtime::Runtime::new().unwrap();
    runtime.block_on(async {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let origin = format!("http://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(axum::serve(listener, router(service.clone())).into_future());
        let client = reqwest::Client::new();
        let events = format!("{origin}/v1/events");
        let body = serde_json::to_string(&Batch {
            events: vec![event("one")],
        })
        .unwrap();
        assert_eq!(
            client
                .post(&events)
                .body(body.clone())
                .send()
                .await
                .unwrap()
                .status(),
            StatusCode::UNAUTHORIZED
        );
        assert_eq!(
            client
                .post(&events)
                .bearer_auth(&token)
                .header("Origin", "http://localhost")
                .body(body.clone())
                .send()
                .await
                .unwrap()
                .status(),
            StatusCode::UNAUTHORIZED
        );
        for duplicate in [false, true] {
            let response = client
                .post(&events)
                .bearer_auth(&token)
                .header("Content-Type", "application/json")
                .body(body.clone())
                .send()
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::OK);
            let receipt: serde_json::Value =
                serde_json::from_str(&response.text().await.unwrap()).unwrap();
            assert_eq!(receipt["receipts"][0]["duplicate"], duplicate);
        }
        let queue = format!("{origin}/v1/notifications");
        let response = client.get(&queue).bearer_auth(&token).send().await.unwrap();
        let payload: serde_json::Value =
            serde_json::from_str(&response.text().await.unwrap()).unwrap();
        assert_eq!(payload["notifications"].as_array().unwrap().len(), 1);
        assert_eq!(payload["notifications"][0]["kind"], "received");
        let id = payload["notifications"][0]["notification_id"]
            .as_i64()
            .unwrap();
        let ack = format!("{queue}/{id}/ack");
        assert_eq!(
            client
                .post(&ack)
                .bearer_auth(&other)
                .send()
                .await
                .unwrap()
                .status(),
            StatusCode::NOT_FOUND
        );
        assert_eq!(
            client
                .post(&ack)
                .bearer_auth(&token)
                .send()
                .await
                .unwrap()
                .status(),
            StatusCode::NO_CONTENT
        );
        let response = client.get(&queue).bearer_auth(&token).send().await.unwrap();
        let payload: serde_json::Value =
            serde_json::from_str(&response.text().await.unwrap()).unwrap();
        assert!(payload["notifications"].as_array().unwrap().is_empty());
        assert_eq!(
            client
                .get(format!("{origin}/mcp"))
                .send()
                .await
                .unwrap()
                .status(),
            StatusCode::NOT_FOUND
        );
        let oversized = "x".repeat(1_048_577);
        assert_eq!(
            client
                .post(&events)
                .bearer_auth(&token)
                .header("Content-Type", "application/json")
                .body(oversized)
                .send()
                .await
                .unwrap()
                .status(),
            StatusCode::PAYLOAD_TOO_LARGE
        );
        server.abort();
        let _ = server.await;
    });
}
