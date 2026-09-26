use super::*;

fn inventory() -> EnvironmentSnapshot {
    let mut inventory = snapshot();
    for (name, status) in [("other", "up"), ("stopped", "down (exit 0)")] {
        let mut instance = inventory.instances[0].clone();
        instance.name = name.into();
        instance.template = name.into();
        instance.template_directory = format!("/missing/{name}");
        instance.services[0].status = status.into();
        inventory.instances.push(instance);
    }
    inventory
}

fn render(app: &mut super::super::App, width: u16) -> (tuicore::LayoutCtx, Vec<String>) {
    let area = Rect::new(0, 0, width, 30);
    let mut layout = tuicore::LayoutCtx::new();
    app.layout(area, &mut layout);
    let mut terminal = Terminal::new(TestBackend::new(width, area.height)).unwrap();
    terminal
        .draw(|frame| {
            let mut ctx = RenderCtx::new();
            app.render(frame, area, &mut ctx);
            ctx.flush(frame);
        })
        .unwrap();
    (layout, rendered_lines(&terminal, area))
}

#[test]
fn bulk_buttons_are_responsive_and_follow_instance_availability() {
    tuicore::init();
    let mut app = root(AppService::for_tests());
    for width in [40, 130] {
        for (status, stop_enabled, purge_enabled) in [
            (None, false, false),
            (Some("down (exit 0)"), false, true),
            (Some("up"), true, true),
            (Some("paused"), true, true),
            (Some("restarting"), true, true),
        ] {
            let mut inventory = snapshot();
            if let Some(status) = status {
                inventory.instances[0].services[0].status = status.into();
            } else {
                inventory.instances.clear();
            }
            app.update_snapshot(inventory);
            let (layout, lines) = render(&mut app, width);
            assert!(lines[1].contains(if width < 100 { "" } else { " Stop all" }));
            assert!(lines[1].contains(if width < 100 { "" } else { " Purge all" }));
            assert!(
                lines[1].contains(if width < 50 { " S" } else { "|S|" }),
                "{}",
                lines[1]
            );
            assert!(
                lines[1].contains(if width < 50 { " P" } else { "|P|" }),
                "{}",
                lines[1]
            );
            for (key, enabled) in [("stop-all", stop_enabled), ("purge-all", purge_enabled)] {
                let target = layout
                    .focus_targets()
                    .iter()
                    .find(|target| target.path.keys().iter().any(|part| part.as_str() == key));
                assert_eq!(
                    target.is_some_and(|target| target.enabled),
                    enabled,
                    "{status:?} {key}"
                );
                if let Some(target) = target {
                    assert!(target.area.right() <= width);
                    if !enabled {
                        let mut ctx = EventCtx::new(AnimationSettings::default());
                        app.dispatch_event(
                            &tuicore::EventRoute::new(target.path.clone()),
                            &TuiEvent::Key(KeyEvent::from(Key::Enter)),
                            &mut ctx,
                        );
                        assert!(ctx.messages().is_empty());
                    }
                    app.dispatch_event(
                        &tuicore::EventRoute::new(target.path.clone()),
                        &TuiEvent::Key(KeyEvent::from(Key::Esc)),
                        &mut EventCtx::new(AnimationSettings::default()),
                    );
                }
            }
        }
    }
    app.update_snapshot(Default::default());
    for message in [Msg::StopAll, Msg::PurgeAll] {
        app.handle_message(message, &mut EventCtx::new(AnimationSettings::default()));
        assert!(app.intent.is_none());
        assert!(!app.view.is_active());
    }
    assert!(app.service.operations().is_empty());
}

