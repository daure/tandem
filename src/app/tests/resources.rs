use super::*;
use crate::app::{
    instances::{self, Instances},
    rows::Tone,
};
use crate::store::environments::ResourceUsage;
use std::time::Duration;

fn usage(cpu: u64, memory_mib: u64) -> ResourceUsage {
    ResourceUsage {
        cpu_basis_points: Some(cpu),
        memory_bytes: memory_mib * 1048576,
        sampled_at_unix_seconds: 42,
    }
}

#[test]
fn resource_values_round_to_whole_units_and_preserve_tiny_nonzero_samples() {
    tuicore::init();
    for (cpu, memory, cpu_text, memory_text) in [
        (0, 0, "0%", "0 MiB"),
        (1, 1, "<1%", "<1 MiB"),
        (99, 1048575, "<1%", "<1 MiB"),
        (100, 1048576, "1%", "1 MiB"),
        (149, 1572863, "1%", "1 MiB"),
        (150, 1572864, "2%", "2 MiB"),
        (250, 2621440, "3%", "3 MiB"),
        (4380, 30723277, "44%", "29 MiB"),
        (12550, 1048576, "126%", "1 MiB"),
        (
            u64::MAX,
            u64::MAX,
            "184467440737095516%",
            "17592186044416 MiB",
        ),
    ] {
        let mut snapshot = snapshot();
        snapshot.instances[0].services[0].usage = Some(ResourceUsage {
            cpu_basis_points: Some(cpu),
            memory_bytes: memory,
            sampled_at_unix_seconds: 42,
        });
        let tree = rows::from_snapshot(&snapshot);
        let row = &tree[2];
        assert_eq!(
            row.resource_text().to_string(),
            format!("󰑹 {memory_text}\n {cpu_text}")
        );
        assert_eq!(
            row.resource_text().lines[1].spans[0].style.fg,
            Some(Tone::Normal.color())
        );
        for (name, expected) in [("CPU", cpu_text), ("Memory", memory_text)] {
            assert_eq!(
                row.details
                    .iter()
                    .find(|row| row.name == name)
                    .unwrap()
                    .value,
                expected
            );
        }
    }
    let mut snapshot = snapshot();
    snapshot.instances.clear();
    let tree = rows::from_snapshot(&snapshot);
    let text = tree[0].resource_text();
    assert_eq!(text.to_string(), "󰑹 —\n —");
    assert_eq!(text.lines[1].spans[0].style.fg, Some(Tone::Muted.color()));
}

#[test]
fn memory_color_tracks_explicit_limits_in_tree_and_details() {
    tuicore::init();
    for (used, limit, expected) in [
        (69, Some(100), Tone::Normal),
        (70, Some(100), Tone::Warning),
        (89, Some(100), Tone::Warning),
        (90, Some(100), Tone::Error),
        (120, Some(100), Tone::Error),
        (120, None, Tone::Normal),
    ] {
        let mut snapshot = snapshot();
        let service = &mut snapshot.instances[0].services[0];
        service.usage = Some(usage(240, used));
        service.memory_limit_bytes = limit.map(|mib| mib * 1048576);
        let tree = rows::from_snapshot(&snapshot);
        for row in &tree {
            let text = row.resource_text();
            assert_eq!(text.lines[0].spans[0].style.fg, Some(expected.color()));
            assert_eq!(
                row.details
                    .iter()
                    .find(|property| property.name == "Memory")
                    .unwrap()
                    .tone,
                expected
            );
        }
        let state = instances::state(tree);
        let mut view = Instances::new(state);
        let terminal = render(&mut view, 130);
        let lines = rendered_lines(&terminal, Rect::new(0, 0, 130, 18));
        let y = lines.iter().position(|line| line.contains("web")).unwrap();
        let prefix = lines[y].split("MiB").next().unwrap();
        let cell = terminal
            .backend()
            .buffer()
            .cell((prefix.chars().count() as u16, y as u16))
            .unwrap();
        assert_eq!(cell.fg, expected.color());
    }

    let mut snapshot = snapshot();
    snapshot.instances[0].services[0].usage = Some(usage(100, 90));
    snapshot.instances[0].services[0].memory_limit_bytes = Some(100 * 1048576);
    let mut uncapped = snapshot.instances[0].services[0].clone();
    uncapped.name = "worker".into();
    uncapped.container_id = "worker-id".into();
    uncapped.memory_limit_bytes = None;
    snapshot.instances[0].services.push(uncapped);
    let tree = rows::from_snapshot(&snapshot);
    assert_eq!(tree[0].memory_limit_bytes, None);
    assert_eq!(tree[1].memory_limit_bytes, None);
    snapshot.instances[0].services[1].usage = None;
    assert_eq!(
        rows::from_snapshot(&snapshot)[0]
            .resource_text()
            .to_string(),
        "󰑹 —\n —"
    );
}

