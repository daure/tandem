use super::*;

#[test]
fn starting_instances_without_sessions_follow_each_tabs_active_filter() {
    init_ui();
    let mut app = root(AppService::for_tests());
    for sessions in [false, true] {
        app.attached_sessions_only = sessions;
        for active_only in [false, true] {
            app.running_only = active_only;
            for action in [None, Some("create_instance"), Some("start_service")] {
                let mut inventory = snapshot();
                let instance = &mut inventory.instances[0];
                instance.services.clear();
                instance.pending = action.is_none();
                instance.runtime.activity =
                    action.map(|action| crate::store::environments::Activity {
                        id: "startup".into(),
                        name: "review".into(),
                        template: Some("website".into()),
                        service: None,
                        action: action.into(),
                        owner_pid: std::process::id(),
                        started_at: 1,
                        deadline: u64::MAX,
                        error: None,
                        finished: false,
                    });
                let rows = app.project_rows(&inventory, &[]);
                let row = rows.iter().find(|row| row.id == "instance:review");
                assert_eq!(row.is_some(), !sessions || !active_only);
                if let Some(row) = row {
                    assert!(row.starting && row.loading);
                    assert_eq!(row.parent.is_none(), sessions);
                }

                inventory.instances[0].pending = false;
                inventory.instances[0].runtime.activity = None;
                let rows = app.project_rows(&inventory, &[]);
                assert_eq!(
                    rows.iter().any(|row| row.id == "instance:review"),
                    !sessions && !active_only,
                );
            }
        }
    }
}

#[test]
fn creation_preserves_literal_prompts_and_resets_fields_between_dialogs() {
    init_ui();
    let mut app = root(AppService::for_tests());
    app.set_rows_for_tests(rows::from_snapshot(&snapshot()));
    let mut ctx = EventCtx::new(AnimationSettings::default());
    app.action(1, &mut ctx);
    assert_eq!(app.creation.opencode(), None);
    assert!(app.creation.start_instance);
    app.handle_message(Msg::StartInstanceChanged(false), &mut ctx);
    assert!(!app.creation.start_instance);
    for prompt in ["", "   ", "\n\t\r\n", "\u{2003}\u{a0}"] {
        app.handle_message(Msg::InitialPromptChanged(prompt.into()), &mut ctx);
        assert_eq!(app.creation.opencode(), None);
    }
    let prompt = "Explain 'this' \"project\"; $(touch injected)\nsecond line";
    app.handle_message(Msg::InitialPromptChanged(prompt.into()), &mut ctx);
    assert_eq!(app.creation.opencode(), Some(Some(prompt.into())));
    app.handle_message(Msg::DescriptionChanged("First\nSecond".into()), &mut ctx);
    assert_eq!(app.description, "First\nSecond");
    app.handle_message(Msg::Close, &mut ctx);
    app.action(1, &mut ctx);
    assert_eq!(app.creation.opencode(), None);
    assert!(app.creation.prompt.is_empty());
    assert!(app.creation.start_instance);
    assert!(app.description.is_empty());
}

