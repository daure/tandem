use ratatui::{Terminal, backend::TestBackend, layout::Rect};
use tuicore::{
    AnimationSettings, EventCtx, EventRoute, HotkeyEvent, Key, KeyEvent, KeyModifiers,
    LayoutEngine, RenderCtx, TuiEvent, TuiNode,
};

use super::{App, Msg, rows};
use crate::{
    service::AppService,
    store::environments::{EnvironmentSnapshot, Instance, InstanceService, Manifest, Template},
};

mod attached_sessions;
mod bulk;
mod guidance;
mod input_routing;
mod labels;
mod opencode;
mod operations;
mod properties;
mod refresh;
mod resources;
mod restart;
mod service_state;
mod template_actions;
mod toolbar;
mod workspaces;

fn root(service: AppService) -> App {
    let mut app = super::root(service);
    app.handle_message(
        Msg::SetAttachedSessionsOnly(false),
        &mut EventCtx::new(AnimationSettings::default()),
    );
    super::instances::replace_rows(
        &app.instances,
        super::visible_rows(&app.snapshot, &[], false),
    );
    app
}

fn snapshot() -> EnvironmentSnapshot {
    EnvironmentSnapshot {
        templates: vec![Template {
            name: "website".into(),
            directory: "/tmp/templates/website".into(),
            compose_file: "/tmp/templates/website/compose.yaml".into(),
            manifest_file: "/tmp/templates/website/tandem.json".into(),
            guidance_file: "/tmp/templates/website/tandem-agents.md".into(),
            guidance_source: None,
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
                ..Default::default()
            }],
            runtime: crate::store::environments::InstanceRuntime {
                topology_known: true,
                ..Default::default()
            },
            ..Default::default()
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
        ..Default::default()
    });
    let rows = rows::from_snapshot(&snapshot);
    assert_eq!(rows.len(), 5);
    assert_eq!(rows[0].label, "website\n 1/1");
    assert_eq!(rows[0].icon, "󰠲");
    assert_eq!(rows[1].parent, Some(rows[0].id.clone()));
    assert_eq!(rows[1].label, "review · Running");
    assert_eq!(rows[1].icon, "");
    assert_eq!(rows[1].status_detail, None);
    assert_eq!(rows[2].parent, Some(rows[1].id.clone()));
    assert_eq!(rows[2].label, "Services");
    assert_eq!(rows[2].icon, "󰒋");
    assert_eq!(rows[2].tone, rows::Tone::Success);
    assert_eq!(rows[2].status_detail.as_deref(), Some("2/2 running"));
    assert_eq!(rows[3].parent, Some(rows[2].id.clone()));
    assert_eq!(
        rows[3].label,
        "web · Running\n http://localhost:9876/review/web/ · 󰈀 8080"
    );
    assert_eq!(rows[3].icon, "");
    assert_eq!(rows[4].parent, Some(rows[2].id.clone()));
    assert_eq!(rows[4].label, "db · Healthy\npostgres:17.5-alpine");
    assert_eq!(rows[4].icon, "");
    assert_eq!(
        rows[3].gateway_url.as_deref(),
        Some("http://localhost:9876/review/web/")
    );
    snapshot.templates.clear();
    let rows = rows::from_snapshot(&snapshot);
    assert_eq!(rows.len(), 5);
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
    assert!(background("services:guide"));
    assert!(!background("service:guide:web"));
    assert!(background("template:/tmp/templates/website"));
    assert!(!background("instance:review"));
    assert!(background("services:review"));
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
fn setup_jobs_are_nested_under_setup_and_services_are_nested_under_services() {
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
        assert_eq!(rows.len(), 6);
        let setup = rows
            .iter()
            .find(|row| row.id == "service:review:repo-sync")
            .unwrap();
        assert_eq!(setup.resource_text().to_string(), "");
        assert_eq!(setup.parent.as_deref(), Some("setup:review"));
        assert_eq!(
            rows.iter()
                .find(|row| row.id == "service:review:web")
                .unwrap()
                .parent
                .as_deref(),
            Some("services:review")
        );
        assert!(
            rows[1]
                .details
                .iter()
                .any(|row| row.name == "repo-sync / Docker status" && row.value == status)
        );
    }

    snapshot.templates.clear();
    assert_eq!(rows::from_snapshot(&snapshot).len(), 6);

    snapshot.instances[0].services[1].one_shot = false;
    let rows = rows::from_snapshot(&snapshot);
    assert_eq!(
        rows.iter()
            .find(|row| row.id == "services:review")
            .unwrap()
            .label,
        "Services"
    );
    assert_eq!(
        rows.iter()
            .find(|row| row.id == "service:review:repo-sync")
            .unwrap()
            .parent
            .as_deref(),
        Some("services:review")
    );
}