#[test]
fn resources_sum_services_and_hidden_setup_into_instances_and_templates() {
    let mut snapshot = snapshot();
    snapshot.instances[0].services[0].usage = Some(usage(240, 180));
    let mut db = snapshot.instances[0].services[0].clone();
    db.name = "db".into();
    db.container_id = "db-id".into();
    db.port = None;
    db.url = None;
    db.usage = Some(usage(80, 64));
    snapshot.instances[0].services.push(db.clone());
    let mut setup = db;
    setup.name = "setup".into();
    setup.one_shot = true;
    setup.usage = Some(usage(100, 10));
    snapshot.instances[0].services.push(setup);
    let mut other = snapshot.instances[0].clone();
    other.name = "other".into();
    other.services.truncate(1);
    other.services[0].usage = Some(usage(200, 100));
    snapshot.instances.push(other);
    let tree = rows::from_snapshot(&snapshot);
    let row = |id: &str| tree.iter().find(|row| row.id == id).unwrap();
    assert_eq!(
        row("service:review:web").resource_text().to_string(),
        "󰑹 180 MiB\n 2%"
    );
    assert_eq!(
        row("service:review:db").resource_text().to_string(),
        "󰑹 64 MiB\n <1%"
    );
    assert_eq!(
        row("instance:review").resource_text().to_string(),
        "󰑹 254 MiB\n 4%"
    );
    assert_eq!(
        row("template:/tmp/templates/website")
            .resource_text()
            .to_string(),
        "󰑹 354 MiB\n 6%"
    );
    assert!(tree.iter().all(|row| row.id != "service:review:setup"));

    snapshot.instances[0].services[1].usage = None;
    let tree = rows::from_snapshot(&snapshot);
    assert_eq!(tree[0].resource_text().to_string(), "󰑹 —\n —");
    assert_eq!(tree[1].resource_text().to_string(), "󰑹 —\n —");
    snapshot.instances[0].services[1].status = "down (exit 0)".into();
    snapshot.templates.clear();
    let tree = rows::from_snapshot(&snapshot);
    assert_eq!(tree[0].resource_text().to_string(), "󰑹 290 MiB\n 5%");
}

#[test]
fn memory_stays_visible_while_cpu_baselines_are_pending() {
    tuicore::init();
    let mut snapshot = snapshot();
    snapshot.instances[0].services[0].usage = Some(ResourceUsage {
        cpu_basis_points: None,
        ..usage(0, 180)
    });
    let mut worker = snapshot.instances[0].services[0].clone();
    worker.name = "worker".into();
    worker.container_id = "worker-id".into();
    worker.usage = Some(usage(100, 64));
    snapshot.instances[0].services.push(worker);
    let tree = rows::from_snapshot(&snapshot);
    assert_eq!(tree[0].resource_text().to_string(), "󰑹 244 MiB\n —");
    assert_eq!(tree[1].resource_text().to_string(), "󰑹 244 MiB\n —");
    assert_eq!(tree[2].resource_text().to_string(), "󰑹 180 MiB\n —");
    assert_eq!(
        tree[2].resource_text().lines[1].spans[0].style.fg,
        Some(Tone::Muted.color())
    );
}

#[test]
fn icons_use_semantic_state_colors_without_status_labels() {
    tuicore::init();
    for (status, service_tone, icon, instance_tone, loading) in [
        ("healthy", Tone::Success, "", Tone::Success, false),
        ("up", Tone::Info, "", Tone::Success, false),
        ("unhealthy", Tone::Error, "", Tone::Error, false),
        ("boot", Tone::Muted, "", Tone::Muted, true),
        ("removing", Tone::Muted, "", Tone::Muted, true),
        ("paused", Tone::Muted, "", Tone::Muted, false),
        ("down (exit 0)", Tone::Muted, "", Tone::Muted, false),
        ("down (exit 1)", Tone::Muted, "", Tone::Muted, false),
        ("down (exit 137)", Tone::Muted, "", Tone::Muted, false),
        ("down (exit 143)", Tone::Muted, "", Tone::Muted, false),
        ("unknown", Tone::Warning, "", Tone::Muted, false),
    ] {
        for routed in [true, false] {
            let mut snapshot = snapshot();
            let service = &mut snapshot.instances[0].services[0];
            service.status = status.into();
            if !routed {
                service.url = None;
                service.port = None;
            }
            let tree = rows::from_snapshot(&snapshot);
            assert_eq!(tree[1].label, "review\n/tmp/workspaces/review");
            assert_eq!(tree[1].icon, icon);
            assert_eq!(tree[1].tone, instance_tone);
            assert_eq!(
                tree[1].text("⠋").lines[0].spans[0].style.fg,
                Some(instance_tone.color())
            );
            assert_eq!(tree[1].loading, loading);
            assert_eq!(tree[2].tone, service_tone);
            assert_eq!(tree[2].icon, if routed { "󰖟" } else { "󰒋" });
            let text = tree[2].text("⠋");
            assert_eq!(text.lines[0].spans[0].style.fg, Some(service_tone.color()));
            assert_eq!(text.lines[0].spans[1].content, "web");
            assert_eq!(
                text.lines[1].spans[0].style.fg,
                Some(tuicore::theme().muted_fg())
            );
        }
    }
}

