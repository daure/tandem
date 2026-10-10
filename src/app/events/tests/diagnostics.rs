use super::*;
use crate::store::events::diagnostics::{AttemptDiagnostics, Counts, RuleDiagnostics};

#[test]
fn diagnostics_pane_renders_failures_sanitizes_text_and_preserves_the_open_snapshot() {
    crate::app::tests::init_ui();
    let service = AppService::for_tests();
    let mut pane = Pane::new(service, 42);
    let rule = serde_json::from_value(serde_json::json!({
        "definition":{"name":"inspect", "script":"fn matches(event) { true }", "template":"blank",
            "model":"openai/test", "initial_prompt":"Inspect", "enabled":true},
        "revision":7, "zellij_session":"main",
    }))
    .unwrap();
    let mut diagnostics = Diagnostics {
        sequence: 42,
        event_id: "message-42".into(),
        provider: "sample".into(),
        summary: "Inspect message".into(),
        counts: Counts {
            errors: 1,
            warnings: 1,
        },
        attempts_total: 1,
        attempts_truncated: false,
        attempts: vec![AttemptDiagnostics {
            attempt: crate::store::events::Attempt {
                id: 91,
                status: crate::store::events::ProcessingStatus::Pending,
                replay: true,
                created_at: "2026-10-10T12:00:00Z".into(),
                accepted_at: None,
            },
            rules: vec![RuleDiagnostics {
                rule,
                outcome: EvaluationOutcome::Failed,
                error: Some("missing field\u{1b}\u{202e}".into()),
                warning: None,
                acceptance: None,
                startup: None,
                details_unavailable: None,
            }],
        }],
    };
    let mut deferred = diagnostics.attempts[0].rules[0].clone();
    deferred.outcome = EvaluationOutcome::Deferred;
    deferred.error = None;
    let mut throttled = deferred.clone();
    throttled.outcome = EvaluationOutcome::Throttled;
    throttled.warning = Some("Fixed timer active; match rejected".into());
    diagnostics.attempts[0].rules.extend([deferred, throttled]);
    for expected_change in [true, false] {
        let (sender, receiver) = oneshot::channel();
        pane.receiver = Some(receiver);
        sender.send(Ok(diagnostics.clone())).unwrap();
        assert_eq!(pane.poll(), expected_change);
    }
    let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(90, 30)).unwrap();
    let area = Rect::new(0, 0, 90, 30);
    pane.layout(area, &mut LayoutCtx::default());
    pane.tick(Duration::ZERO, AnimationSettings::default());
    terminal
        .draw(|frame| pane.render(frame, area, &mut RenderCtx::new()))
        .unwrap();
    let text = terminal
        .backend()
        .buffer()
        .content()
        .iter()
        .map(|cell| cell.symbol())
        .collect::<String>();
    assert!(text.contains("Attempt #91 (replay)"), "{text}");
    assert!(
        text.contains("inspect · revision 7 · evaluation failed"),
        "{text}"
    );
    assert!(text.contains("Error: missing field"), "{text}");
    assert!(
        text.contains("pending: waiting for throttle timer"),
        "{text}"
    );
    assert!(
        text.contains("warning: throttled (match rejected)"),
        "{text}"
    );
    assert!(
        text.contains("Warning: Fixed timer active; match rejected"),
        "{text}"
    );
    assert!(!text.contains('\u{1b}') && !text.contains('\u{202e}'));
    assert!(pane.receiver.is_none());
    pane.tick(Duration::from_secs(1), AnimationSettings::default());
    assert!(pane.receiver.is_none());
    let (sender, receiver) = oneshot::channel();
    pane.receiver = Some(receiver);
    sender.send(Err(Error::NotFound)).unwrap();
    assert!(pane.poll());
    assert!(pane.text.contains("Cannot read event diagnostics"));
}