#[test]
fn unassigned_keys_leave_selected_instance_actions_idle() {
    tuicore::init();
    let mut app = root(AppService::for_tests());
    app.set_rows_for_tests(rows::from_snapshot(&snapshot()));
    super::instances::set_highlighted(&app.instances, Some("instance:review".into()));

    for key in [
        KeyEvent::from(Key::Char('v')),
        KeyEvent::from(Key::Char(';')),
    ] {
        app.event(
            &TuiEvent::Key(key),
            &mut EventCtx::new(AnimationSettings::default()),
        );
        assert!(!app.view.is_active());
        assert!(app.service.opened_system_targets().is_empty());
        assert!(app.service.operations().is_empty());
    }
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
    assert!(!lines.iter().any(|line| line.contains("Copy service name")));
    assert!(!lines.iter().any(|line| line.contains("Copy gateway URL")));
    for (label, hotkey) in [("Yank", "y"), ("Open in browser", "⌃Enter")] {
        let line = lines.iter().find(|line| line.contains(label)).unwrap();
        assert!(
            line.trim_end_matches([' ', '┃']).ends_with(hotkey),
            "{line}"
        );
    }
    super::instances::set_highlighted(&app.instances, Some("service:review:web".into()));
    app.event(&TuiEvent::Key(KeyEvent::from(Key::Enter)), &mut events);
    assert!(!app.menu_layer().is_active());
    assert!(app.yank_layer().is_active());
    app.event(&TuiEvent::Key(KeyEvent::from(Key::Esc)), &mut events);
    assert!(!app.yank_layer().is_active());

    super::instances::set_highlighted(&app.instances, Some("service:review:web".into()));
    app.menu_layer_mut()
        .set_active_with_context(false, &mut events);
    app.event(
        &TuiEvent::Key(KeyEvent {
            code: Key::Char(';'),
            modifiers: KeyModifiers::CONTROL,
        }),
        &mut events,
    );
    assert!(app.service.opened_system_targets().is_empty());
    app.event(
        &TuiEvent::Key(KeyEvent {
            code: Key::Enter,
            modifiers: KeyModifiers::CONTROL,
        }),
        &mut events,
    );

    assert_eq!(
        app.service.opened_system_targets(),
        ["http://localhost:9876/review/web/"]
    );
}