#[test]
fn uppercase_bulk_hotkeys_open_confirmation_and_respect_disabled_states_and_search() {
    tuicore::init();
    for width in [40, 130] {
        let mut app = root(AppService::for_tests());
        app.update_snapshot(inventory());
        for key in [
            KeyEvent::from(Key::Char('S')),
            KeyEvent {
                code: Key::Char('s'),
                modifiers: KeyModifiers::SHIFT,
            },
            KeyEvent::from(Key::Char('P')),
            KeyEvent {
                code: Key::Char('p'),
                modifiers: KeyModifiers::SHIFT,
            },
        ] {
            render(&mut app, width);
            let mut ctx = EventCtx::new(AnimationSettings::default());
            app.event(&TuiEvent::Key(key), &mut ctx);
            assert!(matches!(
                app.intent,
                Some(super::super::Intent::StopAll(_) | super::super::Intent::PurgeAll(_))
            ));
            assert!(app.service.operations().is_empty());
            app.handle_message(Msg::Close, &mut ctx);
        }

        let (layout, _) = render(&mut app, width);
        let tree = layout
            .focus_targets()
            .iter()
            .find(|target| target.id.as_str() == super::super::TREE_FOCUS)
            .unwrap()
            .clone();
        app.dispatch_focus(&tree, true, &mut tuicore::FocusCtx::default());
        for key in ['/', 'S', 'P'] {
            let mut ctx = EventCtx::new(AnimationSettings::default());
            app.dispatch_event(
                &tuicore::EventRoute::new(tree.path.clone()),
                &TuiEvent::Key(KeyEvent::from(Key::Char(key))),
                &mut ctx,
            );
            assert!(ctx.messages().is_empty());
            assert!(app.intent.is_none());
        }
        assert!(super::super::instances::is_searching(&app.instances));
    }
    let mut app = root(AppService::for_tests());
    for key in ['S', 'P'] {
        app.event(
            &TuiEvent::Key(KeyEvent::from(Key::Char(key))),
            &mut EventCtx::new(AnimationSettings::default()),
        );
        assert!(app.intent.is_none());
    }
    let mut stopped = snapshot();
    stopped.instances[0].services[0].status = "down (exit 0)".into();
    app.update_snapshot(stopped);
    app.event(
        &TuiEvent::Key(KeyEvent::from(Key::Char('S'))),
        &mut EventCtx::new(AnimationSettings::default()),
    );
    assert!(app.intent.is_none());
}

#[test]
fn toolbar_bulk_hotkeys_use_configured_letters_for_labels_and_activation() {
    tuicore::init();
    for width in [40, 130] {
        let state = std::rc::Rc::new(std::cell::RefCell::new(
            super::super::toolbar::State::from_snapshot(&inventory()),
        ));
        let mut toolbar = super::super::toolbar::Toolbar::new(
            tuicore::KeySpec::shifted('t'),
            tuicore::KeySpec::shifted('r'),
            tuicore::KeySpec::shifted('k'),
            tuicore::KeySpec::shifted('l'),
            state,
        );
        let area = Rect::new(0, 0, width, 1);
        toolbar.layout(area, &mut tuicore::LayoutCtx::new());
        let mut terminal = Terminal::new(TestBackend::new(width, 1)).unwrap();
        terminal
            .draw(|frame| toolbar.render(frame, area, &mut RenderCtx::new()))
            .unwrap();
        let line = &rendered_lines(&terminal, area)[0];
        assert!(
            line.contains(if width < 50 { " K" } else { "|K|" }),
            "{line}"
        );
        assert!(
            line.contains(if width < 50 { " L" } else { "|L|" }),
            "{line}"
        );
        for (key, stop) in [('K', true), ('L', false)] {
            let mut ctx = EventCtx::new(AnimationSettings::default());
            toolbar.event(&TuiEvent::Key(KeyEvent::from(Key::Char(key))), &mut ctx);
            assert!(if stop {
                matches!(ctx.messages(), [Msg::StopAll])
            } else {
                matches!(ctx.messages(), [Msg::PurgeAll])
            });
        }
    }
}

