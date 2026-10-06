use super::*;
use serde_json::json;

#[test]
fn provider_manifests_validate_independent_stream_profiles_and_unique_names() {
    let input = json!({
        "schema_version": 2, "name": "source", "description": "Source",
        "protocol": "tandem-events-v1", "stream_control": true,
        "streams": [
            {"name": "messages", "profile": "message"},
            {"name": "tickets", "profile": "ticket"},
            {"name": "logs", "profile": "system_event"},
            {"name": "releases", "profile": "generic"}
        ]
    });
    let manifest: Manifest = serde_json::from_value(input.clone()).unwrap();
    manifest.validate().unwrap();
    assert_eq!(
        serde_json::to_value(&manifest).unwrap()["streams"],
        input["streams"]
    );
    for streams in [
        json!([]),
        json!([{"name": "same", "profile": "message"}, {"name": "same", "profile": "ticket"}]),
        json!([{"name": "", "profile": "message"}]),
        json!([{"name": "logs", "profile": "unsupported"}]),
    ] {
        let mut invalid = input.clone();
        invalid["streams"] = streams;
        assert!(
            serde_json::from_value::<Manifest>(invalid)
                .unwrap()
                .validate()
                .is_err()
        );
    }
    for streams in [
        json!(["messages"]),
        json!([{"name": "messages"}]),
        json!([{"name": "messages", "profile": "message", "extra": true}]),
    ] {
        let mut invalid = input.clone();
        invalid["streams"] = streams;
        assert!(serde_json::from_value::<Manifest>(invalid).is_err());
    }
    let mut invalid = input;
    invalid["profile"] = json!("message");
    assert!(serde_json::from_value::<Manifest>(invalid).is_err());
}

#[test]
fn schema_one_manifests_preserve_source_identity_and_stream_profiles_when_loaded() {
    let manifest: Manifest = serde_json::from_value(json!({
        "schema_version": 1, "name": "source", "profile": "ticket",
        "description": "Source", "protocol": "tandem-events-v1",
        "streams": ["backlog", "mentions"], "stream_control": true,
        "feedback": ["received"]
    }))
    .unwrap();
    manifest.validate().unwrap();
    let canonical = serde_json::to_value(&manifest).unwrap();
    assert_eq!(
        canonical,
        json!({
            "schema_version": 2, "name": "source", "description": "Source",
            "protocol": "tandem-events-v1", "stream_control": true,
            "feedback": ["received"],
            "streams": [{"name": "backlog", "profile": "ticket"}, {"name": "mentions", "profile": "ticket"}]
        })
    );
    assert_eq!(
        serde_json::from_value::<Manifest>(canonical).unwrap(),
        manifest
    );
}