#[test]
fn action_menu_dims_the_status_bar_background() {
    tuicore::init();
    let mut app = root(AppService::for_tests());
    app.set_rows_for_tests(rows::from_snapshot(&snapshot()));
    let area = Rect::new(0, 0, 130, 40);
    let status_cell = (area.right() - 1, area.bottom() - 1);
    let mut terminal = Terminal::new(TestBackend::new(area.width, area.height)).unwrap();
    let mut layout = tuicore::LayoutCtx::new();
    layout.with_overlay_bounds(area, |ctx| app.layout(area, ctx));
    terminal
        .draw(|frame| {
            let mut render = RenderCtx::new();
            app.render(frame, area, &mut render);
            render.flush(frame);
        })
        .unwrap();
    let status_background = terminal.backend().buffer()[status_cell].bg;

    app.event(
        &TuiEvent::Key(KeyEvent::from(Key::Char('.'))),
        &mut EventCtx::new(AnimationSettings::default()),
    );
    app.view.tick(
        std::time::Duration::from_secs(1),
        AnimationSettings {
            enabled: false,
            ..Default::default()
        },
    );
    let mut layout = tuicore::LayoutCtx::new();
    layout.with_overlay_bounds(area, |ctx| app.layout(area, ctx));
    terminal
        .draw(|frame| {
            let mut render = RenderCtx::new();
            app.render(frame, area, &mut render);
            render.flush(frame);
        })
        .unwrap();
    let dimmed_status = &terminal.backend().buffer()[status_cell];

    assert_ne!(dimmed_status.bg, status_background);
    assert!(
        dimmed_status
            .modifier
            .contains(ratatui::style::Modifier::DIM)
    );
}

#[test]
fn instance_control_enter_opens_its_only_service_route_without_a_menu() {
    tuicore::init();
    let mut app = root(AppService::for_tests());
    app.update_snapshot(snapshot());
    super::instances::set_highlighted(&app.instances, Some("instance:review".into()));

    app.event(
        &TuiEvent::Key(KeyEvent {
            code: Key::Enter,
            modifiers: KeyModifiers::CONTROL,
        }),
        &mut EventCtx::new(AnimationSettings::default()),
    );

    assert!(!app.route_layer().is_active());
    assert_eq!(
        app.service.opened_system_targets(),
        ["http://localhost:9876/review/web/"]
    );
}

#[test]
fn instance_control_enter_chooses_a_service_route_to_open() {
    tuicore::init();
    let mut snapshot = snapshot();
    snapshot.instances[0].services.push(InstanceService {
        name: "api".into(),
        container_id: "api-container-id".into(),
        status: "up".into(),
        url: Some("http://localhost:9876/review/api/".into()),
        ..Default::default()
    });
    let mut app = root(AppService::for_tests());
    app.update_snapshot(snapshot);
    super::instances::set_highlighted(&app.instances, Some("instance:review".into()));
    let mut events = EventCtx::new(AnimationSettings::default());

    app.event(
        &TuiEvent::Key(KeyEvent {
            code: Key::Enter,
            modifiers: KeyModifiers::CONTROL,
        }),
        &mut events,
    );

    assert!(app.route_layer().is_active());
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
    let rendered = rendered_lines(&terminal, area).join("\n");
    assert!(
        rendered.contains("api - http://localhost:9876/review/api/"),
        "{rendered}"
    );
    assert!(
        rendered.contains("web - http://localhost:9876/review/web/"),
        "{rendered}"
    );

    app.event(
        &TuiEvent::Key(KeyEvent {
            code: Key::Char('j'),
            modifiers: KeyModifiers::CONTROL,
        }),
        &mut events,
    );
    app.event(&TuiEvent::Key(KeyEvent::from(Key::Enter)), &mut events);

    assert!(!app.route_layer().is_active());
    assert_eq!(
        app.service.opened_system_targets(),
        ["http://localhost:9876/review/api/"]
    );
    super::instances::set_highlighted(&app.instances, Some("instance:review".into()));

    let mut reopen = EventCtx::new(AnimationSettings::default());
    assert!(app.open_selected_route(&mut reopen));
    assert!(app.route_layer().is_active());
    app.event(
        &TuiEvent::Key(KeyEvent::from(Key::Enter)),
        &mut EventCtx::new(AnimationSettings::default()),
    );

    assert_eq!(
        app.service.opened_system_targets(),
        [
            "http://localhost:9876/review/api/",
            "http://localhost:9876/review/web/"
        ]
    );
}

