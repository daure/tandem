use super::*;

#[test]
fn detail_search_matches_property_names_and_multiline_values() {
    tuicore::init();
    let mut snapshot = snapshot();
    snapshot.templates[0].manifest.description = "First line\nneonquartz".into();
    let rows = rows::from_snapshot(&snapshot);
    for (row, value_query, expected_property) in [
        (&rows[0], "neonquartz", "Description"),
        (&rows[1], "review", "Instance"),
        (&rows[2], "nginx", "Image"),
    ] {
        for (query, property) in [
            ("Memory limit", "Memory limit"),
            (value_query, expected_property),
        ] {
            let mut modal = crate::app::dialogs::details(row);
            let area = Rect::new(0, 0, 90, 24);
            let mut layout = tuicore::LayoutCtx::new();
            modal.layout(area, &mut layout);
            let target = layout
                .focus_targets()
                .iter()
                .find(|target| target.id.as_str() == "data-view")
                .unwrap()
                .clone();
            modal.dispatch_focus(&target, true, &mut tuicore::FocusCtx::default());
            let route = tuicore::EventRoute::new(target.path);
            for character in format!("/{query}").chars() {
                let mut ctx = EventCtx::new(AnimationSettings::default());
                modal.dispatch_event(
                    &route,
                    &TuiEvent::Key(KeyEvent::from(Key::Char(character))),
                    &mut ctx,
                );
                assert!(
                    ctx.messages().is_empty(),
                    "search input closed the details tab: {query}"
                );
            }
            modal.layout(area, &mut tuicore::LayoutCtx::new());
            let mut terminal = Terminal::new(TestBackend::new(area.width, area.height)).unwrap();
            terminal
                .draw(|frame| {
                    let mut render = RenderCtx::new();
                    modal.render(frame, area, &mut render);
                    render.flush(frame);
                })
                .unwrap();
            let lines = rendered_lines(&terminal, area);
            assert!(
                lines.iter().any(|line| line.contains(property)),
                "{} / {query}: {lines:#?}",
                row.id
            );
            assert!(
                !lines.iter().any(|line| line.contains("Resource refresh")),
                "unmatched properties remain visible: {lines:#?}"
            );
            if query == "neonquartz" {
                assert!(
                    lines.iter().any(|line| line.contains("First line")),
                    "{lines:#?}"
                );
            }
        }
    }
}

#[test]
fn tree_search_matches_service_labels_and_resource_values() {
    tuicore::init();
    let mut snapshot = snapshot();
    snapshot.instances[0].services[0].usage = Some(crate::store::environments::ResourceUsage {
        memory_bytes: 2 * 1048576,
        cpu_basis_points: Some(300),
        sampled_at_unix_seconds: 42,
    });
    for query in ["localhost", "2 MiB"] {
        let state = crate::app::instances::state(rows::from_snapshot(&snapshot));
        let mut tree = crate::app::instances::Instances::new(state.clone());
        let area = Rect::new(0, 0, 110, 18);
        tree.layout(area, &mut tuicore::LayoutCtx::new());
        tree.focus(None, true, &mut tuicore::FocusCtx::default());
        for character in format!("/{query}").chars() {
            tree.event(
                &TuiEvent::Key(KeyEvent::from(Key::Char(character))),
                &mut EventCtx::new(AnimationSettings::default()),
            );
        }
        assert!(crate::app::instances::is_searching(&state));
        tree.layout(area, &mut tuicore::LayoutCtx::new());
        let mut terminal = Terminal::new(TestBackend::new(area.width, area.height)).unwrap();
        terminal
            .draw(|frame| {
                let mut render = RenderCtx::new();
                tree.render(frame, area, &mut render);
                render.flush(frame);
            })
            .unwrap();
        let lines = rendered_lines(&terminal, area);
        assert!(
            lines.iter().any(|line| line.contains("󰖟 web")),
            "{query}: {lines:#?}"
        );
        assert!(
            lines.iter().any(|line| line.contains("󰑹 2 MiB")),
            "{lines:#?}"
        );
    }
}
