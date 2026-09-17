use super::*;
use crate::app::{Intent, instances};

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
fn restart_key_and_menu_confirm_the_selected_instance_or_service() {
    tuicore::init();
    let mut inventory = snapshot();
    let mut database = inventory.instances[0].services[0].clone();
    database.name = "db".into();
    database.url = None;
    database.port = None;
    inventory.instances[0].services.push(database);
    for (selected, service, title) in [
        ("instance:review", None, "Restart instance"),
        ("service:review:web", Some("web"), "Restart service"),
        ("service:review:db", Some("db"), "Restart service"),
    ] {
        for menu in [false, true] {
            let mut app = root(AppService::for_tests());
            app.set_rows_for_tests(rows::from_snapshot(&inventory));
            render(&mut app);
            instances::set_highlighted(&app.instances, Some(selected.into()));
            if menu {
                app.open_action_menu(&mut EventCtx::new(AnimationSettings::default()));
                let text = render(&mut app);
                let line = text.lines().find(|line| line.contains(title)).unwrap();
                assert!(line.trim_end_matches([' ', '│']).ends_with('r'), "{line}");
                for character in title.chars() {
                    app.event(
                        &TuiEvent::Key(KeyEvent::from(Key::Char(character))),
                        &mut EventCtx::new(AnimationSettings::default()),
                    );
                }
                app.event(
                    &TuiEvent::Key(KeyEvent::from(Key::Enter)),
                    &mut EventCtx::new(AnimationSettings::default()),
                );
            } else {
                app.event(
                    &TuiEvent::Key(KeyEvent::from(Key::Char('r'))),
                    &mut EventCtx::new(AnimationSettings::default()),
                );
            }
            assert!(
                matches!(&app.intent, Some(Intent::Restart { name, service: target }) if name == "review" && target.as_deref() == service),
                "selected={selected}, menu={menu}, actual={:?}",
                app.selected().map(|row| row.id)
            );
            assert!(app.service.operations().is_empty());
            let text = render(&mut app);
            assert!(text.contains(title), "{text}");
            assert!(
                text.contains(&service.map_or_else(
                    || "Restart review?".into(),
                    |service| format!("Restart review/{service}?")
                )),
                "{text}"
            );
            let mut ctx = EventCtx::new(AnimationSettings::default());
            let route = tuicore::EventRoute::new(tuicore::TreePath::from_keys([
                tuicore::ChildKey::first(),
                tuicore::ChildKey::second(),
            ]));
            app.dispatch_event(
                &route,
                &TuiEvent::Key(KeyEvent::from(Key::Char('r'))),
                &mut ctx,
            );
            assert!(matches!(ctx.messages(), [Msg::Submit]));
            app.handle_message(Msg::Close, &mut EventCtx::new(AnimationSettings::default()));
            assert!(app.service.operations().is_empty());
        }
    }
}

#[test]
fn template_selection_ignores_restart_and_uppercase_r_refreshes() {
    tuicore::init();
    let mut app = root(AppService::for_tests());
    app.set_rows_for_tests(rows::from_snapshot(&snapshot()));
    app.event(
        &TuiEvent::Key(KeyEvent::from(Key::Char('r'))),
        &mut EventCtx::new(AnimationSettings::default()),
    );
    assert!(app.intent.is_none());
    assert!(!app.view.first().is_active());
    assert!(app.manual_refresh.is_none());
    instances::set_highlighted(&app.instances, Some("instance:review".into()));
    app.event(
        &TuiEvent::Key(KeyEvent::from(Key::Char('R'))),
        &mut EventCtx::new(AnimationSettings::default()),
    );
    assert!(app.intent.is_none());
    assert!(app.manual_refresh.is_some());
}