#[test]
fn instance_yank_menu_copies_the_name_description_or_full_workspace_path() {
    tuicore::init();
    for (hotkey, value) in [
        ('i', "review"),
        ('d', "Review environment"),
        ('w', "/tmp/workspaces/review"),
    ] {
        let mut environment = snapshot();
        environment.instances[0].description = "Review environment".into();
        let mut app = root(AppService::for_tests());
        app.set_rows_for_tests(rows::from_snapshot(&environment));
        super::instances::set_highlighted(&app.instances, Some("instance:review".into()));
        let mut events = EventCtx::new(AnimationSettings::default());

        app.event(&TuiEvent::Yank, &mut events);
        assert!(app.yank_layer().is_active());
        app.event(
            &TuiEvent::Key(KeyEvent::from(Key::Char(hotkey))),
            &mut events,
        );

        assert_eq!(events.clipboard_request(), Some(value));
        assert!(!app.yank_layer().is_active());
    }
}

#[test]
fn routed_service_yank_menu_only_copies_its_url() {
    tuicore::init();
    let mut app = root(AppService::for_tests());
    app.set_rows_for_tests(rows::from_snapshot(&snapshot()));
    super::instances::set_highlighted(&app.instances, Some("service:review:web".into()));
    let mut events = EventCtx::new(AnimationSettings::default());

    app.event(&TuiEvent::Yank, &mut events);
    assert!(app.yank_layer().is_active());
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
    assert!(lines.iter().any(|line| line.trim() == "URL"));
    assert!(!lines.iter().any(|line| line.trim() == "Workspace"));

    app.event(&TuiEvent::Key(KeyEvent::from(Key::Char('u'))), &mut events);

    assert_eq!(
        events.clipboard_request(),
        Some("http://localhost:9876/review/web/")
    );
}

#[test]
fn copy_hotkeys_are_registered_on_the_instances_tab() {
    tuicore::init();
    let mut app = root(AppService::for_tests());
    app.set_rows_for_tests(rows::from_snapshot(&snapshot()));
    let area = Rect::new(0, 0, 130, 40);
    let mut layout = tuicore::LayoutEngine::new();
    layout.layout(&mut app, area);
    let path = layout
        .focus_targets()
        .iter()
        .find(|target| {
            target
                .hotkey_sequences
                .iter()
                .any(|sequence| sequence == "yy")
        })
        .unwrap()
        .path
        .clone();

    for sequence in ["yy", "yi", "yd", "yw", "yu"] {
        let mut events = EventCtx::new(AnimationSettings::default());
        app.dispatch_event(
            &EventRoute::new(path.clone()),
            &TuiEvent::Hotkey(HotkeyEvent::Commit(sequence.into())),
            &mut events,
        );

        assert!(matches!(
            (sequence, events.messages()),
            ("yy" | "yi", [Msg::CopyName])
                | ("yd", [Msg::CopyDescription])
                | ("yw", [Msg::CopyWorkspace])
                | ("yu", [Msg::CopyGatewayUrl])
        ));
    }
}

#[test]
fn control_semicolon_opens_the_selected_instance_workspace() {
    tuicore::init();
    let mut app = root(AppService::for_tests());
    let directory = tempfile::tempdir().unwrap();
    let workspace = directory.path().join("review");
    std::fs::create_dir(&workspace).unwrap();
    std::fs::write(workspace.join("AGENTS.md"), "Workspace guidance\n").unwrap();
    let mut snapshot = snapshot();
    snapshot.instances[0].workspace = workspace.display().to_string();
    app.set_rows_for_tests(rows::from_snapshot(&snapshot));
    super::instances::set_highlighted(&app.instances, Some("instance:review".into()));

    app.event(
        &TuiEvent::Key(KeyEvent {
            code: Key::Char(';'),
            modifiers: KeyModifiers::CONTROL,
        }),
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
        [workspace.display().to_string()]
    );
}