#[test]
fn creation_start_toggle_requires_a_template_compose_file() {
    init_ui();
    let mut app = root(AppService::for_tests());
    let mut ctx = EventCtx::new(AnimationSettings::default());
    for has_compose in [true, false, true] {
        let mut inventory = snapshot();
        if !has_compose {
            inventory.templates[0].compose_file.clear();
            inventory.templates[0].compose_source.clear();
            inventory.instances[0].services.clear();
            inventory.instances[0].workspace_only = true;
        }
        app.set_rows_for_tests(rows::from_snapshot(&inventory));
        for selected in ["template:/tmp/templates/website", "instance:review"] {
            crate::app::instances::set_highlighted(&app.instances, Some(selected.into()));
            app.action(1, &mut ctx);
            assert_eq!(app.creation.start_instance, has_compose);

            let area = Rect::new(0, 0, 100, 40);
            let mut layout = LayoutEngine::new();
            layout.layout(&mut app, area);
            let target = layout
                .focus_targets()
                .iter()
                .find(|target| {
                    target
                        .path
                        .keys()
                        .iter()
                        .any(|key| key.as_str() == "start-instance")
                })
                .unwrap()
                .clone();
            let mut terminal = Terminal::new(TestBackend::new(area.width, area.height)).unwrap();
            terminal
                .draw(|frame| {
                    let mut render = RenderCtx::new();
                    app.render(frame, area, &mut render);
                    render.flush(frame);
                })
                .unwrap();
            let lines = rendered_lines(&terminal, area);
            let line = lines
                .iter()
                .find(|line| line.contains("Start instance"))
                .unwrap();
            assert!(line.contains(if has_compose {
                "──●"
            } else {
                "○──"
            }));
            if !has_compose {
                let label_x = line.find('S').unwrap();
                let label_x = line[..label_x].chars().count() as u16;
                let label_y = lines
                    .iter()
                    .position(|candidate| candidate == line)
                    .unwrap() as u16;
                assert_eq!(
                    terminal
                        .backend()
                        .buffer()
                        .cell((label_x, label_y))
                        .unwrap()
                        .fg,
                    tuicore::theme().muted_fg(),
                );
            }

            app.dispatch_focus(&target, true, &mut tuicore::FocusCtx::default());
            let mut input = EventCtx::new(AnimationSettings::default());
            app.dispatch_event(
                &EventRoute::new(target.path),
                &TuiEvent::Key(KeyEvent::from(Key::Char(' '))),
                &mut input,
            );
            if has_compose {
                assert!(matches!(
                    input.messages(),
                    [Msg::StartInstanceChanged(false)]
                ));
            } else {
                assert!(input.messages().is_empty());
                app.handle_message(Msg::StartInstanceChanged(true), &mut ctx);
                assert!(!app.creation.start_instance);
            }
            app.handle_message(Msg::StartInstanceChanged(false), &mut ctx);
            app.handle_message(Msg::Close, &mut ctx);
        }
    }
}

#[test]
fn creation_launch_errors_are_reported_for_each_instance() {
    let mut app = root(AppService::for_tests());
    let (pending, reply) = tokio::sync::oneshot::channel();
    app.creation.launches.push(("pending".into(), reply));
    let (failed, reply) = tokio::sync::oneshot::channel();
    app.creation.launches.push(("review".into(), reply));
    failed.send(Err("launch failed".into())).unwrap();
    assert!(app.poll_creation_launches());
    assert_eq!(app.creation.launches.len(), 1);
    let notice = app.notifications.center().history().last().unwrap();
    assert_eq!(notice.title(), "Cannot open OpenCode");
    assert_eq!(notice.body(), "review: launch failed");
    pending.send(Ok(())).unwrap();
    assert!(!app.poll_creation_launches());
    assert!(app.creation.launches.is_empty());
}

#[test]
fn description_content_grows_from_two_to_eight_rows() {
    init_ui();
    for (description, expected_height) in [("".into(), 2), ("line\n".repeat(12), 8)] {
        let mut dialog = crate::app::dialogs::instance_entry(
            "New instance",
            "review",
            &description,
            &crate::app::creation::Creation::default(),
            "Instance name",
            None,
        );
        let mut layout = LayoutEngine::new();
        layout.layout(&mut dialog, Rect::new(0, 0, 80, 40));
        let field = layout
            .focus_targets()
            .iter()
            .find(|target| {
                target
                    .path
                    .keys()
                    .iter()
                    .any(|key| key.as_str() == "description")
            })
            .unwrap();
        assert_eq!(field.area.height, expected_height);
    }
}

