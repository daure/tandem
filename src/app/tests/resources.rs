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
fn resource_values_use_compact_units_and_preserve_tiny_nonzero_samples() {
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
        (100, 100 * 1048576, "1%", "100 MiB"),
        (100, 101 * 1048576, "1%", "0.1 GiB"),
        (100, 180 * 1048576, "1%", "0.2 GiB"),
        (100, 999 * 1048576, "1%", "1 GiB"),
        (100, 1000 * 1048576, "1%", "1 GiB"),
        (100, 1073741824, "1%", "1 GiB"),
        (100, 1610612736, "1%", "1.5 GiB"),
        (u64::MAX, u64::MAX, "184467440737095516%", "17179869184 GiB"),
    ] {
        let mut snapshot = snapshot();
        snapshot.instances[0].services[0].usage = Some(ResourceUsage {
            cpu_basis_points: Some(cpu),
            memory_bytes: memory,
            sampled_at_unix_seconds: 42,
        });
        let tree = rows::from_snapshot(&snapshot);
        let row = tree
            .iter()
            .find(|row| row.id == "service:review:web")
            .unwrap();
        assert_eq!(
            row.resource_text().to_string(),
            format!("{memory_text}\n{cpu_text}")
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
    assert_eq!(text.to_string(), "—\n—");
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
        for row in tree
            .iter()
            .filter(|row| row.details.iter().any(|property| property.name == "Memory"))
        {
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
        expand_first_instance(&mut view);
        let terminal = render(&mut view, 130);
        let lines = rendered_lines(&terminal, Rect::new(0, 0, 130, 18));
        let y = lines.iter().position(|line| line.contains("web")).unwrap();
        let memory = crate::app::details::memory(used * 1048576);
        let prefix = lines[y].split(&memory).next().unwrap();
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
        "90 MiB …\n1% …"
    );
}

#[test]
fn resources_sum_services_and_active_setup_into_instances_and_templates() {
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
        "0.2 GiB\n2%"
    );
    assert_eq!(
        row("service:review:db").resource_text().to_string(),
        "64 MiB\n<1%"
    );
    assert_eq!(
        row("instance:review").resource_text().to_string(),
        "0.2 GiB\n4%"
    );
    assert_eq!(
        row("template:/tmp/templates/website")
            .resource_text()
            .to_string(),
        "0.3 GiB\n6%"
    );
    assert!(tree.iter().any(|row| row.id == "service:review:setup"));

    snapshot.instances[0].services[1].usage = None;
    snapshot.instances[0].services[1].runtime.resource_error = Some("stats unavailable".into());
    let tree = rows::from_snapshot(&snapshot);
    let row = |id: &str| tree.iter().find(|row| row.id == id).unwrap();
    assert_eq!(
        row("template:/tmp/templates/website")
            .resource_text()
            .to_string(),
        "0.3 GiB\n5%"
    );
    assert_eq!(
        row("instance:review").resource_text().to_string(),
        "0.2 GiB\n3%"
    );
    assert_eq!(row("service:review:db").resource_text().to_string(), "—\n—");
    snapshot.instances[0].services[1].status = "down (exit 0)".into();
    snapshot.templates.clear();
    let tree = rows::from_snapshot(&snapshot);
    assert_eq!(tree[0].resource_text().to_string(), "0.3 GiB\n5%");
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
    assert_eq!(tree[0].resource_text().to_string(), "0.2 GiB\n1% …");
    assert_eq!(tree[1].resource_text().to_string(), "0.2 GiB\n1% …");
    let web = tree
        .iter()
        .find(|row| row.id == "service:review:web")
        .unwrap();
    assert_eq!(web.resource_text().to_string(), "0.2 GiB\n— …");
    assert_eq!(web.memory_text().to_string(), "");
    assert_eq!(web.cpu_text_with_spinner("⠋").to_string(), "⠋");
    assert_eq!(
        web.resource_text().lines[1].spans[0].style.fg,
        Some(Tone::Muted.color())
    );
}

