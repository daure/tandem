use super::*;

fn throttle_event(app: &mut App, event: TuiEvent) {
    let (layout, _) = render(app, 130);
    let target = layout
        .focus_targets()
        .iter()
        .find(|target| {
            target
                .path
                .keys()
                .iter()
                .any(|key| key.as_str() == "throttle-seconds")
        })
        .unwrap();
    app.dispatch_focus(target, true, &mut tuicore::FocusCtx::default());
    let mut ctx = EventCtx::default();
    app.dispatch_event(&EventRoute::new(target.path.clone()), &event, &mut ctx);
    for message in ctx.drain_messages() {
        app.handle_message(message, &mut EventCtx::default());
    }
}

fn replace_throttle(app: &mut App, value: &str) {
    let length = app
        .rule_editor
        .as_ref()
        .unwrap()
        .borrow()
        .throttle_input
        .len();
    throttle_event(app, TuiEvent::Key(Key::Enter.into()));
    throttle_event(app, TuiEvent::Key(Key::Home.into()));
    for _ in 0..length {
        throttle_event(app, TuiEvent::Key(Key::Delete.into()));
    }
    if !value.is_empty() {
        throttle_event(app, TuiEvent::Paste(value.into()));
    }
    throttle_event(app, TuiEvent::Key(Key::Esc.into()));
    finish_autosaves(app);
    app.tick(Duration::ZERO, AnimationSettings::default());
}

#[test]
fn throttle_field_rejects_non_digits_and_saves_zero_seconds_and_u32_values() {
    let mut app = editor(false);
    show_settings(&mut app);
    let original = saved_rule(&app);
    throttle_event(&mut app, TuiEvent::Key(Key::Enter.into()));
    throttle_event(&mut app, TuiEvent::Key(Key::Char('x').into()));
    throttle_event(&mut app, TuiEvent::Paste("4x5".into()));
    assert!(app.rule_save.is_none());
    assert_eq!(
        app.rule_editor.as_ref().unwrap().borrow().throttle_input,
        "0"
    );
    assert_eq!(saved_rule(&app), original);
    throttle_event(&mut app, TuiEvent::Key(Key::End.into()));
    throttle_event(&mut app, TuiEvent::Key(Key::Char('7').into()));
    throttle_event(&mut app, TuiEvent::Key(Key::Esc.into()));
    finish_autosaves(&mut app);
    assert_eq!(saved_rule(&app).definition.throttle_seconds, 7);
    for value in [45, 0, u32::MAX] {
        replace_throttle(&mut app, &value.to_string());
        let saved = saved_rule(&app);
        assert_eq!(saved.definition.throttle_seconds, value);
        assert!(!saved.definition.enabled);
        assert!(
            app.rule_editor
                .as_ref()
                .unwrap()
                .borrow()
                .throttle_error
                .is_none()
        );
        assert!(!render(&mut app, 130).1.contains("Enter seconds"));
    }
}