#[test]
fn startup_waits_for_complete_inventory_before_showing_and_selecting_rows() {
    tuicore::init();
    let mut app = root(AppService::for_tests());
    let mut inventory = snapshot();
    let mut empty_template = inventory.templates[0].clone();
    empty_template.name = "aardvark".into();
    empty_template.directory = "/tmp/templates/aardvark".into();
    inventory.templates.insert(0, empty_template);
    inventory.loading = true;
    inventory.instances.clear();

    app.update_snapshot(inventory.clone());
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
    let loading_text = rendered_lines(&terminal, area).join("");
    assert!(!loading_text.contains("aardvark"));
    assert!(!loading_text.contains("website"));

    inventory.loading = false;
    inventory.instances = snapshot().instances;
    app.update_snapshot(inventory);
    app.layout(area, &mut tuicore::LayoutCtx::new());
    assert_eq!(app.selected().unwrap().template, "website");
    terminal
        .draw(|frame| {
            let mut render = RenderCtx::new();
            app.render(frame, area, &mut render);
            render.flush(frame);
        })
        .unwrap();
    let loaded_text = rendered_lines(&terminal, area).join("");
    assert!(loaded_text.contains("website"));
    assert!(loaded_text.contains("aardvark"));
}

#[test]
fn data_view_starts_expanded_through_instances() {
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
    assert!(text.contains("Service"));
    assert!(!text.contains("http://localhost:9876/review/web/"));
    assert!(!text.contains("Templates / instances"));
    assert!(!text.contains("Status"));
}

#[test]
fn global_h_clears_search_focuses_the_first_item_and_expands_all_rows() {
    tuicore::init();
    let mut app = root(AppService::for_tests());
    app.set_rows_for_tests(rows::from_snapshot(&snapshot()));
    let area = Rect::new(0, 0, 130, 40);

    let mut tree = super::Instances::new(app.instances.clone());
    let mut layout = LayoutEngine::new();
    layout.layout(&mut tree, area);
    assert!(layout.focus_targets().iter().any(|target| {
        target
            .hotkey_sequences
            .iter()
            .any(|sequence| sequence == "shift+h")
    }));
    expand_first_instance(&mut tree);
    let mut ctx = EventCtx::new(AnimationSettings::default());
    tree.focus(None, true, &mut tuicore::FocusCtx::default());
    for key in [Key::Char('/'), Key::Char('z')] {
        tree.event(&TuiEvent::Key(KeyEvent::from(key)), &mut ctx);
    }
    assert_eq!(tree.search_query(), "z");
    tree.event(
        &TuiEvent::Hotkey(HotkeyEvent::Commit("shift+h".into())),
        &mut ctx,
    );

    assert_eq!(tree.search_query(), "");
    assert_eq!(
        super::instances::selected(&app.instances).unwrap().template,
        "website"
    );
    assert!(ctx.focus_request().is_some());
    let mut terminal = Terminal::new(TestBackend::new(area.width, area.height)).unwrap();
    terminal
        .draw(|frame| {
            let mut render = RenderCtx::new();
            tree.render(frame, area, &mut render);
            render.flush(frame);
        })
        .unwrap();
    assert!(rendered_lines(&terminal, area).join("").contains("review"));
    assert!(rendered_lines(&terminal, area).join("").contains("Service"));
    assert!(
        rendered_lines(&terminal, area)
            .join("")
            .contains("http://localhost:9876/review/web/")
    );
}

