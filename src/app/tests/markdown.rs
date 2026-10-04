use super::*;
use ratatui::{Terminal, backend::TestBackend};

#[test]
fn conversation_content_loads_asynchronously_and_disposal_cancels_pending_reads() {
    crate::app::tests::init_ui();
    let (sender, receiver) = tokio::sync::oneshot::channel();
    let mut view = Markdown::new("conversation", Ok(receiver), std::convert::identity);
    let area = Rect::new(0, 0, 80, 15);
    view.layout(area, &mut LayoutCtx::new());
    sender
        .send(Ok(
            "# Conversation\n\n## Agent\n\nLatest answer\n\n## You\n\nLatest question".into(),
        ))
        .unwrap();
    assert!(
        view.tick(Duration::ZERO, AnimationSettings::default())
            .changed
    );
    view.layout(area, &mut LayoutCtx::new());
    let mut terminal = Terminal::new(TestBackend::new(80, 15)).unwrap();
    terminal
        .draw(|frame| view.render(frame, area, &mut RenderCtx::new()))
        .unwrap();
    let text: String = terminal
        .backend()
        .buffer()
        .content()
        .iter()
        .map(|cell| cell.symbol())
        .collect();
    assert!(text.contains("Latest answer") && text.contains("Latest question"));
    let (sender, receiver) = tokio::sync::oneshot::channel();
    let view = Markdown::<String>::new("conversation", Ok(receiver), std::convert::identity);
    drop(view);
    assert!(sender.is_closed());
}

#[test]
fn readonly_reports_load_full_contents_and_scroll_without_editing() {
    crate::app::tests::init_ui();
    let (sender, receiver) = tokio::sync::oneshot::channel();
    let mut view = Markdown::readonly("report", Ok(receiver), std::convert::identity);
    let document = (0..40)
        .map(|row| format!("Evidence row {row:02}"))
        .collect::<Vec<_>>()
        .join("\n");
    sender.send(Ok(document.clone())).unwrap();
    let settings = AnimationSettings {
        enabled: false,
        ..Default::default()
    };
    assert!(view.tick(Duration::ZERO, settings).changed);
    let area = Rect::new(0, 0, 80, 15);
    view.layout(area, &mut LayoutCtx::new());
    view.focus(None, true, &mut FocusCtx::default());
    for event in [
        TuiEvent::Key(tuicore::Key::Char('i').into()),
        TuiEvent::Paste("changed".into()),
    ] {
        view.event(&event, &mut EventCtx::default());
    }
    for _ in 0..40 {
        view.event(
            &TuiEvent::Key(tuicore::Key::Down.into()),
            &mut EventCtx::default(),
        );
        view.tick(Duration::ZERO, settings);
    }
    assert_eq!(view.text.current_value(), document);
    assert!(!view.text.insert_mode());
    let mut terminal = Terminal::new(TestBackend::new(area.width, area.height)).unwrap();
    terminal
        .draw(|frame| view.render(frame, area, &mut RenderCtx::new()))
        .unwrap();
    let text: String = terminal
        .backend()
        .buffer()
        .content()
        .iter()
        .map(|cell| cell.symbol())
        .collect();
    assert!(
        text.contains("Report") && text.contains("Evidence row 39"),
        "{text}"
    );
    let (sender, receiver) = tokio::sync::oneshot::channel();
    let view = Markdown::<String, TextareaInput<Msg>>::readonly(
        "report",
        Ok(receiver),
        std::convert::identity,
    );
    drop(view);
    assert!(sender.is_closed());
}