#[test]
fn instance_summaries_use_the_most_actionable_service_state() {
    tuicore::init();
    for (status, service_tone, summary, icon, instance_tone, loading) in [
        (
            "healthy",
            Tone::Success,
            "Healthy",
            "",
            Tone::Success,
            false,
        ),
        ("up", Tone::Info, "Running", "", Tone::Info, false),
        (
            "unhealthy",
            Tone::Error,
            "Degraded",
            "",
            Tone::Warning,
            false,
        ),
        ("boot", Tone::Info, "Running", "", Tone::Info, false),
        (
            "removing",
            Tone::Info,
            "Degraded",
            "",
            Tone::Warning,
            false,
        ),
        ("paused", Tone::Warning, "Paused", "", Tone::Warning, false),
        (
            "down (exit 0)",
            Tone::Muted,
            "Stopped",
            "",
            Tone::Muted,
            false,
        ),
        (
            "down (exit 1)",
            Tone::Muted,
            "Stopped",
            "",
            Tone::Muted,
            false,
        ),
        (
            "down (exit 137)",
            Tone::Muted,
            "Stopped",
            "",
            Tone::Muted,
            false,
        ),
        (
            "down (exit 143)",
            Tone::Muted,
            "Stopped",
            "",
            Tone::Muted,
            false,
        ),
        (
            "unknown",
            Tone::Warning,
            "Unknown",
            "",
            Tone::Warning,
            false,
        ),
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
            assert_eq!(tree[1].label, format!("review · {summary}"));
            assert_eq!(tree[1].icon, icon);
            assert_eq!(tree[1].tone, instance_tone);
            assert_eq!(
                tree[1].text("⠋", None).lines[0].spans[0].style.fg,
                Some(instance_tone.color())
            );
            assert_eq!(tree[1].loading, loading);
            let service = tree
                .iter()
                .find(|row| row.id == "service:review:web")
                .unwrap();
            assert_eq!(service.tone, service_tone);
            assert!(!service.icon.is_empty());
            let text = service.text("⠋", None);
            assert_eq!(text.lines[0].spans[0].style.fg, Some(service_tone.color()));
            assert_eq!(text.lines[0].spans[1].content, "web · ");
            assert_eq!(
                text.lines[1].spans[0].style.fg,
                Some(tuicore::theme().muted_fg())
            );
        }
    }
}

