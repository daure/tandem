use ratatui::{Terminal, backend::TestBackend, layout::Rect};
use tuicore::{
    AnimationSettings, EventCtx, Key, KeyEvent, KeyModifiers, RenderCtx, TuiEvent, TuiNode,
};

use super::{Msg, root, rows};
use crate::{
    service::AppService,
    store::environments::{EnvironmentSnapshot, Instance, Manifest, Template},
};

fn snapshot() -> EnvironmentSnapshot {
    EnvironmentSnapshot {
        templates: vec![Template {
            name: "website".into(),
            directory: "/tmp/templates/website".into(),
            compose_file: "/tmp/templates/website/compose.yaml".into(),
            manifest_file: "/tmp/templates/website/tandem.json".into(),
            compose_source: "services:\n  web:\n    image: nginx".into(),
            manifest: Manifest::default(),
            error: None,
        }],
        instances: vec![Instance {
            name: "review".into(),
            template: "website".into(),
            template_directory: "/tmp/templates/website".into(),
            workspace: "/tmp/workspaces/review".into(),
            project: "tandem-review".into(),
            services: vec![],
        }],
        ..Default::default()
    }
}

#[test]
fn tree_rows_nest_instances_and_preserve_stale_template_attribution() {
    let mut snapshot = snapshot();
    let rows = rows::from_snapshot(&snapshot);
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[1].parent, Some(rows[0].id.clone()));
    snapshot.templates.clear();
    let rows = rows::from_snapshot(&snapshot);
    assert_eq!(rows.len(), 2);
    assert!(rows[0].label.contains("[missing]"));
    assert_eq!(rows[1].parent, Some(rows[0].id.clone()));
}

#[test]
fn info_hotkey_opens_template_paths_and_compose_source_in_a_dialog() {
    tuicore::init();
    let mut app = root(AppService::for_tests());
    app.tree_mut().set_rows(rows::from_snapshot(&snapshot()));
    app.tree_mut()
        .highlight_id(&"template:/tmp/templates/website".into());
    let mut events = EventCtx::new(AnimationSettings::default());
    let area = Rect::new(0, 0, 130, 40);
    app.layout(area, &mut tuicore::LayoutCtx::new());
    app.event(
        &TuiEvent::Key(KeyEvent {
            code: Key::Char('i'),
            modifiers: KeyModifiers::NONE,
        }),
        &mut events,
    );
    assert!(app.view.first().is_active());
    app.layout(area, &mut tuicore::LayoutCtx::new());
    let mut terminal = Terminal::new(TestBackend::new(area.width, area.height)).unwrap();
    terminal
        .draw(|frame| {
            let mut render = RenderCtx::new();
            app.render(frame, area, &mut render);
            render.flush(frame);
        })
        .unwrap();
    let buffer = terminal.backend().buffer();
    let text: String = (0..area.height)
        .flat_map(|y| {
            (0..area.width).map(move |x| buffer.cell((x, y)).unwrap().symbol().to_owned())
        })
        .collect();
    assert!(text.contains("/tmp/templates/website/compose.yaml"));
    assert!(text.contains("image: nginx"));
    app.handle_message(Msg::Copy("/tmp/templates/website".into()), &mut events);
    assert_eq!(events.clipboard_request(), Some("/tmp/templates/website"));
    app.handle_message(Msg::Close, &mut events);
    assert!(!app.view.first().is_active());
}

#[test]
fn start_dialog_validates_input_without_losing_the_dialog() {
    tuicore::init();
    let mut app = root(AppService::for_tests());
    app.tree_mut().set_rows(rows::from_snapshot(&snapshot()));
    let mut events = EventCtx::new(AnimationSettings::default());
    app.action(1, &mut events);
    app.handle_message(Msg::NameChanged("../unsafe".into()), &mut events);
    app.handle_message(Msg::Submit, &mut events);
    assert!(app.view.first().is_active());
    assert!(app.notice.contains("name must"));
    assert!(app.service.operations().is_empty());
}
