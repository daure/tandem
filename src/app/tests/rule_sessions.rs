use super::*;
use crate::store::{
    opencode::{PromptOutcome, Session, Snapshot},
    rules::DeliveryMode,
};

#[test]
fn named_session_fields_autosave_and_rule_rows_show_the_target_and_delivery_mode() {
    let mut app = editor(false);
    show_settings(&mut app);
    let keys = |value: &str| {
        std::iter::once(Key::Enter)
            .chain(value.chars().map(Key::Char))
            .chain(std::iter::once(Key::Esc))
            .collect::<Vec<_>>()
    };
    settings_input(&mut app, "target-instance", &keys("shared"));
    finish_autosaves(&mut app);
    settings_input(&mut app, "session-name", &keys("triage"));
    finish_autosaves(&mut app);
    for expected in [DeliveryMode::Steer, DeliveryMode::Interrupt] {
        settings_input(&mut app, "delivery-mode", &[Key::Right]);
        finish_autosaves(&mut app);
        let saved = saved_rule(&app);
        assert_eq!(saved.definition.target_instance.as_deref(), Some("shared"));
        assert_eq!(saved.definition.session_name.as_deref(), Some("triage"));
        assert_eq!(saved.definition.delivery_mode, expected);
        assert!(!saved.definition.enabled);
    }
    let saved = saved_rule(&app);
    app.handle_message(Msg::Close, &mut EventCtx::default());
    show_rules(&mut app, vec![saved]);
    let text = render(&mut app, 130).1;
    assert!(text.contains("shared/triage · Interrupt"), "{text}");
}

#[test]
fn named_session_acceptances_show_delivery_and_only_their_linked_conversation() {
    init_ui();
    let mut rule = rule("triage");
    rule.definition.template = "other".into();
    rule.definition.target_instance = Some("review".into());
    rule.definition.session_name = Some("PR triage".into());
    let mut acceptance = acceptance(rule, 1, 1);
    acceptance.instance = "review".into();
    acceptance.session_id = Some("ses_shared".into());
    acceptance.session_server = Some("http://127.0.0.1:4199".into());
    acceptance.prompt_outcome = Some(PromptOutcome::Queued);
    let session = Session {
        id: "ses_shared".into(),
        title: "User title".into(),
        server: "http://127.0.0.1:4199".into(),
        directory: "/tmp/workspaces/review".into(),
        ..Default::default()
    };
    let context = crate::app::acceptances::Context {
        inventory: snapshot(),
        opencode: Snapshot {
            sessions: vec![
                session.clone(),
                Session {
                    id: "ses_unrelated".into(),
                    ..session.clone()
                },
                Session {
                    server: "http://127.0.0.1:4200".into(),
                    ..session
                },
            ],
            ..Default::default()
        },
    };
    let target = crate::app::acceptances::Target::new(&acceptance, None, &context, true);
    assert!(target.instance.is_some());
    assert_eq!(target.conversations.len(), 1);
    let text = target.text("", None).to_string();
    assert!(text.contains("Session PR triage · Queue"), "{text}");
    assert!(text.contains("Prompt Queued"), "{text}");
    let mut app = crate::app::root(AppService::for_tests());
    app.service
        .set_opencode_snapshot_for_tests(context.opencode);
    app.update_snapshot(context.inventory);
    app.pages_mut().update_rules(crate::store::rules::Snapshot {
        acceptances: vec![acceptance.clone()],
        ..Default::default()
    });
    app.pages_mut()
        .update_events(crate::store::events::Snapshot {
            records: vec![crate::store::events::Record {
                sequence: 1,
                provider: "sample".into(),
                received_at: "now".into(),
                event: crate::environments::events::tests::event("triage"),
                attempts: vec![],
                acceptances: vec![acceptance],
            }],
            total: 1,
            ..Default::default()
        });
    app.tabs_mut().select_index(1);
    app.after_event(&mut EventCtx::default());
    let (layout, _) = render(&mut app, 160);
    let focus = layout
        .focus_targets()
        .iter()
        .find(|target| target.id.as_str() == crate::app::events::FOCUS)
        .unwrap();
    app.dispatch_focus(focus, true, &mut tuicore::FocusCtx::default());
    app.dispatch_event(
        &EventRoute::new(focus.path.clone()),
        &TuiEvent::Key(Key::Right.into()),
        &mut EventCtx::default(),
    );
    let text = render(&mut app, 160).1;
    assert!(text.contains("Session PR triage · Queue"), "{text}");
    assert!(text.contains("review"), "{text}");
    assert!(!text.contains("instance not created"), "{text}");
}

#[test]
fn event_expressions_and_follow_up_input_autosave_with_inline_prompt_validation() {
    let mut app = editor(false);
    show_settings(&mut app);
    for (slot, length, source) in [
        ("template", "website".len(), "{{event.metadata.template}}"),
        ("target-instance", 0, "shared-{{event.metadata.ticket}}"),
        ("model", "openai/test".len(), "{{event.metadata.model}}"),
        ("variant", 0, "{{event.metadata.variant}}"),
        ("follow-up-prompt", 0, "Continue with {{event.data.text}}"),
    ] {
        let keys = std::iter::once(Key::Enter)
            .chain(std::iter::once(Key::Home))
            .chain(std::iter::repeat_n(Key::Delete, length))
            .chain(source.chars().map(Key::Char))
            .chain(std::iter::once(Key::Esc))
            .collect::<Vec<_>>();
        settings_input(&mut app, slot, &keys);
        finish_autosaves(&mut app);
    }
    let saved = saved_rule(&app);
    assert_eq!(saved.definition.template, "{{event.metadata.template}}");
    assert_eq!(
        saved.definition.target_instance.as_deref(),
        Some("shared-{{event.metadata.ticket}}")
    );
    assert_eq!(saved.definition.model, "{{event.metadata.model}}");
    assert_eq!(
        saved.definition.variant.as_deref(),
        Some("{{event.metadata.variant}}")
    );
    assert_eq!(
        saved.definition.follow_up_prompt.as_deref(),
        Some("Continue with {{event.data.text}}")
    );
    settings_input(
        &mut app,
        "follow-up-prompt",
        &[Key::Enter, Key::End, Key::Char('{'), Key::Char('{')],
    );
    finish_autosaves(&mut app);
    assert!(
        app.rule_editor
            .as_ref()
            .unwrap()
            .borrow()
            .follow_up_prompt_error
            .is_some()
    );
    assert!(
        app.service
            .validate_rule_prompt(
                saved_rule(&app)
                    .definition
                    .follow_up_prompt
                    .as_deref()
                    .unwrap()
            )
            .is_ok()
    );
    settings_input(
        &mut app,
        "follow-up-prompt",
        &[Key::Enter, Key::End, Key::Backspace, Key::Backspace],
    );
    finish_autosaves(&mut app);
    assert!(
        app.rule_editor
            .as_ref()
            .unwrap()
            .borrow()
            .follow_up_prompt_error
            .is_none()
    );
    assert_eq!(
        saved_rule(&app).definition.follow_up_prompt,
        saved.definition.follow_up_prompt
    );
}
