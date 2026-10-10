use super::*;
use crate::store::events::{Batch, Deletion, diagnostics::EvaluationOutcome};
use chrono::{DateTime, Utc};

fn now(ms: i64) -> DateTime<Utc> {
    DateTime::from_timestamp_millis(ms).unwrap()
}

fn definition(name: &str, seconds: u32, end: bool) -> Definition {
    serde_json::from_value(serde_json::json!({
        "name": name, "script": "fn matches(event) { true }", "template": "blank",
        "model": "openai/test", "initial_prompt": "Inspect {{event.summary}}",
        "enabled": true, "throttle_seconds": seconds, "trigger_at_end": end,
    }))
    .unwrap()
}

fn setup(end: bool) -> (tempfile::TempDir, Config, RuleStore, EventStore, String) {
    let home = tempfile::tempdir().unwrap();
    let config = Config::at(home.path().into(), "throttling".into(), 9876).unwrap();
    let rules = RuleStore::open(&config).unwrap();
    rules
        .save(definition("inspect", 10, end), None, "main".into())
        .unwrap();
    let events = EventStore::open(&config).unwrap();
    let token = events.register_provider("sample").unwrap();
    (home, config, rules, events, token)
}

fn ingest(events: &EventStore, token: &str, id: &str) -> i64 {
    events
        .ingest(
            token,
            Batch {
                events: vec![super::super::events::tests::event(id)],
            },
        )
        .unwrap()
        .receipts[0]
        .sequence
}

fn deadline(events: &EventStore, namespace: &str, name: &str) -> Option<i64> {
    events
        .connection()
        .unwrap()
        .query_row(
            "SELECT until_ms FROM rule_throttles WHERE namespace = ?1 AND rule_name = ?2",
            params![namespace, name],
            |row| row.get(0),
        )
        .optional()
        .unwrap()
}

#[test]
fn first_matches_own_fixed_evaluation_time_windows_in_both_modes() {
    for end in [false, true] {
        let (_home, config, rules, events, token) = setup(end);
        let first = ingest(&events, &token, "first");
        let second = ingest(&events, &token, "second");
        rules.evaluate_at(now(1000)).unwrap();
        assert_eq!(deadline(&events, &config.namespace, "inspect"), Some(11000));
        let snapshot = rules.snapshot().unwrap();
        assert_eq!(snapshot.acceptances.len(), usize::from(!end));
        assert!(snapshot.evaluation_errors.is_empty());
        assert_eq!(snapshot.evaluation_warnings.len(), 1);
        let first_diagnostics = events.diagnostics(first).unwrap();
        assert_eq!(
            first_diagnostics.attempts[0].rules[0].outcome,
            if end {
                EvaluationOutcome::Deferred
            } else {
                EvaluationOutcome::Matched
            }
        );
        let warning = events.diagnostics(second).unwrap();
        assert_eq!(warning.counts.errors, 0);
        assert_eq!(warning.counts.warnings, 1);
        assert_eq!(
            warning.attempts[0].rules[0].outcome,
            EvaluationOutcome::Throttled
        );
        assert!(warning.attempts[0].rules[0].error.is_none());
        assert_eq!(
            warning.attempts[0].rules[0].warning.as_deref(),
            Some("Rule acceptance throttled until 11000 ms since Unix epoch.")
        );
        assert!(warning.attempts[0].rules[0].acceptance.is_none());
        assert!(events.snapshot().unwrap().diagnostic_counts[&second].has_issues());
        let third = ingest(&events, &token, "third");
        rules.evaluate_at(now(10999)).unwrap();
        assert_eq!(events.diagnostics(third).unwrap().counts.warnings, 1);
        assert_eq!(deadline(&events, &config.namespace, "inspect"), Some(11000));
        let boundary = ingest(&events, &token, "boundary");
        rules.evaluate_at(now(11000)).unwrap();
        let snapshot = rules.snapshot().unwrap();
        assert_eq!(deadline(&events, &config.namespace, "inspect"), Some(21000));
        assert_eq!(snapshot.acceptances.len(), if end { 1 } else { 2 });
        assert!(
            snapshot
                .acceptances
                .iter()
                .any(|row| row.event_sequence == first)
        );
        assert_eq!(events.diagnostics(boundary).unwrap().counts.warnings, 0);
        rules.evaluate_at(now(21000)).unwrap();
        let snapshot = rules.snapshot().unwrap();
        assert_eq!(snapshot.acceptances.len(), 2);
        assert!(
            snapshot
                .acceptances
                .iter()
                .any(|row| row.event_sequence == boundary)
        );
        assert!(
            !snapshot
                .acceptances
                .iter()
                .any(|row| row.event_sequence == second || row.event_sequence == third)
        );
    }
}