#[test]
fn invalid_throttle_input_is_visible_blocks_saves_and_pauses_active_rules() {
    for enabled in [false, true] {
        for (value, error) in [
            ("", "Enter seconds (0 = off)"),
            ("4294967296", "Enter seconds from 0 to 4294967295"),
        ] {
            let mut app = editor(enabled);
            show_settings(&mut app);
            let original = saved_rule(&app);
            replace_throttle(&mut app, value);
            let paused = saved_rule(&app);
            assert!(!paused.definition.enabled);
            assert_eq!(
                paused.definition.throttle_seconds,
                original.definition.throttle_seconds
            );
            assert_eq!(paused.revision, original.revision + i64::from(enabled));
            let text = render(&mut app, 130).1;
            assert!(text.contains(error), "{text}");
            let draft = app.rule_editor.as_ref().unwrap().clone();
            assert_eq!(draft.borrow().throttle_input, value);
            assert_eq!(draft.borrow().throttle_error.as_deref(), Some(error));
            for slot in ["trigger-at-end", "enabled"] {
                settings_input(&mut app, slot, &[Key::Char(' ')]);
                assert!(app.rule_save.is_none());
                assert!(draft.borrow().pending_toggle.is_none());
            }
            settings_input(
                &mut app,
                "description",
                &[Key::Enter, Key::End, Key::Char('!')],
            );
            finish_autosaves(&mut app);
            assert_eq!(saved_rule(&app), paused);
            assert!(!draft.borrow().dirty);
            assert!(app.rule_autosaves.is_empty());
            assert_eq!(app.rule_invalid_drafts.len(), 1);
            app.handle_message(Msg::Close, &mut EventCtx::default());
            assert!(app.rule_editor.is_none());
            let other = app
                .service
                .save_rule(rule("other").definition, None, Some("main".into()), false)
                .blocking_recv()
                .unwrap()
                .unwrap();
            app.open_rule(other, &mut EventCtx::default());
            show_settings(&mut app);
            settings_input(
                &mut app,
                "description",
                &[Key::Enter, Key::End, Key::Char('!')],
            );
            finish_autosaves(&mut app);
            settings_input(&mut app, "trigger-at-end", &[Key::Char(' ')]);
            finish_autosaves(&mut app);
            let other = app.rule_editor.as_ref().unwrap().borrow().saved.clone();
            assert_eq!(other.definition.description, "other event handler!");
            assert!(other.definition.trigger_at_end);
            assert_eq!(saved_rule(&app), paused);
            app.handle_message(Msg::Close, &mut EventCtx::default());
            app.handle_message(Msg::SaveRule(Box::new(paused)), &mut EventCtx::default());
            finish_autosaves(&mut app);
            let paused = saved_rule(&app);
            assert_eq!(draft.borrow().saved, paused);
            app.open_rule(paused, &mut EventCtx::default());
            show_settings(&mut app);
            assert!(Rc::ptr_eq(app.rule_editor.as_ref().unwrap(), &draft));
            assert_eq!(draft.borrow().throttle_input, value);
            assert_eq!(draft.borrow().throttle_error.as_deref(), Some(error));
            let text = render(&mut app, 130).1;
            assert!(text.contains(error), "{text}");
            assert!(text.contains("editable event handler!"), "{text}");
            replace_throttle(&mut app, "60");
            let saved = saved_rule(&app);
            assert_eq!(saved.definition.throttle_seconds, 60);
            assert_eq!(saved.definition.description, "editable event handler!");
            assert!(!saved.definition.enabled);
            assert!(!render(&mut app, 130).1.contains(error));
            assert!(app.rule_invalid_drafts.is_empty());
            app.handle_message(Msg::Close, &mut EventCtx::default());
            app.open_rule(saved, &mut EventCtx::default());
            assert!(!Rc::ptr_eq(app.rule_editor.as_ref().unwrap(), &draft));
            assert_eq!(
                app.rule_editor.as_ref().unwrap().borrow().throttle_input,
                "60"
            );
        }
    }
}

#[test]
fn invalid_throttle_drafts_keep_pending_toggle_results_and_revisions_after_closing() {
    for value in ["", "4294967296"] {
        let mut app = editor(true);
        show_settings(&mut app);
        settings_input(&mut app, "trigger-at-end", &[Key::Char(' ')]);
        let (send, result) = delay_save(&mut app);
        let draft = app.rule_editor.as_ref().unwrap().clone();
        for key in [Key::Enter, Key::Home, Key::Delete] {
            throttle_event(&mut app, TuiEvent::Key(key.into()));
        }
        if !value.is_empty() {
            throttle_event(&mut app, TuiEvent::Paste(value.into()));
        }
        app.handle_message(Msg::Close, &mut EventCtx::default());
        send.send(result).unwrap();
        assert!(app.poll_rule_save());
        finish_autosaves(&mut app);
        let saved = saved_rule(&app);
        assert!(saved.definition.trigger_at_end);
        assert!(!saved.definition.enabled);
        assert_eq!(draft.borrow().rule.revision, saved.revision);
        assert!(draft.borrow().pending_toggle.is_none());
        assert!(app.rule_autosaves.is_empty());
        app.open_rule(saved, &mut EventCtx::default());
        show_settings(&mut app);
        assert!(Rc::ptr_eq(app.rule_editor.as_ref().unwrap(), &draft));
        assert_eq!(draft.borrow().throttle_input, value);
        assert_setting(&mut app, "Trigger at end", true);
        replace_throttle(&mut app, "60");
        assert!(saved_rule(&app).definition.trigger_at_end);
        assert_eq!(draft.borrow().rule, saved_rule(&app));
        assert!(app.rule_invalid_drafts.is_empty());
    }
}

