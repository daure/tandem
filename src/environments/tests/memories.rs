use crate::environments::config::Config;
use serde_json::json;

use super::*;

fn fixture() -> (tempfile::TempDir, Config, RuleStore) {
    let root = tempfile::tempdir().unwrap();
    let config = Config::at(root.path().into(), "memory-test".into(), 9876).unwrap();
    let store = RuleStore::open(&config).unwrap();
    (root, config, store)
}

fn input(title: &str) -> MemoryInput {
    serde_json::from_value(
        json!({"title":title, "summary":"Verified evidence", "fields":{
        "findings":{"cause":"Retries exhaust the connection pool", "verified":true},
        "decision":"Limit concurrency", "count":3, "optional":null
    }, "labels":[" Backend ", "backend", "incident"],
    "links":[{"label":"PR", "url":"https://EXAMPLE.com/pull/42"}]}),
    )
    .unwrap()
}

#[test]
fn checkpoints_preserve_structured_evidence_across_restarts_namespaces_and_name_reuse() {
    let (_root, config, store) = fixture();
    let first = store
        .remember("review", "first-identity", None, input("Pool diagnosis"))
        .unwrap();
    let second = store
        .remember("review", "second-identity", None, input("Pool decision"))
        .unwrap();
    assert_ne!(first.memory_id, second.memory_id);
    assert_eq!(first.labels, ["backend", "incident"]);
    assert_eq!(first.links[0].url, "https://example.com/pull/42");
    let restarted = RuleStore::open(&config).unwrap();
    let memory = restarted.memory(first.memory_id).unwrap();
    assert_eq!(memory.fields["findings"]["verified"], true);
    assert_eq!(memory.fields["optional"], serde_json::Value::Null);
    assert!(memory.markdown.is_empty());
    let matches = restarted.search_memories(MemorySearch::default()).unwrap();
    assert_eq!(
        matches
            .memories
            .iter()
            .map(|record| record.details.memory_id)
            .collect::<Vec<_>>(),
        [second.memory_id, first.memory_id]
    );
    let other =
        RuleStore::open(&Config::at(config.home.clone(), "other".into(), 9876).unwrap()).unwrap();
    assert!(matches!(
        other.memory(first.memory_id),
        Err(Error::NotFound)
    ));
    assert!(
        other
            .search_memories(MemorySearch::default())
            .unwrap()
            .memories
            .is_empty()
    );
    assert!(restarted.snapshot().unwrap().reports.is_empty());
}

#[test]
fn full_text_search_ranks_evidence_and_combines_word_variants_prefixes_and_exact_filters() {
    let (_root, _config, store) = fixture();
    let preferred = store
        .remember("review", "one", None, input("Retry connection pool"))
        .unwrap();
    let mut body_only = input("Secondary evidence");
    body_only.markdown = "Retries involve a connection pool and café workers".into();
    let other = store.remember("other", "two", None, body_only).unwrap();
    for query in ["retry pool", "RETRIES CONNEC", "pool retr"] {
        let results = store
            .search_memories(MemorySearch {
                query: Some(query.into()),
                ..Default::default()
            })
            .unwrap();
        assert_eq!(results.memories.len(), 2, "{query}");
        assert_eq!(
            results.memories[0].details.memory_id, preferred.memory_id,
            "{query}"
        );
        assert!(
            results
                .memories
                .iter()
                .all(|result| result.snippet.contains('['))
        );
    }
    assert_eq!(
        store
            .search_memories(MemorySearch {
                query: Some("CAFE".into()),
                ..Default::default()
            })
            .unwrap()
            .memories[0]
            .details
            .memory_id,
        other.memory_id
    );
    let filter = json!({"query":"pool retry", "labels":["BACKEND", "incident"],
        "links":["https://EXAMPLE.COM/pull/42"], "instance":"review",
        "after":"2000-01-01T00:00:00Z", "before":"2100-01-01T00:00:00+00:00",
        "fields":{"decision":"Limit concurrency", "count":3, "optional":null,
            "findings":{"verified":true, "cause":"Retries exhaust the connection pool"}}});
    let results = store
        .search_memories(serde_json::from_value(filter.clone()).unwrap())
        .unwrap();
    assert_eq!(results.memories.len(), 1);
    assert_eq!(results.memories[0].details.memory_id, preferred.memory_id);
    for (key, value) in [
        ("labels", json!(["missing"])),
        ("links", json!(["https://example.com/pull/43"])),
        ("fields", json!({"count":"3"})),
        ("fields", json!({"missing":null})),
        ("after", json!("2100-01-01T00:00:00Z")),
    ] {
        let mut query = filter.clone();
        query[key] = value;
        if key == "after" {
            query.as_object_mut().unwrap().remove("before");
        }
        assert!(
            store
                .search_memories(serde_json::from_value(query).unwrap())
                .unwrap()
                .memories
                .is_empty(),
            "{key}"
        );
    }
    let page = store
        .search_memories(MemorySearch {
            limit: Some(1),
            ..Default::default()
        })
        .unwrap();
    assert!(page.has_more);
    assert_eq!(page.memories[0].details.memory_id, other.memory_id);
    let next = store
        .search_memories(MemorySearch {
            limit: Some(1),
            offset: Some(1),
            ..Default::default()
        })
        .unwrap();
    assert!(!next.has_more);
    assert_eq!(next.memories[0].details.memory_id, preferred.memory_id);
    for query in ["\" OR title:secret", "pool' UNION SELECT", "NOT"] {
        store
            .search_memories(MemorySearch {
                query: Some(query.into()),
                ..Default::default()
            })
            .unwrap();
    }
    let mut long_word = input("Long evidence");
    long_word.markdown = "a".repeat(16_384);
    store.remember("review", "three", None, long_word).unwrap();
    let results = store
        .search_memories(MemorySearch {
            query: Some("aaaa".into()),
            ..Default::default()
        })
        .unwrap();
    assert_eq!(results.memories.len(), 1);
    assert!(results.memories[0].snippet.chars().count() <= 480);
}