#[test]
fn service_group_icon_summarizes_runtime_service_health() {
    tuicore::init();
    for (statuses, expected) in [
        (["healthy", "up"], Tone::Success),
        (["down (exit 0)", "down (exit 0)"], Tone::Muted),
        (["healthy", "down (exit 0)"], Tone::Warning),
        (["healthy", "boot"], Tone::Info),
        (["boot", "created"], Tone::Info),
        (["healthy", "unhealthy"], Tone::Error),
    ] {
        let mut snapshot = snapshot();
        let mut second = snapshot.instances[0].services[0].clone();
        second.name = "worker".into();
        second.container_id = "worker-id".into();
        snapshot.instances[0].services.push(second);
        for (service, status) in snapshot.instances[0].services.iter_mut().zip(statuses) {
            service.status = status.into();
        }

        let tree = rows::from_snapshot(&snapshot);
        let group = tree.iter().find(|row| row.id == "services:review").unwrap();
        assert_eq!(group.icon, "󰒋");
        assert_eq!(group.tone, expected, "{statuses:?}");
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
        .rev()
        .find(|cell| cell.symbol() == "")
        .expect("database search result");
    assert_eq!(database.bg, ratatui::style::Color::Reset);
}

#[test]
fn resource_rows_show_right_aligned_memory_and_cpu_columns() {
    tuicore::init();
    let mut snapshot = snapshot();
    snapshot.instances[0].services[0].status = "healthy".into();
    snapshot.instances[0].services[0].usage = Some(usage(240, 180));
    let state = instances::state(rows::from_snapshot(&snapshot));
    let mut tree = Instances::new(state);
    expand_first_instance(&mut tree);
    let terminal = render(&mut tree, 110);
    let lines = rendered_lines(&terminal, Rect::new(0, 0, 110, 18));
    let service_line = lines
        .iter()
        .position(|line| line.contains(" web"))
        .expect("routed service");
    assert!(lines[service_line].contains(" 0.2 GiB 2% "), "{lines:#?}");
    assert!(lines[service_line + 1].contains("http://localhost:9876/review/web/ · 󰈀 8080"));
    for offset in [0, 3, 5] {
        let row_line = service_line - offset;
        assert!(lines[row_line].ends_with(" 0.2 GiB 2% "), "{lines:#?}");
    }
    let buffer = terminal.backend().buffer();
    assert_eq!(
        buffer.cell((109, service_line as u16)).unwrap().symbol(),
        " "
    );
    let gateway = buffer
        .content
        .iter()
        .find(|cell| cell.symbol() == "")
        .unwrap();
    assert_eq!(gateway.fg, tuicore::theme().muted_fg());
    let narrow = render(&mut tree, 60);
    let lines = rendered_lines(&narrow, Rect::new(0, 0, 60, 18));
    assert!(
        lines.iter().any(|line| line.contains(" 0.2 GiB 2% ")),
        "{lines:#?}"
    );
    assert_eq!(
        lines
            .iter()
            .filter(|line| line.ends_with(" 0.2 GiB 2% "))
            .count(),
        3
    );
    for line in lines.iter().filter(|line| line.contains("0.2 GiB")) {
        assert!(line.ends_with(" 0.2 GiB 2% "), "{lines:#?}");
    }

    snapshot.instances[0].services[0].usage = None;
    snapshot.instances[0].services[0].url = None;
    snapshot.instances[0].services[0].port = None;
    snapshot.instances[0].services[0].image = None;
    let mut tree = Instances::new(instances::state(rows::from_snapshot(&snapshot)));
    expand_first_instance(&mut tree);
    let terminal = render(&mut tree, 60);
    let lines = rendered_lines(&terminal, Rect::new(0, 0, 60, 18));
    let service_line = lines
        .iter()
        .position(|line| line.contains(" web"))
        .unwrap();
    for offset in [0, 2, 4] {
        assert!(!lines[service_line - offset].contains('—'));
    }
}

#[test]
fn resource_rows_show_one_loading_spinner_after_cpu() {
    tuicore::init();
    let mut inventory = snapshot();
    inventory.instances[0].services[0].usage = Some(usage(240, 180));
    let mut starting = inventory.instances[0].clone();
    starting.name = "starting".into();
    starting.pending = true;
    inventory.instances.push(starting);
    let rows = rows::from_snapshot(&inventory);
    let memory = rows[0].memory_text();
    let cpu = rows[0].cpu_text_with_spinner("⠋");
    assert_eq!(memory.spans[0].style.fg, Some(Tone::Muted.color()));
    assert_eq!(cpu.spans[0].style.fg, Some(Tone::Muted.color()));
    let mut tree = Instances::new(instances::state(rows));

    let terminal = render(&mut tree, 110);
    let lines = rendered_lines(&terminal, Rect::new(0, 0, 110, 18));
    let template = lines.iter().find(|line| line.contains("website")).unwrap();

    assert!(template.contains("0.2 GiB 2% ⠋"), "{template}");
    assert!(!template.contains("GiB ⠋"), "{template}");
}

#[test]
fn template_capabilities_follow_each_name_while_resources_stay_right_aligned() {
    tuicore::init();
    let mut snapshot = snapshot();
    snapshot.instances[0].services[0].usage = Some(usage(240, 180));
    snapshot.templates[0].manifest = serde_json::from_value(serde_json::json!({
        "repositories": [
            {"source": "/source/web", "target": "web"},
            {"source": "/source/api", "target": "api"}
        ],
        "routes": {"web": {"port": 8080, "readiness_path": "", "readiness_contains": "ready"}}
    }))
    .unwrap();
    snapshot.templates[0].guidance_source = Some("Guidance".into());
    let mut guidance = snapshot.templates[0].clone();
    guidance.name = "a-long-guidance-template-name".into();
    guidance.directory = "/tmp/templates/guidance".into();
    guidance.compose_file.clear();
    guidance.compose_source.clear();
    guidance.manifest = Manifest::default();
    snapshot.templates.push(guidance);
    let rows = rows::from_snapshot(&snapshot)
        .into_iter()
        .filter(|row| row.parent.is_none())
        .collect();
    let mut tree = Instances::new(instances::state(rows));

    for width in [110, 80, 60, 46, 110] {
        let terminal = render(&mut tree, width);
        let lines = rendered_lines(&terminal, Rect::new(0, 0, width, 18));
        let template_y = lines
            .iter()
            .position(|line| line.contains("website"))
            .unwrap();
        let template = &lines[template_y];
        let guidance = &lines[template_y + 2];
        let icons = "󰡨 󰳐 󰈀 󱓷";
        let capability_x = template[..template.find(icons).unwrap()].chars().count();
        assert!(template.contains("website 󰡨 󰳐 󰈀 󱓷"), "{template}");
        assert!(
            guidance.contains("a-long-guidance-template-name 󱓷"),
            "{guidance}"
        );
        assert!(template.ends_with(" 0.2 GiB 2% "), "{template}");
        assert!(lines[template_y + 1].contains(" 1/1"));
        assert_eq!(
            terminal
                .backend()
                .buffer()
                .cell((capability_x as u16, template_y as u16))
                .unwrap()
                .fg,
            tuicore::theme().muted_fg()
        );
    }
}

#[test]
fn cpu_column_shows_only_the_loader_before_the_first_sample() {
    tuicore::init();
    let tree = rows::from_snapshot(&snapshot());
    let service = tree
        .iter()
        .find(|row| row.id == "service:review:web")
        .unwrap();

    assert_eq!(service.cpu_text_with_spinner("⠋").to_string(), "⠋");
}

#[test]
fn setup_group_uses_one_line_and_its_children_show_only_labels() {
    tuicore::init();
    for image in [Some("setup-image"), None] {
        let mut snapshot = snapshot();
        let mut setup = snapshot.instances[0].services[0].clone();
        setup.name = "migrate".into();
        setup.one_shot = true;
        setup.status = "exited 0".into();
        setup.url = None;
        setup.port = None;
        setup.image = image.map(str::to_owned);
        snapshot.instances[0].services.push(setup);
        let mut tree = Instances::new(instances::state(rows::from_snapshot(&snapshot)));
        expand_first_instance(&mut tree);
        let terminal = render(&mut tree, 110);
        let lines = rendered_lines(&terminal, Rect::new(0, 0, 110, 18));
        let group = lines
            .iter()
            .position(|line| line.contains("Setup · 1 completed"))
            .unwrap();
        assert!(lines[group].trim_end().ends_with("Setup · 1 completed"));
        assert!(lines[group + 1].contains("Service"), "{lines:#?}");
        assert!(lines[group + 2].contains("web · Running"), "{lines:#?}");

        tree.focus(None, true, &mut tuicore::FocusCtx::default());
        let mut events = EventCtx::new(AnimationSettings::default());
        for key in [Key::Home, Key::Down, Key::Down, Key::Right] {
            tree.event(&TuiEvent::Key(KeyEvent::from(key)), &mut events);
        }
        tree.focus(None, false, &mut tuicore::FocusCtx::default());
        let terminal = render(&mut tree, 110);
        let lines = rendered_lines(&terminal, Rect::new(0, 0, 110, 18));
        assert!(
            lines[group + 1].trim_end().ends_with("migrate · Completed"),
            "{lines:#?}"
        );
        if let Some(image) = image {
            assert!(lines[group + 2].trim_end().ends_with(image), "{lines:#?}");
        }
        assert!(
            lines.iter().any(|line| line.contains("web · Running")),
            "{lines:#?}"
        );
    }
}

#[test]
fn health_check_loader_animates_on_the_service_until_ready() {
    tuicore::init();
    let mut snapshot = snapshot();
    snapshot.instances[0].services[0].status = "boot".into();
    let state = instances::state(rows::from_snapshot(&snapshot));
    let mut tree = Instances::new(state.clone());
    expand_first_instance(&mut tree);
    let terminal = render(&mut tree, 110);
    let lines = rendered_lines(&terminal, Rect::new(0, 0, 110, 18));
    assert!(
        lines.iter().any(|line| line.contains("⠋ web")),
        "{lines:#?}"
    );
    let result = tree.tick(Duration::from_millis(80), AnimationSettings::default());
    assert!(result.changed && result.active);
    let terminal = render(&mut tree, 110);
    let lines = rendered_lines(&terminal, Rect::new(0, 0, 110, 18));
    assert!(
        lines.iter().any(|line| line.contains("⠙ web")),
        "{lines:#?}"
    );
    let spinner = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .find(|cell| cell.symbol() == "⠙" && cell.fg == tuicore::theme().info_fg())
        .unwrap();
    assert_eq!(spinner.fg, tuicore::theme().info_fg());
    snapshot.instances[0].services[0].status = "healthy".into();
    instances::replace_rows(&state, rows::from_snapshot(&snapshot));
    let terminal = render(&mut tree, 110);
    let lines = rendered_lines(&terminal, Rect::new(0, 0, 110, 18));
    assert!(
        lines.iter().any(|line| line.contains(" review")),
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
        assert_eq!(rows[1].detail_tone, Tone::Error);
        assert!(
            rows[1]
                .status_detail
                .as_ref()
                .unwrap()
                .contains("repo-sync: Failed")
        );
        assert_eq!(
            rows.iter()
                .find(|row| row.id == "service:review:repo-sync")
                .unwrap()
                .status
                .as_deref(),
            Some("Failed")
        );
        assert!(rows[1].details.iter().any(|property| {
            property.name == "Startup error"
                && property.value.contains("repo-sync: down (exit 128)")
        }));
        let mut tree = Instances::new(instances::state(rows));
        let terminal = render(&mut tree, 110);
        let lines = rendered_lines(&terminal, Rect::new(0, 0, 110, 18));
        assert!(
            lines.iter().any(|line| line.contains("repo-sync: Failed")),
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

#[test]
fn new_instance_expands_when_it_appears_without_expanding_its_services() {
    tuicore::init();
    let mut empty = snapshot();
    empty.instances.clear();
    let state = instances::state(rows::from_snapshot(&empty));
    let mut tree = Instances::new(state.clone());
    render(&mut tree, 110);

    let mut pending = snapshot();
    pending.instances[0].pending = true;
    pending.instances[0].services.clear();
    instances::replace_rows(&state, rows::from_snapshot(&pending));
    render(&mut tree, 110);

    let mut expected = snapshot().instances[0].services.remove(0);
    expected.status = "created".into();
    pending.instances[0].services.push(expected);
    instances::replace_rows(&state, rows::from_snapshot(&pending));
    let terminal = render(&mut tree, 110);
    let lines = rendered_lines(&terminal, Rect::new(0, 0, 110, 18));
    assert!(lines.iter().any(|line| line.contains("review")));
    assert!(lines.iter().any(|line| line.contains("Service")));
    assert!(!lines.iter().any(|line| line.contains(" web")));
    expand_first_instance(&mut tree);
    let terminal = render(&mut tree, 110);
    let lines = rendered_lines(&terminal, Rect::new(0, 0, 110, 18));
    assert!(
        lines.iter().any(|line| line.contains(" web")),
        "{lines:#?}"
    );
}

#[test]
fn startup_expands_instances_when_they_arrive_after_the_template_listing() {
    tuicore::init();
    let mut inventory = snapshot();
    let mut second_template = inventory.templates[0].clone();
    second_template.name = "other".into();
    second_template.directory = "/tmp/templates/other".into();
    inventory.templates.push(second_template);
    let mut second = inventory.instances[0].clone();
    second.name = "second".into();
    second.template = "other".into();
    second.template_directory = "/tmp/templates/other".into();
    inventory.instances.push(second);
    let state = instances::state(Vec::new());
    let mut tree = Instances::new(state.clone());
    let mut templates_only = inventory.clone();
    templates_only.instances.clear();
    instances::replace_rows(&state, rows::from_snapshot(&templates_only));
    render(&mut tree, 110);
    instances::replace_rows(&state, rows::from_snapshot(&inventory));
    let terminal = render(&mut tree, 110);
    let text = rendered_lines(&terminal, Rect::new(0, 0, 110, 18)).join("\n");
    assert!(text.contains("review · Running"));
    assert!(text.contains("second · Running"));
    assert!(text.contains("Service"));
    assert!(!text.contains(" web"));
    tree.focus(None, true, &mut tuicore::FocusCtx::default());
    tree.event(
        &TuiEvent::Key(KeyEvent::from(Key::Left)),
        &mut EventCtx::new(AnimationSettings::default()),
    );
    instances::replace_rows(&state, rows::from_snapshot(&inventory));
    let terminal = render(&mut tree, 110);
    let text = rendered_lines(&terminal, Rect::new(0, 0, 110, 18)).join("\n");
    assert!(text.contains("review · Running"));
    assert!(!text.contains("second · Running"));
}

#[test]
fn template_totals_exclude_instances_that_are_still_starting() {
    let mut inventory = snapshot();
    inventory.instances[0].services[0].usage = Some(usage(240, 180));
    let mut starting = inventory.instances[0].clone();
    starting.name = "starting".into();
    starting.services[0].usage = None;
    starting.pending = true;
    inventory.instances.push(starting);
    inventory
        .startup
        .insert("starting".into(), Default::default());
    assert_eq!(
        rows::from_snapshot(&inventory)[0]
            .resource_text()
            .to_string(),
        "0.2 GiB …\n2% …"
    );
    inventory.startup.clear();
    inventory.instances[1].pending = false;
    inventory.instances[1].services[0].usage = Some(usage(100, 20));
    assert_eq!(
        rows::from_snapshot(&inventory)[0]
            .resource_text()
            .to_string(),
        "0.2 GiB\n3%"
    );
}
