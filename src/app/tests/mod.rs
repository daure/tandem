use ratatui::{Terminal, backend::TestBackend, layout::Rect};
use tuicore::{
    AnimationSettings, EventCtx, Key, KeyEvent, KeyModifiers, RenderCtx, TuiEvent, TuiNode,
};

use super::{Msg, root, rows};
use crate::{
    service::AppService,
    store::environments::{EnvironmentSnapshot, Instance, InstanceService, Manifest, Template},
};

mod input_routing;
mod labels;
mod operations;
mod properties;
mod refresh;
mod resources;
mod restart;
mod service_state;
mod template_actions;
mod toolbar;

fn snapshot() -> EnvironmentSnapshot {
    EnvironmentSnapshot {
        templates: vec![Template {
            name: "website".into(),
            directory: "/tmp/templates/website".into(),
            compose_file: "/tmp/templates/website/compose.yaml".into(),
            manifest_file: "/tmp/templates/website/tandem.json".into(),
            compose_source: "services:\n  web:\n    image: nginx".into(),
            manifest_source: Some(
                "{\n  \"description\": \"Website manifest\",\n  \"routes\": {}\n}\n".into(),
            ),
            manifest: Manifest::default(),
            error: None,
        }],
        instances: vec![Instance {
            name: "review".into(),
            template: "website".into(),
            template_directory: "/tmp/templates/website".into(),
            workspace: "/tmp/workspaces/review".into(),
            project: "tandem-review".into(),
            pending: false,
            services: vec![InstanceService {
                name: "web".into(),
                container_id: "container-id".into(),
                status: "up".into(),
                one_shot: false,
                image: Some("nginx:latest".into()),
                health: Some("healthy".into()),
                restart_policy: Some("unless-stopped".into()),
                restart_count: 2,
                created_at: Some("2026-09-16T12:00:00Z".into()),
                started_at: Some("2026-09-16T12:01:00Z".into()),
                port: Some(8080),
                url: Some("http://localhost:9876/review/web/".into()),
                usage: None,
                memory_limit_bytes: None,
                volumes: Vec::new(),
            }],
        }],
        ..Default::default()
    }
}

fn rendered_lines(terminal: &Terminal<TestBackend>, area: Rect) -> Vec<String> {
    let buffer = terminal.backend().buffer();
    (0..area.height)
        .map(|y| {
            (0..area.width)
                .map(|x| buffer.cell((x, y)).unwrap().symbol())
                .collect()
        })
        .collect()
}

#[test]
fn tree_rows_show_routed_services_without_gateway_children() {
    let mut snapshot = snapshot();
    snapshot.instances[0].services.push(InstanceService {
        name: "db".into(),
        container_id: "database-container-id".into(),
        status: "healthy".into(),
        one_shot: false,
        image: Some("postgres:17.5-alpine".into()),
        health: Some("healthy".into()),
        restart_policy: Some("unless-stopped".into()),
        restart_count: 0,
        created_at: Some("2026-09-16T12:00:00Z".into()),
        started_at: Some("2026-09-16T12:01:00Z".into()),
        port: None,
        url: None,
        usage: None,
        memory_limit_bytes: None,
        volumes: Vec::new(),
    });
    let rows = rows::from_snapshot(&snapshot);
    assert_eq!(rows.len(), 4);
    assert_eq!(rows[0].label, "website\n 1");
    assert_eq!(rows[0].icon, "󰠲");
    assert_eq!(rows[1].parent, Some(rows[0].id.clone()));
    assert_eq!(rows[1].label, "review · Running\n/tmp/workspaces/review");
    assert_eq!(rows[1].icon, "");
    assert_eq!(rows[2].parent, Some(rows[1].id.clone()));
    assert_eq!(
        rows[2].label,
        "web\nhttp://localhost:9876/review/web/ · port 8080"
    );
    assert_eq!(rows[2].icon, "󰖟");
    assert_eq!(rows[3].parent, Some(rows[1].id.clone()));
    assert_eq!(rows[3].label, "db\npostgres:17.5-alpine");
    assert_eq!(rows[3].icon, "󰒋");
    assert_eq!(
        rows[2].gateway_url.as_deref(),
        Some("http://localhost:9876/review/web/")
    );
    snapshot.templates.clear();
    let rows = rows::from_snapshot(&snapshot);
    assert_eq!(rows.len(), 4);
    assert!(rows[0].label.contains("[missing]"));
    assert_eq!(rows[1].parent, Some(rows[0].id.clone()));
}