#[test]
fn deferred_work_recovers_with_pinned_configuration_after_edit_deactivation_or_removal() {
    for change in ["edit", "deactivate", "remove"] {
        let (_home, config, rules, events, token) = setup(true);
        let first = ingest(&events, &token, "first");
        rules.evaluate_at(now(1000)).unwrap();
        if change == "remove" {
            std::fs::remove_dir_all(config.home.join("templates/rules/inspect")).unwrap();
            assert!(rules.snapshot().unwrap().rules.is_empty());
        } else {
            let mut changed = definition("inspect", 0, false);
            changed.script = "fn matches(event) { false }".into();
            changed.initial_prompt = "Changed prompt".into();
            changed.enabled = change != "deactivate";
            rules.save(changed, Some(1), "other".into()).unwrap();
        }
        let recovered = RuleStore::open(&config).unwrap();
        recovered.evaluate_at(now(10999)).unwrap();
        assert!(recovered.snapshot().unwrap().acceptances.is_empty());
        recovered.evaluate_at(now(11000)).unwrap();
        recovered.evaluate_at(now(12000)).unwrap();
        let snapshot = recovered.snapshot().unwrap();
        assert_eq!(snapshot.acceptances.len(), 1);
        let accepted = &snapshot.acceptances[0];
        assert_eq!(accepted.event_sequence, first);
        assert_eq!(accepted.rule_revision, 1);
        assert_eq!(accepted.rule.zellij_session, "main");
        assert_eq!(accepted.rule.definition.throttle_seconds, 10);
        assert!(accepted.rule.definition.trigger_at_end);
        assert_ne!(accepted.resolved_prompt.as_deref(), Some("Changed prompt"));
        assert_eq!(deadline(&events, &config.namespace, "inspect"), Some(11000));
    }
}

#[test]
fn replay_accepts_immediately_without_creating_or_mutating_live_windows() {
    for end in [false, true] {
        let (_home, config, rules, events, token) = setup(end);
        let mut disabled = definition("inspect", 10, end);
        disabled.enabled = false;
        rules.save(disabled, Some(1), "main".into()).unwrap();
        let retained = ingest(&events, &token, "retained");
        rules
            .save(definition("inspect", 10, end), Some(2), "main".into())
            .unwrap();
        events.replay(retained, "before-live").unwrap();
        rules.evaluate_at(now(1000)).unwrap();
        assert_eq!(rules.snapshot().unwrap().acceptances.len(), 1);
        assert_eq!(deadline(&events, &config.namespace, "inspect"), None);
        let live = ingest(&events, &token, "live");
        rules.evaluate_at(now(2000)).unwrap();
        assert_eq!(deadline(&events, &config.namespace, "inspect"), Some(12000));
        let replay = events.replay(live, "inside-live").unwrap();
        rules.evaluate_at(now(11999)).unwrap();
        assert!(
            rules
                .snapshot()
                .unwrap()
                .acceptances
                .iter()
                .any(|row| row.attempt_id == replay)
        );
        assert_eq!(deadline(&events, &config.namespace, "inspect"), Some(12000));
        events.replay(retained, "after-live").unwrap();
        rules.evaluate_at(now(13000)).unwrap();
        assert_eq!(deadline(&events, &config.namespace, "inspect"), Some(12000));
        assert_eq!(rules.snapshot().unwrap().acceptances.len(), 4);
        assert_eq!(events.snapshot().unwrap().accepted_attempts, Some(4));
    }
}

