use super::*;
use ratatui::{Terminal, backend::TestBackend};

#[test]
fn conversation_content_loads_asynchronously_and_disposal_cancels_pending_reads() {
    tuicore::init();
    let (sender, receiver) = tokio::sync::oneshot::channel();
    let mut view = Conversation::new(Ok(receiver));
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
    let view = Conversation::new(Ok(receiver));
    drop(view);
    assert!(sender.is_closed());
}