#[test]
fn trigger_at_end_preserves_activation_destination_and_selection_during_content_edits() {
    for enabled in [false, true] {
        let mut app = editor(enabled);
        show_settings(&mut app);
        let original = saved_rule(&app);
        let (layout, text) = render(&mut app, 130);
        assert!(text.contains("Throttle seconds (0 = off)"), "{text}");
        let field = layout
            .focus_targets()
            .iter()
            .find(|target| {
                target
                    .path
                    .keys()
                    .iter()
                    .any(|key| key.as_str() == "throttle-seconds")
            })
            .unwrap();
        let toggle = layout
            .focus_targets()
            .iter()
            .find(|target| {
                target
                    .path
                    .keys()
                    .iter()
                    .any(|key| key.as_str() == "trigger-at-end")
            })
            .unwrap();
        assert!(toggle.area.y >= field.area.bottom());
        settings_input(&mut app, "trigger-at-end", &[Key::Char(' ')]);
        let (send, result) = delay_save(&mut app);
        app.tick(Duration::ZERO, AnimationSettings::default());
        assert_setting(&mut app, "Trigger at end", true);
        let saved = result.as_ref().unwrap();
        assert!(saved.definition.trigger_at_end);
        assert_eq!(saved.definition.enabled, enabled);
        assert_eq!(saved.zellij_session, original.zellij_session);
        replace_throttle_without_wait(&mut app, "30");
        send.send(result).unwrap();
        assert!(app.poll_rule_save());
        finish_autosaves(&mut app);
        let saved = saved_rule(&app);
        assert!(saved.definition.trigger_at_end);
        assert_eq!(saved.definition.throttle_seconds, 30);
        assert!(!saved.definition.enabled);
        assert_eq!(saved.zellij_session, original.zellij_session);
        app.tick(Duration::ZERO, AnimationSettings::default());
        assert_setting(&mut app, "Trigger at end", true);
        settings_input(&mut app, "trigger-at-end", &[Key::Char(' ')]);
        finish_autosaves(&mut app);
        assert!(!saved_rule(&app).definition.trigger_at_end);
    }
}

fn replace_throttle_without_wait(app: &mut App, value: &str) {
    for key in [Key::Enter, Key::Home, Key::Delete] {
        throttle_event(app, TuiEvent::Key(key.into()));
    }
    throttle_event(app, TuiEvent::Paste(value.into()));
    throttle_event(app, TuiEvent::Key(Key::Esc.into()));
}