#[test]
fn row_backgrounds_follow_tree_order() {
    let mut snapshot = snapshot();
    let mut docs = snapshot.templates[0].clone();
    docs.name = "docs".into();
    docs.directory = "/tmp/templates/docs".into();
    snapshot.templates.push(docs);

    let mut guide = snapshot.instances[0].clone();
    guide.name = "guide".into();
    guide.template = "docs".into();
    guide.template_directory = "/tmp/templates/docs".into();
    snapshot.instances.push(guide);

    let rows = rows::from_snapshot(&snapshot);
    let background = |id| {
        rows.iter()
            .find(|row| row.id == id)
            .unwrap()
            .alternate_background
    };

    assert!(background("template:/tmp/templates/docs"));
    assert!(!background("instance:guide"));
    assert!(background("service:guide:web"));
    assert!(!background("template:/tmp/templates/website"));
    assert!(background("instance:review"));
    assert!(!background("service:review:web"));
}

#[test]
fn template_rows_with_instances_precede_empty_templates_and_sort_by_name() {
    let mut snapshot = snapshot();
    let mut alpha = snapshot.templates[0].clone();
    alpha.name = "alpha".into();
    alpha.directory = "/tmp/templates/alpha".into();
    snapshot.templates.push(alpha);
    let mut docs = snapshot.templates[0].clone();
    docs.name = "docs".into();
    docs.directory = "/tmp/templates/docs".into();
    snapshot.templates.push(docs);

    let rows = rows::from_snapshot(&snapshot);
    let templates = rows
        .iter()
        .filter(|row| row.parent.is_none())
        .map(|row| row.template.as_str())
        .collect::<Vec<_>>();

    assert_eq!(templates, ["website", "alpha", "docs"]);
}

#[test]
fn declared_one_shots_are_hidden_in_the_tree_and_available_in_instance_details() {
    let mut snapshot = snapshot();
    let mut setup = snapshot.instances[0].services[0].clone();
    setup.name = "repo-sync".into();
    setup.one_shot = true;
    setup.port = None;
    setup.url = None;
    snapshot.instances[0].services.push(setup);

    for status in ["up", "exited 0", "down (exit 1)"] {
        snapshot.instances[0].services[1].status = status.into();
        let rows = rows::from_snapshot(&snapshot);
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[2].id, "service:review:web");
        assert!(
            rows[1]
                .details
                .iter()
                .any(|row| row.name == "repo-sync / Status" && row.value == status)
        );
    }

    snapshot.templates.clear();
    assert_eq!(rows::from_snapshot(&snapshot).len(), 3);

    snapshot.instances[0].services[1].one_shot = false;
    let rows = rows::from_snapshot(&snapshot);
    assert_eq!(rows[3].id, "service:review:repo-sync");
}

#[test]
fn enter_opens_the_selected_routed_service() {
    tuicore::init();
    let mut app = root(AppService::for_tests());
    app.set_rows_for_tests(rows::from_snapshot(&snapshot()));
    super::instances::set_highlighted(&app.instances, Some("service:review:web".into()));

    app.event(
        &TuiEvent::Key(KeyEvent::from(Key::Enter)),
        &mut EventCtx::new(AnimationSettings::default()),
    );

    assert_eq!(
        app.service.opened_system_targets(),
        ["http://localhost:9876/review/web/"]
    );
}

#[test]
fn routed_service_action_menu_opens_the_gateway_in_the_browser() {
    tuicore::init();
    let mut app = root(AppService::for_tests());
    app.set_rows_for_tests(rows::from_snapshot(&snapshot()));
    super::instances::set_highlighted(&app.instances, Some("service:review:web".into()));
    let mut events = EventCtx::new(AnimationSettings::default());

    app.event(&TuiEvent::Key(KeyEvent::from(Key::Char('.'))), &mut events);
    assert!(app.menu_layer().is_active());
    app.event(&TuiEvent::Key(KeyEvent::from(Key::Enter)), &mut events);

    assert_eq!(
        app.service.opened_system_targets(),
        ["http://localhost:9876/review/web/"]
    );
}