#[test]
fn z_toggles_templates_and_instances_without_expanding_their_groups() {
    tuicore::init();
    let mut snapshot = snapshot();
    let mut setup = snapshot.instances[0].services[0].clone();
    setup.name = "migrate".into();
    setup.one_shot = true;
    setup.status = "exited 0".into();
    setup.url = None;
    setup.port = None;
    snapshot.instances[0].services.push(setup);
    let mut tree = super::Instances::new(super::instances::state(rows::from_snapshot(&snapshot)));
    let area = Rect::new(0, 0, 130, 40);
    tree.layout(area, &mut tuicore::LayoutCtx::new());
    tree.focus(None, true, &mut tuicore::FocusCtx::default());
    let mut ctx = EventCtx::new(AnimationSettings::default());

    tree.event(&TuiEvent::Key(KeyEvent::from(Key::Char('z'))), &mut ctx);
    tree.layout(area, &mut tuicore::LayoutCtx::new());
    let mut terminal = Terminal::new(TestBackend::new(area.width, area.height)).unwrap();
    terminal
        .draw(|frame| {
            let mut render = RenderCtx::new();
            tree.render(frame, area, &mut render);
            render.flush(frame);
        })
        .unwrap();
    assert!(!rendered_lines(&terminal, area).join("").contains("review"));

    tree.event(&TuiEvent::Key(KeyEvent::from(Key::Char('z'))), &mut ctx);
    tree.layout(area, &mut tuicore::LayoutCtx::new());
    terminal
        .draw(|frame| {
            let mut render = RenderCtx::new();
            tree.render(frame, area, &mut render);
            render.flush(frame);
        })
        .unwrap();
    let text = rendered_lines(&terminal, area).join("");
    assert!(text.contains("review"));
    assert!(text.contains("Setup · 1 completed"));
    assert!(text.contains("Service"));
    assert!(!text.contains("migrate · Completed"));
    assert!(!text.contains("web · Running"));
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
            code: Key::Enter,
            modifiers: KeyModifiers::NONE,
        }),
        &mut events,
    );
    assert!(app.view.is_active());
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
    let route =
        tuicore::EventRoute::new(tuicore::TreePath::from_keys([tuicore::ChildKey::second()]));
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
    assert!(!app.view.is_active());
}