fn render(tree: &mut Instances, width: u16) -> Terminal<TestBackend> {
    let area = Rect::new(0, 0, width, 18);
    tree.layout(area, &mut tuicore::LayoutCtx::new());
    let mut terminal = Terminal::new(TestBackend::new(width, area.height)).unwrap();
    terminal
        .draw(|frame| {
            let mut ctx = RenderCtx::new();
            tree.render(frame, area, &mut ctx);
            ctx.flush(frame);
        })
        .unwrap();
    terminal
}

#[test]
fn search_results_restripe_the_visible_tree_rows() {
    tuicore::init();
    let mut snapshot = snapshot();
    let mut database = snapshot.instances[0].services[0].clone();
    database.name = "db".into();
    database.url = None;
    database.port = None;
    snapshot.instances[0].services.push(database);
    let rows = rows::from_snapshot(&snapshot);
    let state = instances::state(rows);
    let mut tree = Instances::new(state.clone());
    let mut events = EventCtx::new(AnimationSettings::default());

    tree.layout(Rect::new(0, 0, 110, 18), &mut tuicore::LayoutCtx::new());
    tree.focus(None, true, &mut tuicore::FocusCtx::default());
    tree.event(&TuiEvent::Key(KeyEvent::from(Key::Char('/'))), &mut events);
    tree.event(&TuiEvent::Key(KeyEvent::from(Key::Char('d'))), &mut events);
    tree.event(&TuiEvent::Key(KeyEvent::from(Key::Char('b'))), &mut events);
    assert!(instances::is_searching(&state));
    tree.focus(None, false, &mut tuicore::FocusCtx::default());

    let terminal = render(&mut tree, 110);
    let database = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .find(|cell| cell.symbol() == "󰒋")
        .expect("database search result");
    assert_eq!(database.bg, tuicore::theme().surface_bg());
}

#[test]
fn all_tree_rows_stack_memory_above_cpu() {
    tuicore::init();
    let mut snapshot = snapshot();
    snapshot.instances[0].services[0].status = "healthy".into();
    snapshot.instances[0].services[0].usage = Some(usage(240, 180));
    let state = instances::state(rows::from_snapshot(&snapshot));
    let mut tree = Instances::new(state);
    let terminal = render(&mut tree, 110);
    let lines = rendered_lines(&terminal, Rect::new(0, 0, 110, 18));
    let service_line = lines
        .iter()
        .position(|line| line.contains("󰖟 web"))
        .expect("routed service");
    assert!(lines[service_line].contains("󰑹 180 MiB"), "{lines:#?}");
    assert!(lines[service_line + 1].contains("http://localhost:9876/review/web/ · port 8080"));
    let resource_byte = lines[service_line].find("󰑹").unwrap();
    let resource_column = lines[service_line][..resource_byte].chars().count();
    assert_eq!(
        resource_column,
        110 - "󰑹 180 MiB".chars().count() - 1,
        "{lines:#?}"
    );
    for offset in [0, 2, 4] {
        let row_line = service_line - offset;
        let memory_column = lines[row_line].find("󰑹 180 MiB").unwrap();
        let cpu_column = lines[row_line + 1].find(" 2%").unwrap();
        assert_eq!(
            lines[row_line][..memory_column].chars().count(),
            lines[row_line + 1][..cpu_column].chars().count()
        );
    }
    let buffer = terminal.backend().buffer();
    assert_eq!(
        buffer.cell((109, service_line as u16)).unwrap().symbol(),
        " "
    );
    let gateway = buffer
        .content
        .iter()
        .find(|cell| cell.symbol() == "󰖟")
        .unwrap();
    assert_eq!(gateway.fg, tuicore::theme().success_fg());
    let narrow = render(&mut tree, 60);
    let lines = rendered_lines(&narrow, Rect::new(0, 0, 60, 18));
    assert!(
        lines.iter().any(|line| line.contains("󰑹 180 MiB")),
        "{lines:#?}"
    );
    assert_eq!(lines.iter().filter(|line| line.contains(" 2%")).count(), 3);

    snapshot.instances[0].services[0].usage = None;
    snapshot.instances[0].services[0].url = None;
    snapshot.instances[0].services[0].port = None;
    snapshot.instances[0].services[0].image = None;
    let mut tree = Instances::new(instances::state(rows::from_snapshot(&snapshot)));
    let terminal = render(&mut tree, 60);
    let lines = rendered_lines(&terminal, Rect::new(0, 0, 60, 18));
    let service_line = lines
        .iter()
        .position(|line| line.contains("󰒋 web"))
        .unwrap();
    for offset in [0, 2, 4] {
        assert!(lines[service_line - offset].contains("󰑹 —"));
        assert!(lines[service_line - offset + 1].contains(" —"));
    }
}

