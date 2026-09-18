use super::*;

#[test]
fn template_guidance_is_an_optional_fourth_tab() {
    tuicore::init();
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