#[test]
fn rule_rows_project_retained_warnings_with_warning_color_and_errors_with_error_color() {
    init_ui();
    let issue = |name: &str, message: &str| crate::store::rules::Evaluation {
        attempt_id: 1,
        rule_name: name.into(),
        rule_revision: 1,
        error: message.into(),
    };
    let shared = Rc::new(RefCell::new(crate::store::rules::Snapshot {
        rules: vec![rule("limited"), rule("broken")],
        evaluation_warnings: vec![issue("limited", "Fixed timer active")],
        evaluation_errors: vec![issue("broken", "Invalid predicate")],
        ..Default::default()
    }));
    let mut view =
        crate::app::rules::Rules::new(shared, AppService::for_tests().environment_keys());
    let area = Rect::new(0, 0, 130, 20);
    view.layout(area, &mut tuicore::LayoutCtx::new());
    let mut terminal = Terminal::new(TestBackend::new(area.width, area.height)).unwrap();
    terminal
        .draw(|frame| {
            let mut ctx = RenderCtx::new();
            view.render(frame, area, &mut ctx);
            ctx.flush(frame);
        })
        .unwrap();
    let lines = rendered_lines(&terminal, area);
    for (message, color) in [
        (
            "Last evaluation warning: Fixed timer active",
            tuicore::theme().warning_fg(),
        ),
        (
            "Last evaluation failure: Invalid predicate",
            tuicore::theme().error_fg(),
        ),
    ] {
        let y = lines
            .iter()
            .position(|line| line.contains(message))
            .unwrap();
        let x = lines[y][..lines[y].find(message).unwrap()].chars().count();
        assert_eq!(terminal.backend().buffer()[(x as u16, y as u16)].fg, color);
    }
}

#[test]
fn reopening_during_trigger_at_end_save_preserves_selection_and_text_edits() {
    for edit in [false, true] {
        reopen_pending_toggle("trigger-at-end", edit);
    }
}

#[test]
fn failed_trigger_at_end_save_restores_the_saved_selection() {
    for enabled in [false, true] {
        let mut app = editor(enabled);
        show_settings(&mut app);
        let original = saved_rule(&app);
        app.service
            .save_rule(
                original.definition.clone(),
                Some(original.revision),
                Some(original.zellij_session.clone()),
                true,
            )
            .blocking_recv()
            .unwrap()
            .unwrap();
        settings_input(&mut app, "trigger-at-end", &[Key::Char(' ')]);
        let (send, result) = delay_save(&mut app);
        assert!(result.is_err());
        app.tick(Duration::ZERO, AnimationSettings::default());
        assert_setting(&mut app, "Trigger at end", true);
        send.send(result).unwrap();
        assert!(app.poll_rule_save());
        app.tick(Duration::ZERO, AnimationSettings::default());
        assert_setting(&mut app, "Trigger at end", false);
        assert_eq!(app.rule_editor.as_ref().unwrap().borrow().rule, original);
    }
}

fn discard_edits(app: &mut App, draft: &Rc<RefCell<crate::app::rules::Draft>>) {
    let (layout, text) = render(app, 130);
    assert!(text.contains("Discard unsaved edits"), "{text}");
    assert!(text.contains("input and content lost"), "{text}");
    let target = layout
        .focus_targets()
        .iter()
        .find(|target| {
            target
                .path
                .keys()
                .iter()
                .any(|key| key.as_str() == "discard-rule-draft")
        })
        .unwrap();
    app.dispatch_focus(target, true, &mut tuicore::FocusCtx::default());
    let mut ctx = EventCtx::default();
    app.dispatch_event(
        &EventRoute::new(target.path.clone()),
        &TuiEvent::Key(Key::Enter.into()),
        &mut ctx,
    );
    assert!(matches!(ctx.messages(), [Msg::DiscardRuleDraft(target)] if Rc::ptr_eq(target, draft)));
    for message in ctx.drain_messages() {
        app.handle_message(message, &mut EventCtx::default());
    }
}