#[test]
fn rule_and_namespace_windows_are_independent_and_survive_restart() {
    let (_home, config, rules, events, token) = setup(false);
    ingest(&events, &token, "first");
    rules.evaluate_at(now(1000)).unwrap();
    rules
        .save(definition("other", 10, false), None, "main".into())
        .unwrap();
    ingest(&events, &token, "second");
    RuleStore::open(&config)
        .unwrap()
        .evaluate_at(now(2000))
        .unwrap();
    assert_eq!(deadline(&events, &config.namespace, "inspect"), Some(11000));
    assert_eq!(deadline(&events, &config.namespace, "other"), Some(12000));
    assert_eq!(rules.snapshot().unwrap().acceptances.len(), 2);
    let mut other_config = config.clone();
    other_config.namespace = "other".into();
    let other_rules = RuleStore::open(&other_config).unwrap();
    for rule in other_rules.snapshot().unwrap().rules {
        let mut enabled = rule.definition;
        enabled.enabled = true;
        other_rules
            .save(enabled, Some(rule.revision), "main".into())
            .unwrap();
    }
    let other_events = EventStore::open(&other_config).unwrap();
    let other_token = other_events.register_provider("sample").unwrap();
    ingest(&other_events, &other_token, "first");
    other_rules.evaluate_at(now(3000)).unwrap();
    assert_eq!(other_rules.snapshot().unwrap().acceptances.len(), 2);
    assert_eq!(deadline(&events, "other", "inspect"), Some(13000));
    assert_eq!(deadline(&events, &config.namespace, "inspect"), Some(11000));
}

#[test]
fn configuration_edits_preserve_active_deadlines_and_apply_to_the_next_window() {
    let (_home, config, rules, events, token) = setup(false);
    ingest(&events, &token, "first");
    rules.evaluate_at(now(1000)).unwrap();
    rules
        .save(definition("inspect", 20, true), Some(1), "main".into())
        .unwrap();
    let rejected = ingest(&events, &token, "rejected");
    rules.evaluate_at(now(5000)).unwrap();
    assert_eq!(deadline(&events, &config.namespace, "inspect"), Some(11000));
    assert_eq!(events.diagnostics(rejected).unwrap().counts.warnings, 1);
    let next = ingest(&events, &token, "next");
    rules.evaluate_at(now(11000)).unwrap();
    assert_eq!(deadline(&events, &config.namespace, "inspect"), Some(31000));
    assert_eq!(rules.snapshot().unwrap().acceptances.len(), 1);
    rules.evaluate_at(now(31000)).unwrap();
    let accepted = rules.snapshot().unwrap().acceptances.remove(0);
    assert_eq!(accepted.event_sequence, next);
    assert_eq!(accepted.rule_revision, 2);
}

#[test]
fn concurrent_evaluators_admit_one_live_match_per_window() {
    for end in [false, true] {
        let (_home, _config, rules, events, token) = setup(end);
        for id in 0..20 {
            ingest(&events, &token, &format!("event-{id}"));
        }
        let barrier = std::sync::Barrier::new(2);
        std::thread::scope(|scope| {
            for _ in 0..2 {
                let rules = &rules;
                let barrier = &barrier;
                scope.spawn(move || {
                    barrier.wait();
                    rules.evaluate_at(now(1000)).unwrap();
                });
            }
        });
        let snapshot = rules.snapshot().unwrap();
        assert_eq!(snapshot.acceptances.len(), usize::from(!end));
        assert_eq!(snapshot.evaluation_warnings.len(), 19);
        rules.evaluate_at(now(11000)).unwrap();
        assert_eq!(rules.snapshot().unwrap().acceptances.len(), 1);
    }
}