#[test]
fn instance_loader_animates_during_boot_and_stops_when_ready() {
    tuicore::init();
    let mut snapshot = snapshot();
    snapshot.instances[0].services[0].status = "boot".into();
    let state = instances::state(rows::from_snapshot(&snapshot));
    let mut tree = Instances::new(state.clone());
    let terminal = render(&mut tree, 110);
    let lines = rendered_lines(&terminal, Rect::new(0, 0, 110, 18));
    assert!(
        lines.iter().any(|line| line.contains("⠋ review")),
        "{lines:#?}"
    );
    let result = tree.tick(Duration::from_millis(80), AnimationSettings::default());
    assert!(result.changed && result.active);
    let terminal = render(&mut tree, 110);
    let lines = rendered_lines(&terminal, Rect::new(0, 0, 110, 18));
    assert!(
        lines.iter().any(|line| line.contains("⠙ review")),
        "{lines:#?}"
    );
    let spinner = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .find(|cell| cell.symbol() == "⠙")
        .unwrap();
    assert_eq!(spinner.fg, tuicore::theme().muted_fg());
    snapshot.instances[0].services[0].status = "healthy".into();
    instances::replace_rows(&state, rows::from_snapshot(&snapshot));
    let terminal = render(&mut tree, 110);
    let lines = rendered_lines(&terminal, Rect::new(0, 0, 110, 18));
    assert!(
        lines.iter().any(|line| line.contains(" review")),
        "{lines:#?}"
    );
}

#[test]
fn failed_setup_is_visible_while_dependent_services_are_waiting() {
    tuicore::init();
    let mut snapshot = snapshot();
    let mut setup = snapshot.instances[0].services[0].clone();
    setup.name = "repo-sync".into();
    setup.one_shot = true;
    setup.status = "down (exit 128)".into();
    snapshot.instances[0].services.push(setup);
    for status in ["created", "boot"] {
        snapshot.instances[0].services[0].status = status.into();
        let rows = rows::from_snapshot(&snapshot);
        assert!(!rows[1].loading);
        assert_eq!(rows[1].tone, Tone::Error);
        assert!(rows[1].label.contains("setup failed"));
        assert!(rows[1].details.iter().any(|property| {
            property.name == "Startup error"
                && property.value.contains("repo-sync: down (exit 128)")
        }));
        let mut tree = Instances::new(instances::state(rows));
        let terminal = render(&mut tree, 110);
        let lines = rendered_lines(&terminal, Rect::new(0, 0, 110, 18));
        assert!(
            lines
                .iter()
                .any(|line| line.contains("review · setup failed")),
            "{lines:#?}"
        );
    }
}

#[test]
fn pending_instance_rows_are_visible_and_animate_before_container_discovery() {
    tuicore::init();
    let mut snapshot = snapshot();
    snapshot.instances[0].pending = true;
    snapshot.instances[0].services.clear();
    let state = instances::state(rows::from_snapshot(&snapshot));
    let mut tree = Instances::new(state);

    let terminal = render(&mut tree, 110);
    let lines = rendered_lines(&terminal, Rect::new(0, 0, 110, 18));
    assert!(
        lines.iter().any(|line| line.contains("⠋ review")),
        "{lines:#?}"
    );

    let result = tree.tick(Duration::from_millis(80), AnimationSettings::default());
    assert!(result.changed && result.active);
}
