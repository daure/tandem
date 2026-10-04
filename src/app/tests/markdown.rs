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