#[test]
fn history_deletion_preserves_windows_and_cancels_deferred_actions() {
    for (end, deletion) in [
        (false, "acceptance"),
        (false, "event"),
        (true, "event"),
        (true, "ignored"),
        (true, "provider"),
    ] {
        let (_home, config, rules, events, mut token) = setup(end);
        let sequence = ingest(&events, &token, "first");
        rules.evaluate_at(now(1000)).unwrap();
        match deletion {
            "acceptance" => {
                let id = rules.snapshot().unwrap().acceptances[0].id;
                events.delete(Deletion::Acceptance(id)).unwrap();
            }
            "event" => {
                events.delete(Deletion::Event(sequence)).unwrap();
            }
            "ignored" => {
                events.delete(Deletion::Ignored).unwrap();
            }
            "provider" => {
                events
                    .connection()
                    .unwrap()
                    .execute_batch(include_str!("../../../../migrations/0007_providers.sql"))
                    .unwrap();
                events.begin_provider_deletion("sample", "sample").unwrap();
                events.delete_provider("sample", "sample").unwrap();
                token = events.register_provider("sample").unwrap();
            }
            _ => unreachable!(),
        }
        assert_eq!(deadline(&events, &config.namespace, "inspect"), Some(11000));
        let second = ingest(&events, &token, "second");
        let recovered = RuleStore::open(&config).unwrap();
        recovered.evaluate_at(now(10999)).unwrap();
        assert_eq!(events.diagnostics(second).unwrap().counts.warnings, 1);
        recovered.evaluate_at(now(11000)).unwrap();
        assert!(recovered.snapshot().unwrap().acceptances.is_empty());
        ingest(&events, &token, "boundary");
        recovered.evaluate_at(now(11000)).unwrap();
        recovered.evaluate_at(now(21000)).unwrap();
        assert_eq!(recovered.snapshot().unwrap().acceptances.len(), 1);
    }
}

#[test]
fn nonmatches_and_predicate_errors_do_not_consume_windows_and_zero_ignores_end_mode() {
    let (_home, config, rules, events, token) = setup(true);
    let mut selective = definition("inspect", 10, true);
    selective.script = "fn matches(event) { if event.event_id == \"error\" { event.missing.value == 1 } else { event.event_id == \"match\" } }".into();
    rules.save(selective, Some(1), "main".into()).unwrap();
    let no_match = ingest(&events, &token, "skip");
    let error = ingest(&events, &token, "error");
    rules.evaluate_at(now(1000)).unwrap();
    assert_eq!(deadline(&events, &config.namespace, "inspect"), None);
    assert_eq!(
        events.diagnostics(no_match).unwrap().attempts[0].rules[0].outcome,
        EvaluationOutcome::NoMatch
    );
    assert_eq!(events.diagnostics(error).unwrap().counts.errors, 1);
    let matched = ingest(&events, &token, "match");
    rules.evaluate_at(now(2000)).unwrap();
    assert_eq!(deadline(&events, &config.namespace, "inspect"), Some(12000));
    rules
        .save(definition("zero", 0, true), None, "main".into())
        .unwrap();
    ingest(&events, &token, "zero-one");
    ingest(&events, &token, "zero-two");
    rules.evaluate_at(now(3000)).unwrap();
    assert_eq!(deadline(&events, &config.namespace, "zero"), None);
    assert_eq!(deadline(&events, &config.namespace, "inspect"), Some(12000));
    assert_eq!(rules.snapshot().unwrap().acceptances.len(), 2);
    rules.evaluate_at(now(12000)).unwrap();
    assert!(
        rules
            .snapshot()
            .unwrap()
            .acceptances
            .iter()
            .any(|row| row.event_sequence == matched)
    );
}