#[test]
fn detail_tabs_keep_the_header_and_close_control_above_the_content() {
    tuicore::init();
    let rows = rows::from_snapshot(&snapshot());
    for width in [56, 130] {
        for (row, title, content, value) in [
            (&rows[0], "Metadata", "Template", "website"),
            (&rows[1], "Details", "Instance", "review"),
            (
                rows.iter()
                    .find(|row| row.id == "service:review:web")
                    .unwrap(),
                "Details",
                "Service",
                "web",
            ),
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
    assert!(app.view.is_active());
    assert!(app.service.operations().is_empty());
}

#[test]
fn new_instance_description_uses_a_text_input() {
    tuicore::init();
    let mut dialog = super::dialogs::instance_entry(
        "New instance",
        "review",
        "Review environment",
        "Instance name",
        None,
    );
    let mut layout = LayoutEngine::new();
    layout.layout(&mut dialog, Rect::new(0, 0, 80, 20));
    let targets = layout.focus_targets();

    assert_eq!(
        targets
            .iter()
            .filter(|target| target.id.as_str() == "input")
            .count(),
        2
    );
    assert!(
        targets
            .iter()
            .all(|target| target.id.as_str() != "textarea")
    );
}

#[test]
fn entered_names_are_preserved() {
    let mut app = root(AppService::for_tests());
    app.handle_message(
        Msg::NameChanged("Feature Branch".into()),
        &mut EventCtx::new(AnimationSettings::default()),
    );
    app.handle_message(
        Msg::DescriptionChanged("Review environment".into()),
        &mut EventCtx::new(AnimationSettings::default()),
    );

    assert_eq!(app.name, "Feature Branch");
    assert_eq!(app.description, "Review environment");
}

#[test]
fn description_hotkey_opens_an_unpadded_text_editor_in_insert_mode() {
    tuicore::init();
    let mut environment = snapshot();
    environment.instances[0].description = "Review environment".into();
    let mut app = root(AppService::for_tests());
    app.set_rows_for_tests(rows::from_snapshot(&environment));
    super::instances::set_highlighted(&app.instances, Some("instance:review".into()));
    let area = Rect::new(0, 0, 130, 40);
    let mut events = EventCtx::new(AnimationSettings::default());

    app.event(&TuiEvent::Key(KeyEvent::from(Key::Char('d'))), &mut events);

    assert!(app.view.is_active());
    let mut layout = LayoutEngine::new();
    layout.layout(&mut app, area);
    let input = layout
        .focus_targets()
        .iter()
        .find(|target| target.id.as_str() == "input")
        .unwrap()
        .clone();
    app.dispatch_focus(&input, true, &mut tuicore::FocusCtx::default());
    let route = EventRoute::new(input.path);
    let mut terminal = Terminal::new(TestBackend::new(area.width, area.height)).unwrap();
    terminal
        .draw(|frame| {
            let mut render = RenderCtx::new();
            app.render(frame, area, &mut render);
            render.flush(frame);
        })
        .unwrap();
    let rendered = rendered_lines(&terminal, area).join("\n");
    assert!(rendered.contains("Update description"), "{rendered}");
    assert!(rendered.contains("Save"), "{rendered}");
    assert!(rendered.contains("Cancel"), "{rendered}");

    let mut edit = EventCtx::new(AnimationSettings::default());
    app.dispatch_event(
        &route,
        &TuiEvent::Key(KeyEvent::from(Key::Char('!'))),
        &mut edit,
    );
    let value = match edit.messages() {
        [Msg::DescriptionChanged(value)] => value.clone(),
        messages => panic!("unexpected edit messages: {messages:?}"),
    };
    assert_eq!(value, "Review environment!");
    app.handle_message(
        Msg::DescriptionChanged(value),
        &mut EventCtx::new(AnimationSettings::default()),
    );
    let mut submit = EventCtx::new(AnimationSettings::default());
    app.dispatch_event(
        &route,
        &TuiEvent::Key(KeyEvent {
            code: Key::Enter,
            modifiers: KeyModifiers::CONTROL,
        }),
        &mut submit,
    );
    assert!(!app.view.is_active());
    assert!(app.description_save.is_some());
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
fn details_hotkey_uses_full_width_on_mobile_and_seventy_five_percent_on_desktop() {
    assert_eq!(super::details_width_percent(99), 100);
    assert_eq!(super::details_width_percent(100), 75);
    tuicore::init();
    let mut app = root(AppService::for_tests());
    app.set_rows_for_tests(rows::from_snapshot(&snapshot()));
    app.action(0, &mut EventCtx::new(AnimationSettings::default()));
    app.view.tick(
        std::time::Duration::from_secs(1),
        AnimationSettings {
            enabled: false,
            ..Default::default()
        },
    );
    for width in [130, 56, 130] {
        let area = Rect::new(0, 0, width, 40);
        let mut layout = tuicore::LayoutCtx::new();
        layout.with_overlay_bounds(area, |ctx| app.layout(area, ctx));
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
            for y in header as u16 + 1..area.height {
                assert_eq!(buffer.cell((right, y)).unwrap().symbol(), "│");
                assert_eq!(buffer.cell((left, y)).unwrap().symbol(), "│");
            }
            assert!(
                buffer
                    .cell((0, area.bottom() - 1))
                    .unwrap()
                    .modifier
                    .contains(ratatui::style::Modifier::DIM)
            );
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
fn bottom_dialog_dims_the_status_bar_background() {
    tuicore::init();
    let mut app = root(AppService::for_tests());
    app.set_rows_for_tests(rows::from_snapshot(&snapshot()));
    let area = Rect::new(0, 0, 130, 40);
    let status_cell = (area.right() - 1, area.bottom() - 1);
    let mut terminal = Terminal::new(TestBackend::new(area.width, area.height)).unwrap();
    let mut layout = tuicore::LayoutCtx::new();
    layout.with_overlay_bounds(area, |ctx| app.layout(area, ctx));
    terminal
        .draw(|frame| {
            let mut render = RenderCtx::new();
            app.render(frame, area, &mut render);
            render.flush(frame);
        })
        .unwrap();
    let status_background = terminal.backend().buffer()[status_cell].bg;

    app.action(0, &mut EventCtx::new(AnimationSettings::default()));
    app.view.tick(
        std::time::Duration::from_secs(1),
        AnimationSettings {
            enabled: false,
            ..Default::default()
        },
    );
    let mut layout = tuicore::LayoutCtx::new();
    layout.with_overlay_bounds(area, |ctx| app.layout(area, ctx));
    terminal
        .draw(|frame| {
            let mut render = RenderCtx::new();
            app.render(frame, area, &mut render);
            render.flush(frame);
        })
        .unwrap();
    let dimmed_status = &terminal.backend().buffer()[status_cell];

    assert_ne!(dimmed_status.bg, status_background);
    assert!(
        dimmed_status
            .modifier
            .contains(ratatui::style::Modifier::DIM)
    );
}

#[test]
fn details_hotkey_opens_the_selected_instance_in_a_bottom_dialog() {
    tuicore::init();
    let mut app = root(AppService::for_tests());
    app.set_rows_for_tests(rows::from_snapshot(&snapshot()));
    super::instances::set_highlighted(&app.instances, Some("instance:review".into()));

    app.event(
        &TuiEvent::Key(KeyEvent::from(Key::Enter)),
        &mut EventCtx::new(AnimationSettings::default()),
    );

    assert!(app.view.is_active());
    assert!(app.service.opened_system_targets().is_empty());
}

#[test]
fn details_hotkey_opens_the_selected_routed_service() {
    tuicore::init();
    let mut app = root(AppService::for_tests());
    app.set_rows_for_tests(rows::from_snapshot(&snapshot()));
    super::instances::set_highlighted(&app.instances, Some("service:review:web".into()));

    app.event(
        &TuiEvent::Key(KeyEvent::from(Key::Enter)),
        &mut EventCtx::new(AnimationSettings::default()),
    );

    assert!(app.view.is_active());
    assert!(app.service.opened_system_targets().is_empty());
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
        ("Copy template name", "yy"),
        ("View details", "Enter"),
        ("New instance", "n"),
        ("Stop all instances", "s"),
        ("Purge all instances", "p"),
        ("Delete template", "x"),
    ] {
        let line = lines.iter().find(|line| line.contains(label)).unwrap();
        assert!(
            line.trim_end_matches([' ', '┃']).ends_with(hotkey),
            "{line}"
        );
    }

    app.event(&TuiEvent::Key(KeyEvent::from(Key::Char('v'))), &mut events);
    assert!(!app.view.is_active());
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
    assert!(app.view.is_active());

    app.handle_message(Msg::Close, &mut events);
    app.event(&TuiEvent::Key(KeyEvent::from(Key::Char('p'))), &mut events);
    assert!(matches!(app.intent, Some(super::Intent::DeleteTemplate(_))));
    assert!(app.view.is_active());
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
        ("Yank", "y"),
        ("Update description", "d"),
        ("View details", "Enter"),
        ("Run open command", "⌃;"),
        ("New instance", "n"),
        ("Start instance", "s"),
        ("Stop instance", "s"),
        ("Purge instance", "p"),
    ] {
        let line = lines.iter().find(|line| line.contains(label)).unwrap();
        assert!(
            line.trim_end_matches([' ', '┃']).ends_with(hotkey),
            "{line}"
        );
    }
    assert!(!lines.iter().any(|line| line.contains("Copy instance name")));

    app.event(&TuiEvent::Key(KeyEvent::from(Key::Enter)), &mut events);
    assert!(!app.menu_layer().is_active());
    assert!(app.yank_layer().is_active());
}

fn expand_first_instance(tree: &mut crate::app::Instances) {
    tree.expand_for_tests("instance:review");
    tree.expand_for_tests("services:review");
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
    assert!(!app.view.is_active());
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
    assert!(!app.view.is_active());
}
