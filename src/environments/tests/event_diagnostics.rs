use super::*;
use crate::{
    environments::{
        rules::RuleStore,
        runtime_db::{self, Kind},
    },
    store::{
        events::diagnostics::*,
        rules::{Definition, DispatchStatus},
    },
};

fn definition(name: &str, script: &str) -> Definition {
    serde_json::from_value(json!({
        "name": name, "script": script, "template": "blank", "model": "openai/test",
        "initial_prompt": "Inspect {{event.summary}}", "enabled": true,
    }))
    .unwrap()
}

#[test]
fn diagnostics_preserve_pinned_outcomes_across_replay_and_acceptance_deletion() {
    let (_home, config, events, token) = setup();
    let rules = RuleStore::open(&config).unwrap();
    for (name, script) in [
        ("match", "fn matches(event) { true }"),
        ("skip", "fn matches(event) { false }"),
        ("error", "fn matches(event) { event.missing.value == 1 }"),
        ("success", "fn matches(event) { true }"),
    ] {
        rules
            .save(definition(name, script), None, "main".into())
            .unwrap();
    }
    let sequence = events
        .ingest(
            &token,
            Batch {
                events: vec![event("diagnose")],
            },
        )
        .unwrap()
        .receipts[0]
        .sequence;
    let pending = events.diagnostics(sequence).unwrap();
    assert!(
        pending.attempts[0]
            .rules
            .iter()
            .all(|rule| rule.outcome == EvaluationOutcome::Pending)
    );
    rules.evaluate().unwrap();
    let snapshot = rules.snapshot().unwrap();
    let evaluation_error = snapshot.evaluation_errors[0].error.clone();
    let mut acceptance = snapshot
        .acceptances
        .iter()
        .find(|row| row.rule_name == "match")
        .unwrap()
        .clone();
    let mut successful = snapshot
        .acceptances
        .iter()
        .find(|row| row.rule_name == "success")
        .unwrap()
        .clone();
    successful.status = DispatchStatus::Launched;
    successful.session_id = Some("session-success".into());
    rules.update(&successful, false).unwrap();
    acceptance.status = DispatchStatus::Failed;
    acceptance.error = Some("template provisioning failed".into());
    rules.update(&acceptance, false).unwrap();
    rules
        .save(
            definition("match", "fn matches(event) { false }"),
            Some(1),
            "main".into(),
        )
        .unwrap();
    let replay = events.replay(sequence, "diagnostic-replay").unwrap();
    rules.evaluate().unwrap();
    let before = events.snapshot().unwrap();
    let notifications = events.notifications(&token).unwrap().len();
    let diagnostics = events.diagnostics(sequence).unwrap();
    assert_eq!(diagnostics.attempts_total, 2);
    assert!(!diagnostics.attempts_truncated);
    assert_eq!(diagnostics.counts.errors, 3);
    assert_eq!(diagnostics.counts.warnings, 0);
    assert_eq!(diagnostics.attempts[0].attempt.id, replay);
    let historical = &diagnostics.attempts[1].rules;
    assert_eq!(historical[0].outcome, EvaluationOutcome::Failed);
    assert_eq!(
        historical[0].error.as_deref(),
        Some(evaluation_error.as_str())
    );
    assert_eq!(historical[1].rule.revision, 1);
    assert_eq!(
        historical[1].rule.definition.script,
        "fn matches(event) { true }"
    );
    assert_eq!(historical[1].outcome, EvaluationOutcome::Matched);
    assert_eq!(historical[1].acceptance.as_ref().unwrap(), &acceptance);
    assert_eq!(historical[2].outcome, EvaluationOutcome::NoMatch);
    assert_eq!(historical[3].acceptance.as_ref().unwrap(), &successful);
    assert_eq!(diagnostics.attempts[0].rules[1].rule.revision, 2);
    assert_eq!(
        diagnostics.attempts[0].rules[1].outcome,
        EvaluationOutcome::NoMatch
    );
    assert_eq!(events.snapshot().unwrap(), before);
    assert_eq!(events.notifications(&token).unwrap().len(), notifications);
    events.delete(Deletion::Acceptance(acceptance.id)).unwrap();
    let diagnostics = events.diagnostics(sequence).unwrap();
    let matched = &diagnostics.attempts[1].rules[1];
    assert_eq!(matched.outcome, EvaluationOutcome::Matched);
    assert!(matched.acceptance.is_none());
    assert!(
        matched
            .details_unavailable
            .as_ref()
            .unwrap()
            .contains("no retained acceptance")
    );
    let mut other = config.clone();
    other.namespace = "other".into();
    assert!(matches!(
        EventStore::open(&other).unwrap().diagnostics(sequence),
        Err(Error::NotFound)
    ));
}

