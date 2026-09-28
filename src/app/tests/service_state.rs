use super::*;
use crate::app::{Intent, instances};

#[test]
fn active_instances_include_stopping_until_shutdown_finishes() {
    use crate::store::environments::Activity;

    let mut app = root(AppService::for_tests());
    app.running_only = true;
    for action in ["stop_instance", "stop_service", "stop_template"] {
        let mut inventory = snapshot();
        inventory.instances[0].runtime.activity = Some(Activity {
            id: "stop".into(),
            name: "review".into(),
            template: Some("website".into()),
            service: None,
            action: action.into(),
            owner_pid: std::process::id(),
            started_at: 1,
            deadline: u64::MAX,
            error: None,
            finished: false,
        });
        for status in ["up", "down (exit 0)"] {
            inventory.instances[0].services[0].status = status.into();
            let rows = app.project_rows(&inventory, &[]);
            let row = rows.iter().find(|row| row.id == "instance:review").unwrap();
            assert_eq!(row.status.as_deref(), Some("Stopping"));
            assert!(row.loading);
            assert!(
                rows.iter()
                    .any(|row| row.id == "template:/tmp/templates/website")
            );
        }
        inventory.instances[0]
            .runtime
            .activity
            .as_mut()
            .unwrap()
            .finished = true;
        assert!(app.project_rows(&inventory, &[]).is_empty());
        app.running_only = false;
        assert!(app.project_rows(&inventory, &[]).iter().any(|row| {
            row.id == "instance:review" && row.status.as_deref() == Some("Stopped")
        }));
        app.running_only = true;
    }
}

fn render(app: &mut crate::app::App) -> String {
    let area = Rect::new(0, 0, 130, 40);
    app.layout(area, &mut tuicore::LayoutCtx::new());
    let mut terminal = Terminal::new(TestBackend::new(area.width, area.height)).unwrap();
    terminal
        .draw(|frame| {
            let mut ctx = RenderCtx::new();
            app.render(frame, area, &mut ctx);
            ctx.flush(frame);
        })
        .unwrap();
    rendered_lines(&terminal, area).join("\n")
}

#[test]
fn service_start_stop_uses_selected_state_and_configured_key_with_confirmation() {
    tuicore::init();
    for (status, running) in [
        ("up", true),
        ("healthy", true),
        ("unhealthy", true),
        ("boot", true),
        ("restarting", true),
        ("paused", true),
        ("down (exit 0)", false),
        ("down (exit 1)", false),
        ("created", false),
    ] {
        for routed in [false, true] {
            for menu in [false, true] {
                let mut inventory = snapshot();
                inventory.instances[0].services[0].status = status.into();
                if !routed {
                    inventory.instances[0].services[0].url = None;
                }
                let mut app = root(AppService::for_tests());
                app.set_rows_for_tests(rows::from_snapshot(&inventory));
                let key = if menu { 's' } else { 'z' };
                app.keys[3] = tuicore::KeySpec::plain(key);
                render(&mut app);
                instances::set_highlighted(&app.instances, Some("service:review:web".into()));
                let mut ctx = EventCtx::new(AnimationSettings::default());
                let title = if running {
                    "Stop service"
                } else {
                    "Start service"
                };
                if menu {
                    app.open_action_menu(&mut ctx);
                    let text = render(&mut app);
                    for label in ["Start service", "Stop service"] {
                        let line = text.lines().find(|line| line.contains(label)).unwrap();
                        assert!(line.trim_end_matches([' ', '│']).ends_with('s'), "{line}");
                    }
                    for character in title.chars() {
                        app.event(
                            &TuiEvent::Key(KeyEvent::from(Key::Char(character))),
                            &mut ctx,
                        );
                    }
                    app.event(&TuiEvent::Key(KeyEvent::from(Key::Enter)), &mut ctx);
                } else {
                    app.event(&TuiEvent::Key(KeyEvent::from(Key::Char(key))), &mut ctx);
                }
                assert!(
                    matches!(&app.intent, Some(Intent::ServiceState { name, service, running: target })
                    if name == "review" && service == "web" && *target != running),
                    "{status}, menu={menu}"
                );
                assert!(app.service.operations().is_empty());
                let text = render(&mut app);
                assert!(
                    text.contains(title) && text.contains("review/web?"),
                    "{text}"
                );
                let route = tuicore::EventRoute::new(tuicore::TreePath::from_keys([
                    tuicore::ChildKey::second(),
                ]));
                app.dispatch_event(
                    &route,
                    &TuiEvent::Key(KeyEvent::from(Key::Char('o'))),
                    &mut ctx,
                );
                assert!(matches!(ctx.messages(), [Msg::Submit]));
                app.handle_message(Msg::Close, &mut ctx);
                assert!(app.service.operations().is_empty());
            }
        }
    }
}