#[test]
fn conclusion_memory_is_atomic_immutable_and_reused_on_cleanup_retry_and_upgrade() {
    let (_root, config, store) = fixture();
    let input = input("Final evidence");
    let receipt = store
        .save_conclusion("review", "one", None, Some(&input))
        .unwrap();
    let memory_id = receipt.memory_id.unwrap();
    assert_eq!(
        store.memory(memory_id).unwrap().details.conclusion_id,
        Some(receipt.conclusion_id)
    );
    let retry = store
        .save_conclusion("review", "one", None, Some(&input))
        .unwrap();
    assert_eq!(retry.memory_id, Some(memory_id));
    assert!(
        store
            .save_conclusion(
                "review",
                "one",
                None,
                Some(&self::input("Conflicting evidence"))
            )
            .is_err()
    );
    assert_eq!(
        store
            .search_memories(MemorySearch::default())
            .unwrap()
            .memories
            .len(),
        1
    );
    store.events.connection().unwrap().execute_batch("DROP TRIGGER memories_insert; DROP TRIGGER memories_delete; DROP TABLE memory_search; DROP TABLE memories;").unwrap();
    let upgraded = RuleStore::open(&config).unwrap();
    let migrated = upgraded
        .search_memories(MemorySearch {
            query: Some("retr pool".into()),
            ..Default::default()
        })
        .unwrap();
    assert_eq!(migrated.memories.len(), 1);
    assert_eq!(
        upgraded
            .memory(migrated.memories[0].details.memory_id)
            .unwrap()
            .fields["count"],
        3
    );
    let reopened = RuleStore::open(&config).unwrap();
    assert_eq!(
        reopened
            .search_memories(MemorySearch::default())
            .unwrap()
            .memories
            .len(),
        1
    );
}

#[test]
fn invalid_memory_or_search_inputs_fail_before_storage() {
    let (_root, _config, store) = fixture();
    for value in [
        json!({"title":"x", "summary":"x"}),
        json!({"title":"", "summary":"x", "markdown":"evidence"}),
        json!({"title":"x", "summary":"x", "fields":{"unsafe":"nul\u{0000}"}}),
        json!({"title":"x", "summary":"x", "markdown":"x", "links":[{"label":"secret", "url":"https://user:password@example.com"}]}),
        json!({"title":"x", "summary":"x", "markdown":"x", "links":[{"label":"file", "url":"file:///etc/passwd"}]}),
        json!({"title":"x", "summary":"x", "markdown":"x".repeat(1_048_576)}),
    ] {
        assert!(
            store
                .remember(
                    "review",
                    "one",
                    None,
                    serde_json::from_value(value).unwrap()
                )
                .is_err()
        );
    }
    for value in [
        json!({"query":"!!!"}),
        json!({"query":" "}),
        json!({"limit":0}),
        json!({"limit":101}),
        json!({"offset":10001}),
        json!({"after":"yesterday"}),
        json!({"after":"2100-01-01T00:00:00Z", "before":"2000-01-01T00:00:00Z"}),
    ] {
        assert!(
            store
                .search_memories(serde_json::from_value(value).unwrap())
                .is_err()
        );
    }
    assert!(
        store
            .search_memories(MemorySearch::default())
            .unwrap()
            .memories
            .is_empty()
    );
    let mut deep = json!(true);
    for _ in 0..17 {
        deep = json!({"nested":deep});
    }
    let mut too_deep = input("Deep evidence");
    too_deep.fields.insert("deep".into(), deep);
    assert!(store.remember("review", "one", None, too_deep).is_err());
    let mut too_many = input("Large evidence");
    too_many
        .fields
        .insert("items".into(), json!(vec![true; 4096]));
    assert!(store.remember("review", "one", None, too_many).is_err());
}
