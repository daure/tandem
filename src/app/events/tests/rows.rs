use super::*;

#[test]
fn event_times_stay_right_aligned_when_headers_and_summaries_are_truncated() {
    crate::app::tests::init_ui();
    let mut event = crate::environments::events::tests::event("release");
    event.occurred_at = Some("2026-01-01T00:00:00Z".into());
    event.event_type = "github.release.published".into();
    event.summary = "Gateway v2.8.1 published".into();
    event.payload = Payload::Generic(serde_json::json!({}));
    let mut row = Record {
        sequence: 4590,
        provider: "github".into(),
        received_at: "2026-10-06T14:32:00+02:00".into(),
        event,
        attempts: vec![],
        acceptances: vec![],
    };
    let timestamp = chrono::DateTime::parse_from_rfc3339(&row.received_at)
        .unwrap()
        .with_timezone(&chrono::Local)
        .format("%d %b %Y %H:%M")
        .to_string();
    for provider in ["github", "开发👩‍💻发布系统"] {
        row.provider = provider.into();
        let full_header = row_text(&row, None).lines[0].to_string();
        for width in [24, 40, 80, 130] {
            let mut text = row_text(&row, Some(width));
            let header = &text.lines[0];
            assert_eq!(text.lines.len(), 2);
            assert_eq!(header.width(), usize::from(width));
            assert!(header.to_string().ends_with(&timestamp));
            assert_eq!(
                header.spans.last().unwrap().style.fg,
                Some(tuicore::theme().text_fg())
            );
            if Line::from(full_header.as_str()).width() > usize::from(width) {
                assert!(header.to_string().contains("..."));
            } else {
                assert_eq!(header.spans[2].content, provider);
                assert!(header.spans[2].style.add_modifier.contains(Modifier::BOLD));
            }
            assert_eq!(text.lines[1].to_string(), "Gateway v2.8.1 published");
            right_suffix(
                &mut text.lines[1].spans,
                "A moment ago",
                Some(width),
                Style::default().fg(tuicore::theme().muted_fg()),
            );
            assert_eq!(text.lines[1].width(), usize::from(width));
            assert!(text.lines[1].to_string().ends_with("A moment ago"));
            assert_eq!(
                text.lines[1].spans.last().unwrap().style.fg,
                Some(tuicore::theme().muted_fg())
            );
            if usize::from(width) >= "Gateway v2.8.1 published".len() + "A moment ago".len() + 2 {
                assert!(
                    text.lines[1]
                        .to_string()
                        .starts_with("Gateway v2.8.1 published")
                );
            } else {
                assert!(text.lines[1].to_string().contains("..."));
            }
        }
    }
}