#[test]
fn bulk_actions_confirm_captured_targets_across_all_templates_before_submission() {
    tuicore::init();
    for width in [40, 130] {
        for (key, action, expected) in [
            ("stop-all", "stop_instance", vec!["other", "review"]),
            (
                "purge-all",
                "delete_instance",
                vec!["other", "review", "stopped"],
            ),
        ] {
            let mut app = root(AppService::for_tests());
            app.update_snapshot(inventory());
            let (layout, _) = render(&mut app, width);
            let target = layout
                .focus_targets()
                .iter()
                .find(|target| target.path.keys().iter().any(|part| part.as_str() == key))
                .unwrap()
                .clone();
            app.dispatch_focus(&target, true, &mut tuicore::FocusCtx::default());
            let mut ctx = EventCtx::new(AnimationSettings::default());
            app.dispatch_event(
                &tuicore::EventRoute::new(target.path.clone()),
                &TuiEvent::Key(KeyEvent::from(Key::Enter)),
                &mut ctx,
            );
            assert!(matches!(ctx.messages(), [Msg::StopAll] | [Msg::PurgeAll]));
            app.handle_message(
                if key == "stop-all" {
                    Msg::StopAll
                } else {
                    Msg::PurgeAll
                },
                &mut ctx,
            );
            assert!(app.view.is_active());
            assert!(app.service.operations().is_empty());
            let (_, lines) = render(&mut app, width);
            let text = lines
                .iter()
                .map(|line| line.trim().trim_matches('│').trim())
                .collect::<Vec<_>>()
                .join(" ");
            assert!(text.contains("all templates"), "{text}");
            assert!(text.contains("gateway"), "{text}");
            assert!(
                text.contains(if key == "stop-all" {
                    "Stop all (S)"
                } else {
                    "Purge all (P)"
                }),
                "{text}"
            );
            if key == "purge-all" {
                assert!(text.contains("cannot be undone"), "{text}");
            }
            app.handle_message(Msg::Close, &mut ctx);
            assert!(app.service.operations().is_empty());
            app.handle_message(
                if key == "stop-all" {
                    Msg::StopAll
                } else {
                    Msg::PurgeAll
                },
                &mut ctx,
            );

            let mut changed = inventory();
            let mut added = changed.instances[0].clone();
            added.name = "created-after-confirmation".into();
            changed.instances.push(added);
            app.update_snapshot(changed);
            app.handle_message(Msg::Submit, &mut ctx);
            let operations = app.service.operations();
            let mut names = operations
                .iter()
                .map(|operation| {
                    assert_eq!(operation.action, action);
                    operation.name.as_str()
                })
                .collect::<Vec<_>>();
            names.sort();
            assert_eq!(names, expected);
            assert!(!app.view.is_active());
        }
    }
}

#[test]
fn active_operations_do_not_block_bulk_confirmation_or_other_instances() {
    tuicore::init();
    for message in [Msg::StopAll, Msg::PurgeAll] {
        let mut app = root(AppService::for_tests());
        app.update_snapshot(inventory());
        let mut ctx = EventCtx::new(AnimationSettings::default());
        app.service.queue_instance_for_tests("review", "website");
        app.handle_message(message, &mut ctx);
        assert!(app.view.is_active());
        assert!(matches!(
            app.intent,
            Some(super::super::Intent::StopAll(_)) | Some(super::super::Intent::PurgeAll(_))
        ));
        assert_eq!(app.service.operations().len(), 1);
    }

    let mut app = root(AppService::for_tests());
    app.update_snapshot(inventory());
    let mut ctx = EventCtx::new(AnimationSettings::default());
    app.service.queue_instance_for_tests("review", "website");
    app.intent = Some(super::super::Intent::Stop("other".into()));
    app.handle_message(Msg::Submit, &mut ctx);
    assert!(!app.view.is_active());
    assert!(app.intent.is_none());
    assert_eq!(app.service.operations().len(), 2);
}
