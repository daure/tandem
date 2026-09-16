use super::*;

fn render_app(app: &mut crate::app::App) -> Vec<String> {
    let area = Rect::new(0, 0, 130, 40);
    app.layout(area, &mut tuicore::LayoutCtx::new());
    let mut terminal = Terminal::new(TestBackend::new(area.width, area.height)).unwrap();
    terminal
        .draw(|frame| {
            let mut render = RenderCtx::new();
            app.render(frame, area, &mut render);
            render.flush(frame);
        })
        .unwrap();
    rendered_lines(&terminal, area)
}

fn name_input_route(app: &mut crate::app::App) -> tuicore::EventRoute {
    let mut layout = tuicore::LayoutCtx::new();
    app.layout(Rect::new(0, 0, 130, 40), &mut layout);
    let target = layout
        .focus_targets()
        .iter()
        .find(|target| target.path.keys().iter().any(|key| key.as_str() == "name"))
        .unwrap();
    tuicore::EventRoute::new(target.path.clone())
}

#[test]
fn template_button_and_uppercase_hotkey_open_template_creation() {
    tuicore::init();
    let mut app = root(AppService::for_tests());
    app.set_rows_for_tests(rows::from_snapshot(&snapshot()));
    let lines = render_app(&mut app);
    let button_y = lines
        .iter()
        .position(|line| line.contains("Template |T|"))
        .unwrap_or_else(|| panic!("template button with hotkey badge: {lines:#?}"));
    let tree_y = lines
        .iter()
        .position(|line| line.contains("website"))
        .unwrap();
    assert!(button_y < tree_y);
    for modifiers in [KeyModifiers::NONE, KeyModifiers::SHIFT] {
        app.event(
            &TuiEvent::Key(KeyEvent {
                code: Key::Char('T'),
                modifiers,
            }),
            &mut EventCtx::new(AnimationSettings::default()),
        );
        assert!(matches!(app.intent, Some(crate::app::Intent::NewTemplate)));
        assert!(
            render_app(&mut app)
                .iter()
                .any(|line| line.contains("Template name"))
        );
        app.handle_message(Msg::Close, &mut EventCtx::new(AnimationSettings::default()));
    }
    crate::app::instances::set_highlighted(&app.instances, Some("instance:review".into()));
    let mut layout = tuicore::LayoutCtx::new();
    app.layout(Rect::new(0, 0, 130, 40), &mut layout);
    let button = layout
        .focus_targets()
        .iter()
        .find(|target| {
            target
                .path
                .keys()
                .iter()
                .any(|key| key.as_str() == "new-template")
        })
        .unwrap()
        .clone();
    app.dispatch_focus(&button, true, &mut tuicore::FocusCtx::default());
    let mut ctx = EventCtx::new(AnimationSettings::default());
    app.dispatch_event(
        &tuicore::EventRoute::new(button.path),
        &TuiEvent::Key(KeyEvent::from(Key::Enter)),
        &mut ctx,
    );
    assert!(matches!(ctx.messages(), [Msg::NewTemplate]));
    assert!(app.service.opened_system_targets().is_empty());
    app.handle_message(
        Msg::NewTemplate,
        &mut EventCtx::new(AnimationSettings::default()),
    );
    assert!(matches!(app.intent, Some(crate::app::Intent::NewTemplate)));
}

#[test]
fn new_instance_uses_a_mode_specific_placeholder() {
    tuicore::init();
    let mut app = root(AppService::for_tests());
    app.handle_message(
        Msg::SetBranchInstances(false),
        &mut EventCtx::new(AnimationSettings::default()),
    );
    app.set_rows_for_tests(rows::from_snapshot(&snapshot()));
    for selected in ["instance:review", "service:review:web"] {
        crate::app::instances::set_highlighted(&app.instances, Some(selected.into()));
        app.event(
            &TuiEvent::Key(KeyEvent::from(Key::Char('n'))),
            &mut EventCtx::new(AnimationSettings::default()),
        );
        assert!(app.intent.is_none());
        assert!(!app.view.first().is_active());
    }
    crate::app::instances::set_highlighted(
        &app.instances,
        Some("template:/tmp/templates/website".into()),
    );
    app.action(1, &mut EventCtx::new(AnimationSettings::default()));
    let lines = render_app(&mut app);
    assert!(lines.iter().any(|line| line.contains("New instance")));
    assert!(
        lines.iter().any(|line| line.contains("Instance name")),
        "{lines:#?}"
    );

    app.handle_message(
        Msg::SetBranchInstances(true),
        &mut EventCtx::new(AnimationSettings::default()),
    );
    app.handle_message(Msg::Close, &mut EventCtx::new(AnimationSettings::default()));
    app.action(1, &mut EventCtx::new(AnimationSettings::default()));
    assert!(
        render_app(&mut app)
            .iter()
            .any(|line| line.contains("branch-name"))
    );
}

