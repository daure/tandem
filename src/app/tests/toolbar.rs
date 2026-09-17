use super::*;

#[test]
fn refresh_button_stays_top_right_and_shows_its_hotkey_at_both_sizes() {
    tuicore::init();
    let mut app = root(AppService::for_tests());
    for width in [130, 99, 40, 100, 150] {
        let area = Rect::new(0, 0, width, 30);
        let mut layout = tuicore::LayoutCtx::new();
        app.layout(area, &mut layout);
        let target = layout
            .focus_targets()
            .iter()
            .find(|target| {
                target
                    .path
                    .keys()
                    .iter()
                    .any(|key| key.as_str() == "refresh")
            })
            .unwrap();
        assert_eq!(target.area.right(), area.right());
        assert_eq!(target.area.y, 1);
        let mut terminal = Terminal::new(TestBackend::new(width, area.height)).unwrap();
        terminal
            .draw(|frame| {
                let mut render = RenderCtx::new();
                app.render(frame, area, &mut render);
                render.flush(frame);
            })
            .unwrap();
        let lines = rendered_lines(&terminal, area);
        let label: String = lines[usize::from(target.area.y)]
            .chars()
            .skip(usize::from(target.area.x))
            .take(usize::from(target.area.width))
            .collect();
        assert_eq!(
            label.trim(),
            if width < 100 {
                "󰑓 R"
            } else {
                "󰑓 Refresh"
            }
        );
    }
}

#[test]
fn toolbar_refresh_activation_and_capital_r_request_manual_refresh_at_both_sizes() {
    tuicore::init();
    for width in [130, 40] {
        let mut app = root(AppService::for_tests());
        let mut layout = tuicore::LayoutCtx::new();
        app.layout(Rect::new(0, 0, width, 30), &mut layout);
        let target = layout
            .focus_targets()
            .iter()
            .find(|target| {
                target
                    .path
                    .keys()
                    .iter()
                    .any(|key| key.as_str() == "refresh")
            })
            .unwrap()
            .clone();
        app.dispatch_focus(&target, true, &mut tuicore::FocusCtx::default());
        for key in [Key::Enter, Key::Char('R')] {
            let mut ctx = EventCtx::new(AnimationSettings::default());
            app.dispatch_event(
                &tuicore::EventRoute::new(target.path.clone()),
                &TuiEvent::Key(KeyEvent::from(key)),
                &mut ctx,
            );
            assert!(matches!(ctx.messages(), [Msg::Refresh]));
            app.handle_message(Msg::Refresh, &mut ctx);
            assert!(app.manual_refresh.is_some());
            app.manual_refresh = None;
        }
        app.event(
            &TuiEvent::Key(KeyEvent::from(Key::Char('R'))),
            &mut EventCtx::new(AnimationSettings::default()),
        );
        assert!(app.manual_refresh.is_some());
    }
}

#[test]
fn escape_from_toolbar_returns_focus_to_the_data_view() {
    tuicore::init();
    let mut app = root(AppService::for_tests());
    let area = Rect::new(0, 0, 130, 30);
    let mut layout = tuicore::LayoutCtx::new();
    app.layout(area, &mut layout);
    let toolbar_targets = layout
        .focus_targets()
        .iter()
        .filter(|target| {
            target
                .path
                .keys()
                .iter()
                .any(|key| matches!(key.as_str(), "new-template" | "refresh"))
        })
        .cloned()
        .collect::<Vec<_>>();
    assert_eq!(toolbar_targets.len(), 2);

    for target in toolbar_targets {
        for key in [
            KeyEvent::from(Key::Esc),
            KeyEvent {
                code: Key::Char('['),
                modifiers: KeyModifiers::CONTROL,
            },
        ] {
            let mut ctx = EventCtx::new(AnimationSettings::default());
            app.dispatch_event(
                &tuicore::EventRoute::new(target.path.clone()),
                &TuiEvent::Key(key),
                &mut ctx,
            );
            assert_eq!(ctx.focus_request(), Some(&super::super::initial_focus()));
        }
    }
}

#[test]
fn escape_in_the_data_view_keeps_its_focus() {
    tuicore::init();
    let mut app = root(AppService::for_tests());
    let area = Rect::new(0, 0, 130, 30);
    let mut layout = tuicore::LayoutCtx::new();
    app.layout(area, &mut layout);
    let data_view = layout
        .focus_targets()
        .iter()
        .find(|target| target.id.as_str() == "environments")
        .unwrap()
        .clone();

    for key in [
        KeyEvent::from(Key::Esc),
        KeyEvent {
            code: Key::Char('['),
            modifiers: KeyModifiers::CONTROL,
        },
    ] {
        let mut ctx = EventCtx::new(AnimationSettings::default());
        let outcome = app.dispatch_event(
            &tuicore::EventRoute::new(data_view.path.clone()),
            &TuiEvent::Key(key),
            &mut ctx,
        );
        assert_eq!(outcome, tuicore::EventOutcome::Handled);
        assert_eq!(ctx.focus_request(), Some(&super::super::initial_focus()));
    }
}

#[test]
fn compact_refresh_button_honors_a_configured_hotkey() {
    tuicore::init();
    let mut toolbar = crate::app::toolbar::Toolbar::new('T', 'G');
    let area = Rect::new(0, 0, 40, 1);
    toolbar.layout(area, &mut tuicore::LayoutCtx::new());
    let mut terminal = Terminal::new(TestBackend::new(40, 1)).unwrap();
    terminal
        .draw(|frame| toolbar.render(frame, area, &mut RenderCtx::new()))
        .unwrap();
    assert!(
        rendered_lines(&terminal, area)[0]
            .trim_end()
            .ends_with("󰑓 G")
    );
    let mut ctx = EventCtx::new(AnimationSettings::default());
    toolbar.event(&TuiEvent::Key(KeyEvent::from(Key::Char('G'))), &mut ctx);
    assert!(matches!(ctx.messages(), [Msg::Refresh]));
}