#[test]
fn enter_opens_the_selected_instance_workspace() {
    tuicore::init();
    let mut app = root(AppService::for_tests());
    app.set_rows_for_tests(rows::from_snapshot(&snapshot()));
    super::instances::set_highlighted(&app.instances, Some("instance:review".into()));

    app.event(
        &TuiEvent::Key(KeyEvent::from(Key::Enter)),
        &mut EventCtx::new(AnimationSettings::default()),
    );

    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while app.service.opened_system_targets().is_empty() {
        assert!(
            std::time::Instant::now() < deadline,
            "workspace opener did not finish"
        );
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    assert_eq!(
        app.service.opened_system_targets(),
        ["/tmp/workspaces/review"]
    );
}

#[test]
fn data_view_starts_with_templates_expanded_and_instances_collapsed() {
    tuicore::init();
    let mut app = root(AppService::for_tests());
    app.set_rows_for_tests(rows::from_snapshot(&snapshot()));
    let area = Rect::new(0, 0, 130, 40);
    app.layout(area, &mut tuicore::LayoutCtx::new());
    let mut terminal = Terminal::new(TestBackend::new(area.width, area.height)).unwrap();
    terminal
        .draw(|frame| {
            let mut render = RenderCtx::new();
            app.render(frame, area, &mut render);
            render.flush(frame);
        })
        .unwrap();

    let text = rendered_lines(&terminal, area).join("");
    assert!(text.contains("review"));
    assert!(!text.contains("http://localhost:9876/review/web/"));
    assert!(!text.contains("Templates / instances"));
    assert!(!text.contains("Status"));
}

#[test]
fn details_hotkey_opens_the_selected_template_in_bottom_tabs() {
    tuicore::init();
    let mut app = root(AppService::for_tests());
    app.set_rows_for_tests(rows::from_snapshot(&snapshot()));
    let mut events = EventCtx::new(AnimationSettings::default());
    let area = Rect::new(0, 0, 130, 40);
    app.layout(area, &mut tuicore::LayoutCtx::new());
    app.event(
        &TuiEvent::Key(KeyEvent {
            code: Key::Char('v'),
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
    let text = rendered_lines(&terminal, area).join("");
    assert!(text.contains("Metadata"));
    assert!(text.contains("Compose"));
    assert!(text.contains("Manifest"));
    assert!(text.contains("website"));
    assert!(text.contains("/tmp/templates/website"));
    let route = tuicore::EventRoute::new(tuicore::TreePath::from_keys([
        tuicore::ChildKey::first(),
        tuicore::ChildKey::second(),
    ]));
    app.dispatch_event(
        &route,
        &TuiEvent::Key(KeyEvent::from(Key::Char(']'))),
        &mut EventCtx::new(AnimationSettings::default()),
    );
    app.layout(area, &mut tuicore::LayoutCtx::new());
    terminal
        .draw(|frame| {
            let mut render = RenderCtx::new();
            app.render(frame, area, &mut render);
            render.flush(frame);
        })
        .unwrap();
    let text = rendered_lines(&terminal, area).join("");
    assert!(text.contains("services:"), "{text}");
    assert!(text.contains("image: nginx"), "{text}");
    app.dispatch_event(
        &route,
        &TuiEvent::Key(KeyEvent::from(Key::Char(']'))),
        &mut EventCtx::new(AnimationSettings::default()),
    );
    app.layout(area, &mut tuicore::LayoutCtx::new());
    terminal
        .draw(|frame| {
            let mut render = RenderCtx::new();
            app.render(frame, area, &mut render);
            render.flush(frame);
        })
        .unwrap();
    let text = rendered_lines(&terminal, area).join("");
    assert!(
        text.contains("\"description\": \"Website manifest\""),
        "{text}"
    );
    assert!(text.contains("\"routes\": {}"), "{text}");
    let mut close = EventCtx::new(AnimationSettings::default());
    app.dispatch_event(&route, &TuiEvent::Key(KeyEvent::from(Key::Esc)), &mut close);
    assert!(matches!(close.messages(), [Msg::Close]));
    app.handle_message(Msg::Close, &mut events);
    assert!(!app.view.first().is_active());
}

#[test]
fn detail_tabs_keep_the_header_and_close_control_above_the_content() {
    tuicore::init();
    let rows = rows::from_snapshot(&snapshot());
    for width in [56, 130] {
        for (row, title, content, value) in [
            (&rows[0], "Metadata", "Template", "website"),
            (&rows[1], "Details", "Instance", "review"),
            (&rows[2], "Details", "Service", "web"),
        ] {
            let mut details = super::dialogs::details(row);
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
            let lines = rendered_lines(&terminal, area);
            assert!(
                lines[0].contains(title),
                "{} at {width}: {lines:#?}",
                row.id
            );
            assert!(lines[0].contains("┤x├"), "{lines:#?}");
            assert!(lines[1].contains("Search"), "{lines:#?}");
            assert!(lines[2].contains(content), "{lines:#?}");
            assert!(lines[2].contains(value), "{lines:#?}");
            if row.parent.is_none() {
                assert!(lines[0].contains("Compose"));
                assert!(lines[0].contains("Manifest"));
            }
        }
    }
}

#[test]
fn new_instance_dialog_validates_input_without_losing_the_dialog() {
    tuicore::init();
    let mut app = root(AppService::for_tests());
    app.set_rows_for_tests(rows::from_snapshot(&snapshot()));
    let mut events = EventCtx::new(AnimationSettings::default());
    app.action(1, &mut events);
    app.handle_message(Msg::NameChanged("../unsafe".into()), &mut events);
    app.event(
        &TuiEvent::Key(KeyEvent {
            code: Key::Enter,
            modifiers: KeyModifiers::CONTROL,
        }),
        &mut events,
    );
    assert!(app.view.first().is_active());
    assert!(app.service.operations().is_empty());
}

#[test]
fn entered_names_are_preserved() {
    let mut app = root(AppService::for_tests());
    app.handle_message(
        Msg::NameChanged("Feature Branch".into()),
        &mut EventCtx::new(AnimationSettings::default()),
    );

    assert_eq!(app.name, "Feature Branch");
}

#[test]
fn template_name_entry_contains_only_a_wide_input() {
    tuicore::init();
    let mut app = root(AppService::for_tests());
    app.set_rows_for_tests(rows::from_snapshot(&snapshot()));
    app.action(2, &mut EventCtx::new(AnimationSettings::default()));
    let area = Rect::new(0, 0, 130, 40);
    app.layout(area, &mut tuicore::LayoutCtx::new());
    let mut terminal = Terminal::new(TestBackend::new(area.width, area.height)).unwrap();
    terminal
        .draw(|frame| {
            let mut render = RenderCtx::new();
            app.render(frame, area, &mut render);
            render.flush(frame);
        })
        .unwrap();

    let lines = rendered_lines(&terminal, area);
    let title = lines
        .iter()
        .find(|line| line.contains("New template"))
        .unwrap();
    let dialog_width = title.trim().chars().count();
    assert!(dialog_width >= 50, "{title}");
    assert!(
        !lines
            .iter()
            .any(|line| line.contains("Create a template folder."))
    );
}

#[test]
fn details_hotkey_uses_full_width_on_mobile_and_sixty_percent_on_desktop() {
    assert_eq!(super::details_width_percent(99), 100);
    assert_eq!(super::details_width_percent(100), 60);
    tuicore::init();
    let mut app = root(AppService::for_tests());
    app.set_rows_for_tests(rows::from_snapshot(&snapshot()));
    app.action(0, &mut EventCtx::new(AnimationSettings::default()));
    for width in [130, 56, 130] {
        let area = Rect::new(0, 0, width, 40);
        app.layout(area, &mut tuicore::LayoutCtx::new());
        let mut terminal = Terminal::new(TestBackend::new(width, area.height)).unwrap();
        terminal
            .draw(|frame| {
                let mut render = RenderCtx::new();
                app.render(frame, area, &mut render);
                render.flush(frame);
            })
            .unwrap();
        let lines = rendered_lines(&terminal, area);
        let header = lines
            .iter()
            .position(|line| line.contains("Metadata"))
            .unwrap();
        let panel_width = width * super::details_width_percent(width) / 100;
        let left = (width - panel_width) / 2;
        let right = left + panel_width - 1;
        let buffer = terminal.backend().buffer();
        if width >= 100 {
            for y in header as u16 + 1..area.height - 1 {
                assert_eq!(buffer.cell((right, y)).unwrap().symbol(), "│");
                assert_eq!(buffer.cell((left, y)).unwrap().symbol(), "│");
            }
        } else {
            assert!(lines.iter().any(|line| line.starts_with("Template ")));
            let compose_row = lines
                .iter()
                .position(|line| line.starts_with("Compose file "))
                .unwrap();
            assert_ne!(
                buffer.cell((right, compose_row as u16)).unwrap().symbol(),
                "│"
            );
        }
    }
}

#[test]
fn details_hotkey_opens_the_selected_instance_in_a_bottom_dialog() {
    tuicore::init();
    let mut app = root(AppService::for_tests());
    app.set_rows_for_tests(rows::from_snapshot(&snapshot()));
    super::instances::set_highlighted(&app.instances, Some("instance:review".into()));

    app.event(
        &TuiEvent::Key(KeyEvent::from(Key::Char('v'))),
        &mut EventCtx::new(AnimationSettings::default()),
    );

    assert!(app.view.first().is_active());
}

#[test]
fn details_hotkey_opens_the_selected_routed_service() {
    tuicore::init();
    let mut app = root(AppService::for_tests());
    app.set_rows_for_tests(rows::from_snapshot(&snapshot()));
    super::instances::set_highlighted(&app.instances, Some("service:review:web".into()));

    app.event(
        &TuiEvent::Key(KeyEvent::from(Key::Char('v'))),
        &mut EventCtx::new(AnimationSettings::default()),
    );

    assert!(app.view.first().is_active());
}

#[test]
fn template_action_menu_keeps_typed_hotkeys_in_its_search() {
    tuicore::init();
    let mut app = root(AppService::for_tests());
    app.set_rows_for_tests(rows::from_snapshot(&snapshot()));
    let area = Rect::new(0, 0, 130, 40);
    let mut events = EventCtx::new(AnimationSettings::default());
    app.layout(area, &mut tuicore::LayoutCtx::new());

    app.event(&TuiEvent::Key(KeyEvent::from(Key::Char('.'))), &mut events);
    app.layout(area, &mut tuicore::LayoutCtx::new());
    let mut terminal = Terminal::new(TestBackend::new(area.width, area.height)).unwrap();
    terminal
        .draw(|frame| {
            let mut render = RenderCtx::new();
            app.render(frame, area, &mut render);
            render.flush(frame);
        })
        .unwrap();
    let lines = rendered_lines(&terminal, area);
    for (label, hotkey) in [
        ("View details", "v"),
        ("New instance", "n"),
        ("Stop all instances", "s"),
        ("Purge all instances", "p"),
        ("Delete template", "x"),
    ] {
        let line = lines.iter().find(|line| line.contains(label)).unwrap();
        assert!(line.trim_end().ends_with(hotkey));
    }

    app.event(&TuiEvent::Key(KeyEvent::from(Key::Char('v'))), &mut events);
    assert!(!app.view.first().is_active());
    assert!(app.menu_layer().is_active());
    assert!(app.menu_layer().layer().is_open());
}

#[test]
fn template_stop_and_purge_hotkeys_open_bulk_confirmations() {
    tuicore::init();
    let mut app = root(AppService::for_tests());
    app.set_rows_for_tests(rows::from_snapshot(&snapshot()));
    let mut events = EventCtx::new(AnimationSettings::default());

    app.event(&TuiEvent::Key(KeyEvent::from(Key::Char('s'))), &mut events);
    assert!(matches!(app.intent, Some(super::Intent::StopTemplate(_))));
    assert!(app.view.first().is_active());

    app.handle_message(Msg::Close, &mut events);
    app.event(&TuiEvent::Key(KeyEvent::from(Key::Char('p'))), &mut events);
    assert!(matches!(app.intent, Some(super::Intent::DeleteTemplate(_))));
    assert!(app.view.first().is_active());
}

#[test]
fn instance_action_menu_lists_instance_actions() {
    tuicore::init();
    let mut app = root(AppService::for_tests());
    app.set_rows_for_tests(rows::from_snapshot(&snapshot()));
    let area = Rect::new(0, 0, 130, 40);
    let mut events = EventCtx::new(AnimationSettings::default());
    app.layout(area, &mut tuicore::LayoutCtx::new());
    super::instances::set_highlighted(&app.instances, Some("instance:review".into()));

    app.event(&TuiEvent::Key(KeyEvent::from(Key::Char('.'))), &mut events);
    app.layout(area, &mut tuicore::LayoutCtx::new());
    let mut terminal = Terminal::new(TestBackend::new(area.width, area.height)).unwrap();
    terminal
        .draw(|frame| {
            let mut render = RenderCtx::new();
            app.render(frame, area, &mut render);
            render.flush(frame);
        })
        .unwrap();
    let lines = rendered_lines(&terminal, area);
    for (label, hotkey) in [
        ("View details", "v"),
        ("New instance", "n"),
        ("Start instance", "s"),
        ("Stop instance", "s"),
        ("Delete instance", "x"),
    ] {
        let line = lines.iter().find(|line| line.contains(label)).unwrap();
        assert!(line.trim_end().ends_with(hotkey));
    }
}

fn expand_first_instance(tree: &mut crate::app::Instances) {
    tree.focus(None, true, &mut tuicore::FocusCtx::default());
    let mut ctx = EventCtx::new(AnimationSettings::default());
    for key in [Key::Down, Key::Right, Key::Home] {
        tree.event(&TuiEvent::Key(KeyEvent::from(key)), &mut ctx);
    }
    tree.focus(None, false, &mut tuicore::FocusCtx::default());
}

#[test]
fn stopped_instance_menu_mutes_stop_without_changing_its_hotkey() {
    tuicore::init();
    let mut environment = snapshot();
    environment.instances[0].services[0].status = "down (exit 0)".into();
    let mut app = root(AppService::for_tests());
    app.set_rows_for_tests(rows::from_snapshot(&environment));
    let area = Rect::new(0, 0, 130, 40);
    let mut events = EventCtx::new(AnimationSettings::default());
    app.layout(area, &mut tuicore::LayoutCtx::new());
    super::instances::set_highlighted(&app.instances, Some("instance:review".into()));

    app.event(&TuiEvent::Key(KeyEvent::from(Key::Char('.'))), &mut events);
    app.layout(area, &mut tuicore::LayoutCtx::new());
    let mut terminal = Terminal::new(TestBackend::new(area.width, area.height)).unwrap();
    terminal
        .draw(|frame| {
            let mut render = RenderCtx::new();
            app.render(frame, area, &mut render);
            render.flush(frame);
        })
        .unwrap();
    let lines = rendered_lines(&terminal, area);
    assert!(
        lines
            .iter()
            .any(|line| line.contains("Start instance") && line.trim_end().ends_with('s'))
    );
    assert!(
        lines
            .iter()
            .any(|line| line.contains("Stop instance") && line.trim_end().ends_with('s'))
    );

    app.event(&TuiEvent::Key(KeyEvent::from(Key::Char('s'))), &mut events);
    assert!(!app.view.first().is_active());
    assert!(app.menu_layer().is_active());
}

#[test]
fn start_stop_hotkey_follows_the_instance_state() {
    tuicore::init();
    let mut app = root(AppService::for_tests());
    app.set_rows_for_tests(rows::from_snapshot(&snapshot()));
    let mut events = EventCtx::new(AnimationSettings::default());
    super::instances::set_highlighted(&app.instances, Some("instance:review".into()));

    app.event(&TuiEvent::Key(KeyEvent::from(Key::Char('s'))), &mut events);
    assert!(matches!(app.intent, Some(super::Intent::Stop(_))));
    app.handle_message(Msg::Close, &mut events);

    let mut environment = snapshot();
    environment.instances[0].services[0].status = "down (exit 0)".into();
    app.set_rows_for_tests(rows::from_snapshot(&environment));
    super::instances::set_highlighted(&app.instances, Some("instance:review".into()));
    app.event(&TuiEvent::Key(KeyEvent::from(Key::Char('s'))), &mut events);

    assert!(matches!(app.intent, Some(super::Intent::Resume { .. })));
}

#[test]
fn active_data_view_search_keeps_action_hotkeys_as_search_text() {
    tuicore::init();
    let mut app = root(AppService::for_tests());
    app.set_rows_for_tests(rows::from_snapshot(&snapshot()));
    let mut events = EventCtx::new(AnimationSettings::default());
    super::instances::set_searching(&app.instances, true);
    app.event(&TuiEvent::Key(KeyEvent::from(Key::Char('n'))), &mut events);

    assert!(super::instances::is_searching(&app.instances));
    assert!(app.intent.is_none());
    assert!(!app.view.first().is_active());
}