#[test]
fn discarding_a_conflicted_throttle_draft_reopens_external_values_and_allows_new_edits() {
    for value in ["", "4294967296"] {
        let mut app = editor(false);
        show_settings(&mut app);
        replace_throttle(&mut app, value);
        let draft = app.rule_editor.as_ref().unwrap().clone();
        settings_input(
            &mut app,
            "description",
            &[Key::Enter, Key::End, Key::Char('!')],
        );
        finish_autosaves(&mut app);
        let original = saved_rule(&app);
        let mut external = original.definition.clone();
        external.description = "External change".into();
        external.throttle_seconds = 90;
        external.trigger_at_end = true;
        let external = app
            .service
            .save_rule(
                external,
                Some(original.revision),
                Some("main".into()),
                false,
            )
            .blocking_recv()
            .unwrap()
            .unwrap();
        replace_throttle(&mut app, "60");
        assert_eq!(saved_rule(&app), external);
        assert_eq!(draft.borrow().saved, original);
        let notice = app.notifications.center().history().last().unwrap();
        assert_eq!(notice.title(), "Cannot save rule");
        assert_eq!(notice.kind(), tuicore::NotificationKind::Error);
        app.handle_message(Msg::Close, &mut EventCtx::default());
        app.open_rule(external.clone(), &mut EventCtx::default());
        show_settings(&mut app);
        assert!(Rc::ptr_eq(app.rule_editor.as_ref().unwrap(), &draft));
        assert_eq!(draft.borrow().throttle_input, "60");
        assert_eq!(
            draft.borrow().rule.definition.description,
            "editable event handler!"
        );
        discard_edits(&mut app, &draft);
        assert!(app.rule_invalid_drafts.is_empty());
        assert!(app.rule_autosaves.is_empty());
        assert!(app.rule_editor.is_none());
        assert!(!app.view.is_active());
        assert_eq!(saved_rule(&app), external);
        show_rules(&mut app, vec![external.clone()]);
        app.open_rule(external.clone(), &mut EventCtx::default());
        show_settings(&mut app);
        let reopened = app.rule_editor.as_ref().unwrap().clone();
        assert!(!Rc::ptr_eq(&reopened, &draft));
        assert_eq!(reopened.borrow().rule, external);
        assert_eq!(reopened.borrow().throttle_input, "90");
        replace_throttle(&mut app, "45");
        let saved = saved_rule(&app);
        assert_eq!(saved.definition.description, "External change");
        assert_eq!(saved.definition.throttle_seconds, 45);
        assert!(saved.definition.trigger_at_end);
        assert!(saved.revision > external.revision);
        assert_eq!(reopened.borrow().saved, saved);
    }
}

#[test]
fn discarding_edits_waits_for_saves_and_preserves_other_rules_retained_drafts() {
    let mut app = editor(false);
    show_settings(&mut app);
    replace_throttle(&mut app, "");
    let first = app.rule_editor.as_ref().unwrap().clone();
    let other = app
        .service
        .save_rule(rule("other").definition, None, Some("main".into()), false)
        .blocking_recv()
        .unwrap()
        .unwrap();
    app.open_rule(other.clone(), &mut EventCtx::default());
    show_settings(&mut app);
    replace_throttle(&mut app, "4294967296");
    let second = app.rule_editor.as_ref().unwrap().clone();
    app.handle_message(Msg::Close, &mut EventCtx::default());
    app.open_rule(saved_rule(&app), &mut EventCtx::default());
    show_settings(&mut app);
    replace_throttle_without_wait(&mut app, "60");
    let (send, result) = delay_save(&mut app);
    discard_edits(&mut app, &first);
    assert!(app.view.is_active());
    assert!(Rc::ptr_eq(app.rule_editor.as_ref().unwrap(), &first));
    assert!(app.rule_invalid_drafts.contains_key("editable"));
    assert!(app.rule_invalid_drafts.contains_key("other"));
    send.send(result).unwrap();
    finish_autosaves(&mut app);
    app.rule_autosaves.extend([first.clone(), second.clone()]);
    discard_edits(&mut app, &first);
    assert!(!app.rule_invalid_drafts.contains_key("editable"));
    assert!(Rc::ptr_eq(
        app.rule_invalid_drafts.get("other").unwrap(),
        &second
    ));
    assert!(matches!(app.rule_autosaves.as_slice(), [queued] if Rc::ptr_eq(queued, &second)));
    assert!(app.rule_editor.is_none());
    app.open_rule(other, &mut EventCtx::default());
    assert!(Rc::ptr_eq(app.rule_editor.as_ref().unwrap(), &second));
    assert_eq!(second.borrow().throttle_input, "4294967296");
}
