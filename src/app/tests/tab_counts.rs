use super::*;
use crate::store::providers::{Snapshot, Status, Stream};

#[test]
fn tab_counts_follow_active_rules_and_collecting_streams_and_hide_zero_counts() {
    init_ui();
    let runtime = tokio::runtime::Runtime::new().unwrap();
    for opencode_enabled in [true, false] {
        let service = AppService::for_tests();
        runtime
            .block_on(service.set_opencode_enabled(opencode_enabled).unwrap())
            .unwrap()
            .unwrap();
        let mut app = crate::app::root(service);
        app.update_snapshot(snapshot());
        let mut rules = crate::store::rules::Snapshot {
            rules: (0..4)
                .map(|index| {
                    let mut rule = super::rules::rule(&format!("rule-{index}"));
                    rule.definition.enabled = index < 3;
                    rule
                })
                .collect(),
            ..Default::default()
        };
        let mut providers = Snapshot {
            providers: [
                Status::Running,
                Status::Running,
                Status::Running,
                Status::Paused,
                Status::Stopped,
                Status::Starting,
                Status::NotStarted,
                Status::Unknown,
            ]
            .into_iter()
            .enumerate()
            .map(|(index, status)| super::providers::provider(&format!("provider-{index}"), status))
            .collect(),
            error: None,
        };
        for (index, provider) in providers.providers[..2].iter_mut().enumerate() {
            provider.streams = [
                Status::Running,
                if index == 0 {
                    Status::Running
                } else {
                    Status::Stopped
                },
                Status::Starting,
                Status::Paused,
                Status::NotStarted,
                Status::Unknown,
            ]
            .into_iter()
            .enumerate()
            .map(|(index, status)| Stream {
                name: format!("stream-{index}"),
                profile: Some("generic".into()),
                controllable: true,
                enabled: status != Status::Stopped,
                status,
                operation: None,
                error: None,
                total: 0,
                handovers: 0,
            })
            .collect();
        }
        let selected = app.providers_tab_index();
        app.tabs_mut().select_index(selected);
        app.sync_overview_tab(&mut EventCtx::default());
        for (active_rules, running_streams, expected) in [
            (3, 3, "Rules (3) · Streams (3)"),
            (2, 2, "Rules (2) · Streams (2)"),
            (1, 1, "Rules (1) · Streams (1)"),
            (0, 0, "Rules · Streams"),
        ] {
            for (index, rule) in rules.rules.iter_mut().enumerate() {
                rule.definition.enabled = index < active_rules;
            }
            for (index, stream) in providers
                .providers
                .iter_mut()
                .flat_map(|provider| &mut provider.streams)
                .filter(|stream| stream.status == Status::Running)
                .enumerate()
            {
                stream.status = if index < running_streams {
                    Status::Running
                } else {
                    Status::Stopped
                };
            }
            app.pages_mut().update_rules(rules.clone());
            app.pages_mut().update_providers(providers.clone());
            let area = Rect::new(0, 0, 130, 30);
            app.layout(area, &mut tuicore::LayoutCtx::new());
            let mut terminal = Terminal::new(TestBackend::new(area.width, area.height)).unwrap();
            terminal
                .draw(|frame| {
                    let mut ctx = RenderCtx::new();
                    app.render(frame, area, &mut ctx);
                    ctx.flush(frame);
                })
                .unwrap();
            let header = rendered_lines(&terminal, area)[0].clone();
            assert!(header.contains(expected), "{header}");
            assert_eq!(app.tabs_mut().selected_index(), selected);
            assert!(app.providers_active);
            assert!(!app.sync_tab_counts());
        }
    }
}
