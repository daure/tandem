use super::*;

#[test]
fn template_guidance_is_an_optional_fourth_tab() {
    init_ui();
    for source in [
        None,
        Some(""),
        Some("# Template guidance\n\nUse the seeded database."),
    ] {
        for width in [56, 130] {
            let mut snapshot = snapshot();
            snapshot.templates[0].guidance_source = source.map(str::to_owned);
            let rows = rows::from_snapshot(&snapshot);
            let mut details = super::super::dialogs::details(&rows[0]);
            let area = Rect::new(0, 0, width, 24);
            details.layout(area, &mut tuicore::LayoutCtx::new());
            let mut terminal = Terminal::new(TestBackend::new(width, area.height)).unwrap();
            terminal
                .draw(|frame| {
                    let mut render = RenderCtx::new();
                    details.render(frame, area, &mut render);
                    render.flush(frame);
                })
                .unwrap();
            let header = rendered_lines(&terminal, area).remove(0);
            assert_eq!(header.contains("Guidance"), source.is_some(), "{header}");
            if source.is_some() {
                assert!(header.find("Manifest").unwrap() < header.find("Guidance").unwrap());
                for _ in 0..3 {
                    details.event(
                        &TuiEvent::Key(KeyEvent::from(Key::Char(']'))),
                        &mut EventCtx::new(AnimationSettings::default()),
                    );
                }
                details.layout(area, &mut tuicore::LayoutCtx::new());
                terminal
                    .draw(|frame| {
                        let mut render = RenderCtx::new();
                        details.render(frame, area, &mut render);
                        render.flush(frame);
                    })
                    .unwrap();
                if source.is_some_and(|source| !source.is_empty()) {
                    let text = rendered_lines(&terminal, area).join("\n");
                    assert!(text.contains("# Template guidance"), "{text}");
                    assert!(text.contains("Use the seeded database."), "{text}");
                }
            }
        }
    }
}

#[test]
fn open_template_tabs_follow_snapshots_and_keep_the_active_file() {
    init_ui();
    for tab_index in 1..=3 {
        let mut app = root(AppService::for_tests());
        let mut inventory = snapshot();
        inventory.templates[0].guidance_source = Some("# Initial guidance".into());
        app.update_snapshot(inventory.clone());
        let row = rows::from_snapshot(&inventory).remove(0);
        let mut events = EventCtx::new(AnimationSettings::default());
        app.open_details(&row, &mut events);
        let route = EventRoute::new(tuicore::TreePath::from_keys([tuicore::ChildKey::second()]));
        for _ in 0..tab_index {
            app.dispatch_event(
                &route,
                &TuiEvent::Key(KeyEvent::from(Key::Char(']'))),
                &mut events,
            );
        }
        inventory.templates[0].compose_source = "services: {updated: {}}".into();
        inventory.templates[0].manifest_source =
            Some("{\"description\":\"Updated manifest\"}".into());
        inventory.templates[0].guidance_source = Some("# Updated guidance".into());
        app.update_snapshot(inventory.clone());
        let area = Rect::new(0, 0, 130, 40);
        let mut terminal = Terminal::new(TestBackend::new(area.width, area.height)).unwrap();
        let mut render = |app: &mut App| {
            app.layout(area, &mut tuicore::LayoutCtx::new());
            terminal
                .draw(|frame| {
                    let mut ctx = RenderCtx::new();
                    app.render(frame, area, &mut ctx);
                    ctx.flush(frame);
                })
                .unwrap();
            rendered_lines(&terminal, area).join("\n")
        };
        let expected = match tab_index {
            1 => "services: {updated: {}}",
            2 => "Updated manifest",
            _ => "# Updated guidance",
        };
        let text = render(&mut app);
        assert!(text.contains(expected), "{text}");
        if tab_index > 1 {
            inventory.templates[0].compose_file.clear();
            inventory.templates[0].compose_source.clear();
            app.update_snapshot(inventory.clone());
            let text = render(&mut app);
            assert!(text.contains(expected), "{text}");
        }
        if tab_index == 3 {
            inventory.templates[0].guidance_source = None;
            app.update_snapshot(inventory.clone());
            let text = render(&mut app);
            assert!(text.contains("Template details"), "{text}");
            assert!(text.contains("Directory"), "{text}");
        }
        inventory.templates.clear();
        inventory.instances.clear();
        app.update_snapshot(inventory);
        assert!(!app.view.is_active());
        assert!(
            app.tick(std::time::Duration::ZERO, AnimationSettings::default())
                .layout
        );
    }
}