#[test]
fn only_branch_mode_restricts_new_instance_input() {
    tuicore::init();
    let mut app = root(AppService::for_tests());
    app.set_rows_for_tests(rows::from_snapshot(&snapshot()));
    crate::app::instances::set_highlighted(
        &app.instances,
        Some("template:/tmp/templates/website".into()),
    );
    app.handle_message(
        Msg::SetBranchInstances(false),
        &mut EventCtx::new(AnimationSettings::default()),
    );
    app.action(1, &mut EventCtx::new(AnimationSettings::default()));
    let route = name_input_route(&mut app);
    let mut unrestricted = EventCtx::new(AnimationSettings::default());
    app.dispatch_event(
        &route,
        &TuiEvent::Key(KeyEvent::from(Key::Char(' '))),
        &mut unrestricted,
    );
    assert!(matches!(unrestricted.messages(), [Msg::NameChanged(value)] if value == " "));

    app.handle_message(Msg::Close, &mut EventCtx::new(AnimationSettings::default()));
    app.handle_message(
        Msg::SetBranchInstances(true),
        &mut EventCtx::new(AnimationSettings::default()),
    );
    app.action(1, &mut EventCtx::new(AnimationSettings::default()));
    let route = name_input_route(&mut app);
    let mut invalid = EventCtx::new(AnimationSettings::default());
    app.dispatch_event(
        &route,
        &TuiEvent::Key(KeyEvent::from(Key::Char(' '))),
        &mut invalid,
    );
    assert!(invalid.messages().is_empty());
    for _ in 0..40 {
        let mut allowed = EventCtx::new(AnimationSettings::default());
        app.dispatch_event(
            &route,
            &TuiEvent::Key(KeyEvent::from(Key::Char('a'))),
            &mut allowed,
        );
        assert!(matches!(allowed.messages(), [Msg::NameChanged(_)]));
    }
    let mut rejected = EventCtx::new(AnimationSettings::default());
    app.dispatch_event(
        &route,
        &TuiEvent::Key(KeyEvent::from(Key::Char('a'))),
        &mut rejected,
    );
    assert!(rejected.messages().is_empty());
}

#[test]
fn row_delete_and_bulk_purge_keys_open_distinct_confirmations() {
    tuicore::init();
    for (selected, key, confirm_key, title) in [
        (
            "template:/tmp/templates/website",
            'p',
            'p',
            "Purge all instances",
        ),
        ("instance:review", 'x', 'd', "Delete instance"),
        (
            "template:/tmp/templates/website",
            'x',
            'd',
            "Delete template",
        ),
    ] {
        let hotkey = KeyEvent {
            code: Key::Char(key),
            modifiers: KeyModifiers::NONE,
        };
        let mut app = root(AppService::for_tests());
        app.set_rows_for_tests(rows::from_snapshot(&snapshot()));
        crate::app::instances::set_highlighted(&app.instances, Some(selected.into()));
        app.event(
            &TuiEvent::Key(hotkey),
            &mut EventCtx::new(AnimationSettings::default()),
        );
        match (selected, key) {
            ("template:/tmp/templates/website", 'x') => assert!(
                matches!(app.intent, Some(crate::app::Intent::RemoveTemplate(ref name)) if name == "website")
            ),
            ("instance:review", _) => assert!(
                matches!(app.intent, Some(crate::app::Intent::Delete(ref name)) if name == "review")
            ),
            _ => assert!(
                matches!(app.intent, Some(crate::app::Intent::DeleteTemplate(ref name)) if name == "website")
            ),
        }
        assert!(render_app(&mut app).iter().any(|line| line.contains(title)));
        let route = tuicore::EventRoute::new(tuicore::TreePath::from_keys([
            tuicore::ChildKey::first(),
            tuicore::ChildKey::second(),
        ]));
        let mut confirm = EventCtx::new(AnimationSettings::default());
        app.dispatch_event(
            &route,
            &TuiEvent::Key(KeyEvent::from(Key::Char(confirm_key))),
            &mut confirm,
        );
        assert!(matches!(confirm.messages(), [Msg::Submit]));
        assert!(app.service.operations().is_empty());
    }
    let mut app = root(AppService::for_tests());
    app.set_rows_for_tests(rows::from_snapshot(&snapshot()));
    crate::app::instances::set_highlighted(&app.instances, Some("instance:review".into()));
    app.event(
        &TuiEvent::Key(KeyEvent::from(Key::Char('p'))),
        &mut EventCtx::new(AnimationSettings::default()),
    );
    assert!(app.intent.is_none());
    assert!(!app.view.first().is_active());
}

#[test]
fn template_menu_deletion_requires_confirmation_and_is_unavailable_for_missing_recipes() {
    tuicore::init();
    for available in [true, false] {
        let mut snapshot = snapshot();
        if !available {
            snapshot.templates.clear();
        }
        let mut app = root(AppService::for_tests());
        app.set_rows_for_tests(rows::from_snapshot(&snapshot));
        render_app(&mut app);
        for character in ".Delete template".chars() {
            app.event(
                &TuiEvent::Key(KeyEvent::from(Key::Char(character))),
                &mut EventCtx::new(AnimationSettings::default()),
            );
        }
        app.event(
            &TuiEvent::Key(KeyEvent::from(Key::Enter)),
            &mut EventCtx::new(AnimationSettings::default()),
        );
        assert!(app.service.operations().is_empty());
        if available {
            assert!(
                matches!(app.intent, Some(crate::app::Intent::RemoveTemplate(ref name)) if name == "website")
            );
            let lines = render_app(&mut app);
            assert!(lines.iter().any(|line| line.contains("Delete template")));
            assert!(
                lines
                    .iter()
                    .any(|line| line.contains("Stop and remove all its instances"))
            );
            assert!(lines.iter().any(|line| line.contains("workspace folders")));
            assert!(
                lines
                    .iter()
                    .any(|line| line.contains("This cannot be undone"))
            );
            assert!(
                lines
                    .iter()
                    .any(|line| line.contains("/tmp/templates/website"))
            );
            app.handle_message(Msg::Close, &mut EventCtx::new(AnimationSettings::default()));
            assert!(app.service.operations().is_empty());
        } else {
            assert!(app.intent.is_none());
        }
    }
}