#[test]
fn diagnostics_read_events_outside_the_feed_and_disclose_attempt_truncation() {
    let (_home, _config, events, token) = setup();
    let sequence = events
        .ingest(
            &token,
            Batch {
                events: vec![event("oldest")],
            },
        )
        .unwrap()
        .receipts[0]
        .sequence;
    for batch in 0..3 {
        events
            .ingest(
                &token,
                Batch {
                    events: (0..100)
                        .map(|index| event(&format!("{batch}-{index}")))
                        .collect(),
                },
            )
            .unwrap();
    }
    for index in 0..55 {
        events.replay(sequence, &format!("replay-{index}")).unwrap();
    }
    assert!(
        !events
            .snapshot()
            .unwrap()
            .records
            .iter()
            .any(|event| event.sequence == sequence)
    );
    let diagnostics = events.diagnostics(sequence).unwrap();
    assert_eq!(diagnostics.event_id, "oldest");
    assert_eq!(diagnostics.attempts_total, 56);
    assert_eq!(diagnostics.attempts.len(), ATTEMPT_LIMIT);
    assert!(diagnostics.attempts_truncated);
    assert!(
        diagnostics
            .attempts
            .iter()
            .all(|attempt| attempt.rules.is_empty())
    );
}

#[test]
fn diagnostics_use_only_lineage_verified_startup_logs_with_bounded_tails() {
    let (_home, config, events, token) = setup();
    let rules = RuleStore::open(&config).unwrap();
    rules
        .save(
            definition("inspect", "fn matches(event) { true }"),
            None,
            "main".into(),
        )
        .unwrap();
    let sequence = events
        .ingest(
            &token,
            Batch {
                events: vec![event("logs")],
            },
        )
        .unwrap()
        .receipts[0]
        .sequence;
    rules.evaluate().unwrap();
    let mut acceptance = rules.snapshot().unwrap().acceptances.remove(0);
    acceptance.operation_id = Some("1-1".into());
    acceptance.status = DispatchStatus::Uncertain;
    acceptance.error = Some("launch outcome unknown".into());
    rules.update(&acceptance, false).unwrap();
    let mut record = json!({
        "operation": {"id":"2-2", "name":acceptance.instance, "template":"blank",
            "action":"create_instance", "service":null, "state":"failed", "instance":null,
            "elapsed_seconds":1, "elapsed_milliseconds":1000,
            "error":"readiness failed", "warnings":["partial observation"],
            "progress": (0..120).map(|index| format!("step-{index}")).collect::<Vec<_>>()},
        "origin_operation_id":"1-1", "description":null, "branch_instances":false,
        "kind":"cold", "started_at":0, "timeout":60, "owner_pid":0, "workspace_ready":false, "services":[],
    });
    let save = |record: &serde_json::Value| {
        runtime_db::save(
            &config,
            &acceptance.instance,
            Kind::Startup,
            &record.to_string(),
        )
        .unwrap()
    };
    save(&record);
    let diagnostics = events.diagnostics(sequence).unwrap();
    assert_eq!(diagnostics.counts.warnings, 1);
    let startup = diagnostics.attempts[0].rules[0].startup.as_ref().unwrap();
    assert_eq!(startup.operation_id, "2-2");
    assert_eq!(startup.error.as_deref(), Some("readiness failed"));
    assert_eq!(startup.warnings, vec!["partial observation"]);
    assert_eq!(startup.progress.len(), LOG_LINE_LIMIT);
    assert_eq!(startup.progress[0], "step-20");
    assert_eq!(startup.progress.last().unwrap(), "step-119");
    assert!(startup.logs_truncated);
    record["operation"]["progress"] = json!(["界".repeat(LOG_BYTE_LIMIT)]);
    save(&record);
    let diagnostics = events.diagnostics(sequence).unwrap();
    let startup = diagnostics.attempts[0].rules[0].startup.as_ref().unwrap();
    assert!(startup.logs_truncated);
    assert!(
        startup
            .progress
            .iter()
            .chain(&startup.warnings)
            .map(|line| line.len() + 1)
            .sum::<usize>()
            <= LOG_BYTE_LIMIT
    );
    assert!(!startup.progress[0].is_empty());
    for (field, value) in [("origin_operation_id", "9-9"), ("template", "other")] {
        let mut foreign = record.clone();
        if field == "template" {
            foreign["operation"][field] = json!(value);
        } else {
            foreign[field] = json!(value);
        }
        save(&foreign);
        let diagnostics = events.diagnostics(sequence).unwrap();
        let rule = &diagnostics.attempts[0].rules[0];
        assert!(rule.startup.is_none());
        assert!(
            rule.details_unavailable
                .as_ref()
                .unwrap()
                .contains("recorded lineage")
        );
    }
}
