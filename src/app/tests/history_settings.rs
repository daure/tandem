use super::*;

#[test]
fn creation_history_setting_is_visible_checked_and_keyboard_toggleable() {
    tuicore::init();
    let mut app = root(AppService::for_tests());
    let settings = AnimationSettings {
        enabled: false,
        ..AnimationSettings::default()
    };
    let area = Rect::new(0, 0, 130, 40);
    app.handle_message(Msg::OpenSettings, &mut EventCtx::new(settings));
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
                .any(|key| key.as_str() == "clear-opencode-history")
        })
        .unwrap();
    let mut ctx = EventCtx::new(settings);
    app.dispatch_event(
        &EventRoute::new(target.path.clone()),
        &TuiEvent::Key(KeyEvent::from(Key::Enter)),
        &mut ctx,
    );
    assert!(matches!(
        ctx.messages(),
        [Msg::SetClearOpencodeHistory(false)]
    ));
    app.handle_message(
        Msg::SetClearOpencodeHistory(false),
        &mut EventCtx::new(settings),
    );
    app.service.flush_settings();
    assert!(!app.service.clear_opencode_history());
    let mut terminal = Terminal::new(TestBackend::new(area.width, area.height)).unwrap();
    terminal
        .draw(|frame| {
            let mut render = RenderCtx::new();
            app.render(frame, area, &mut render);
            render.flush(frame);
        })
        .unwrap();
    let rendered = rendered_lines(&terminal, area).join("\n");
    assert!(
        rendered.contains("Clear OpenCode history on new instance"),
        "{rendered}"
    );
}