#[test]
fn creation_textareas_require_enter_after_tab_and_accept_multiline_input() {
    init_ui();
    let mut app = root(AppService::for_tests());
    app.set_rows_for_tests(rows::from_snapshot(&snapshot()));
    app.action(1, &mut EventCtx::new(AnimationSettings::default()));
    let mut layout = LayoutEngine::new();
    layout.layout(&mut app, Rect::new(0, 0, 80, 30));
    let name = layout
        .focus_targets()
        .iter()
        .find(|target| target.path.keys().iter().any(|key| key.as_str() == "name"))
        .unwrap();
    let settings = AnimationSettings::default();
    let mut focus = tuicore::FocusManager::new();
    let mut dispatcher = tuicore::TreeDispatcher::new();
    let transition = focus
        .apply_request(
            &tuicore::FocusRequest::Path(name.path.clone()),
            layout.focus_targets(),
        )
        .unwrap();
    dispatcher.dispatch_focus(&mut app, transition, settings);
    app.dispatch_event(
        &EventRoute::new(focus.current_path()),
        &TuiEvent::Key(KeyEvent::from(Key::Esc)),
        &mut EventCtx::new(settings),
    );
    for field in ["description", "initial-prompt"] {
        let mut ctx = EventCtx::new(settings);
        let outcome = app.dispatch_event(
            &EventRoute::new(focus.current_path()),
            &TuiEvent::Key(KeyEvent::from(Key::Tab)),
            &mut ctx,
        );
        assert_eq!(outcome, tuicore::EventOutcome::Ignored);
        assert_eq!(ctx.propagation(), tuicore::Propagation::Continue);
        let transition = focus.next(layout.focus_targets()).unwrap();
        dispatcher.dispatch_focus(&mut app, transition, settings);
        assert!(
            focus
                .current_path()
                .keys()
                .iter()
                .any(|key| key.as_str() == field)
        );
        let route = EventRoute::new(focus.current_path());
        let mut ctx = EventCtx::new(settings);
        app.dispatch_event(&route, &TuiEvent::Paste("Ignored".into()), &mut ctx);
        assert!(
            ctx.messages().is_empty(),
            "{field} must start in focus mode"
        );
        let mut ctx = EventCtx::new(settings);
        app.dispatch_event(&route, &TuiEvent::Key(KeyEvent::from(Key::Enter)), &mut ctx);
        assert!(
            ctx.messages().is_empty(),
            "Enter activates editing without inserting a newline"
        );
        let mut ctx = EventCtx::new(AnimationSettings::default());
        app.dispatch_event(&route, &TuiEvent::Paste("First\nSecond".into()), &mut ctx);
        assert!(matches!(ctx.messages(),
            [Msg::DescriptionChanged(value) | Msg::InitialPromptChanged(value)] if value == "First\nSecond"
        ));
        let mut ctx = EventCtx::new(AnimationSettings::default());
        app.dispatch_event(&route, &TuiEvent::Key(KeyEvent::from(Key::Enter)), &mut ctx);
        assert!(matches!(ctx.messages(),
            [Msg::DescriptionChanged(value) | Msg::InitialPromptChanged(value)] if value == "First\nSecond\n"
        ));
        assert!(app.view.is_active());
        assert!(app.service.operations().is_empty());
        app.dispatch_event(
            &route,
            &TuiEvent::Key(KeyEvent::from(Key::Esc)),
            &mut EventCtx::new(settings),
        );
    }
}

#[test]
fn creation_shortcut_submits_from_name_or_textarea_focus_mode() {
    init_ui();
    let shortcut = TuiEvent::Key(KeyEvent {
        code: Key::Enter,
        modifiers: KeyModifiers::CONTROL,
    });
    for field in ["name", "description", "initial-prompt"] {
        let service = AppService::for_tests();
        let existing = service.queue_instance_for_tests("review", "website");
        service.complete_instance_for_tests(&existing.id, snapshot().instances.remove(0));
        let mut app = root(service);
        let mut ctx = EventCtx::new(AnimationSettings::default());
        app.intent = Some(crate::app::Intent::CreateInstance("website".into()));
        app.open_name_entry(&mut ctx);
        app.handle_message(Msg::NameChanged("review".into()), &mut ctx);
        let mut layout = LayoutEngine::new();
        layout.layout(&mut app, Rect::new(0, 0, 80, 30));
        let target = layout
            .focus_targets()
            .iter()
            .find(|target| target.path.keys().iter().any(|key| key.as_str() == field))
            .unwrap()
            .clone();
        app.dispatch_focus(&target, true, &mut tuicore::FocusCtx::default());
        let route = EventRoute::new(target.path);
        app.dispatch_event(
            &route,
            &shortcut,
            &mut EventCtx::new(AnimationSettings::default()),
        );
        assert!(!app.view.is_active(), "{field}");
        assert!(app.intent.is_none(), "{field}");
        assert_eq!(app.service.operations().len(), 1);
        assert_eq!(
            app.notifications.center().history().last().unwrap().title(),
            "Instance already exists"
        );
    }
}