#[test]
fn future_deferred_work_does_not_starve_new_evaluations() {
    let home = tempfile::tempdir().unwrap();
    let config = Config::at(home.path().into(), "starvation".into(), 9876).unwrap();
    let rules = RuleStore::open(&config).unwrap();
    for index in 0..40 {
        rules
            .save(
                definition(&format!("deferred-{index:02}"), 10, true),
                None,
                "main".into(),
            )
            .unwrap();
    }
    let events = EventStore::open(&config).unwrap();
    let token = events.register_provider("sample").unwrap();
    let first = ingest(&events, &token, "first");
    rules.evaluate_at(now(1000)).unwrap();
    rules.evaluate_at(now(1000)).unwrap();
    assert_eq!(
        events.diagnostics(first).unwrap().attempts[0]
            .rules
            .iter()
            .filter(|rule| rule.outcome == EvaluationOutcome::Deferred)
            .count(),
        40
    );
    rules
        .save(definition("instant", 0, false), None, "main".into())
        .unwrap();
    let second = ingest(&events, &token, "second");
    rules.evaluate_at(now(2000)).unwrap();
    rules.evaluate_at(now(2000)).unwrap();
    let snapshot = rules.snapshot().unwrap();
    assert_eq!(snapshot.acceptances.len(), 1);
    assert_eq!(snapshot.acceptances[0].event_sequence, second);
    assert_eq!(snapshot.acceptances[0].rule_name, "instant");
    assert_eq!(snapshot.evaluation_warnings.len(), 40);
    rules.evaluate_at(now(11000)).unwrap();
    rules.evaluate_at(now(11000)).unwrap();
    assert_eq!(rules.snapshot().unwrap().acceptances.len(), 41);
}

#[test]
fn schema_upgrade_preserves_history_and_defaults_legacy_pinned_rules_to_no_throttle() {
    let (_home, config, rules, events, token) = setup(false);
    rules
        .save(definition("inspect", 0, false), Some(1), "main".into())
        .unwrap();
    let first = ingest(&events, &token, "first");
    rules.evaluate_at(now(1000)).unwrap();
    let before = rules.snapshot().unwrap();
    let history = events.snapshot().unwrap();
    events.replay(first, "legacy-pending").unwrap();
    events.connection().unwrap().execute_batch(
        "UPDATE event_rules SET definition = json_remove(definition, '$.throttle_seconds', '$.trigger_at_end');
         UPDATE rule_evaluations SET rule_snapshot = json_remove(rule_snapshot, '$.definition.throttle_seconds', '$.definition.trigger_at_end');
         UPDATE rule_acceptances SET payload = json_remove(payload, '$.rule.definition.throttle_seconds', '$.rule.definition.trigger_at_end');
         DROP INDEX rule_evaluations_due;
         ALTER TABLE rule_evaluations DROP COLUMN deadline_ms;
         ALTER TABLE rule_evaluations DROP COLUMN warning;
         DROP TABLE rule_throttles;",
    ).unwrap();
    let recovered = RuleStore::open(&config).unwrap();
    assert_eq!(recovered.snapshot().unwrap(), before);
    assert_eq!(
        events.snapshot().unwrap().records[0].event,
        history.records[0].event
    );
    recovered.evaluate_at(now(1000)).unwrap();
    let after = recovered.snapshot().unwrap();
    assert_eq!(after.acceptances.len(), 2);
    assert!(after.acceptances.contains(&before.acceptances[0]));
    assert!(
        after
            .acceptances
            .iter()
            .all(|row| row.rule.definition.throttle_seconds == 0)
    );
    assert_eq!(deadline(&events, &config.namespace, "inspect"), None);
}

#[test]
fn maximum_typed_seconds_has_an_exact_safe_deadline_and_unknown_outcomes_fail_closed() {
    let (_home, config, rules, events, token) = setup(true);
    rules
        .save(
            definition("inspect", u32::MAX, true),
            Some(1),
            "main".into(),
        )
        .unwrap();
    let sequence = ingest(&events, &token, "maximum");
    rules.evaluate_at(now(1000)).unwrap();
    assert_eq!(
        deadline(&events, &config.namespace, "inspect"),
        Some(1000 + i64::from(u32::MAX) * 1000)
    );
    events
        .connection()
        .unwrap()
        .execute("UPDATE rule_evaluations SET outcome = 'unexpected'", [])
        .unwrap();
    assert!(
        matches!(events.diagnostics(sequence), Err(Error::Storage(message)) if message == "unknown evaluation outcome: unexpected")
    );
}
