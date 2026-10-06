use super::*;
use crate::app::instances::{self, Instances};
use crate::store::environments::{Activity, Operation, OperationState, StartupKind, StartupTiming};

#[test]
fn starting_instances_show_a_bounded_countdown_then_an_overrun() {
    let mut snapshot = snapshot();
    let mut service = snapshot.instances[0].services.remove(0);
    snapshot.startup.insert(
        "review".into(),
        StartupTiming {
            elapsed_milliseconds: 6_001,
            estimate_milliseconds: Some(18_000),
            kind: StartupKind::Cold,
            ..Default::default()
        },
    );
    assert_eq!(
        rows::from_snapshot(&snapshot)[1]
            .label
            .lines()
            .next()
            .unwrap(),
        "review · 󰜗 Creating 12s"
    );
    service.status = "boot".into();
    snapshot.instances[0].services.push(service);
    for (elapsed, estimate, expected) in [
        (6_001, 18_000, "󰜗 Starting 12s"),
        (18_000, 18_000, "󰜗 Starting taking longer than usual"),
        (31_001, 18_000, "󰜗 Starting 14s over estimate"),
        (18_001, 108_000, "󰜗 Starting 1m30s"),
        (48_000, 108_000, "󰜗 Starting 1m"),
        (168_001, 108_000, "󰜗 Starting 1m01s over estimate"),
    ] {
        snapshot.startup.insert(
            "review".into(),
            StartupTiming {
                elapsed_milliseconds: elapsed,
                estimate_milliseconds: Some(estimate),
                kind: StartupKind::Cold,
                ..Default::default()
            },
        );
        assert_eq!(
            rows::from_snapshot(&snapshot)[1]
                .label
                .lines()
                .next()
                .unwrap(),
            format!("review · {expected}")
        );
    }
    snapshot.startup.insert(
        "review".into(),
        StartupTiming {
            elapsed_milliseconds: 6_001,
            estimate_milliseconds: Some(18_000),
            kind: StartupKind::Hot,
            ..Default::default()
        },
    );
    assert_eq!(
        rows::from_snapshot(&snapshot)[1]
            .label
            .lines()
            .next()
            .unwrap(),
        "review · 󰈸 Starting 12s"
    );
}

#[test]
fn service_group_detail_separator_uses_the_normal_text_color() {
    init_ui();
    let rows = rows::from_snapshot(&snapshot());
    let services = rows.iter().find(|row| row.id == "services:review").unwrap();
    let text = services.text("⠋", None);
    assert_eq!(text.lines[0].spans[2].content, " · ");
    assert_eq!(
        text.lines[0].spans[2].style.fg,
        Some(tuicore::theme().text_fg())
    );
}

#[test]
fn active_startup_rows_show_the_latest_progress_inline() {
    let progress = "Waiting for service health and gateway content assertions";
    let operation = Operation {
        id: "operation-id".into(),
        action: "create_instance".into(),
        name: "review".into(),
        template: Some("website".into()),
        service: None,
        state: OperationState::Running,
        progress: vec!["Queued".into(), progress.into()],
        warnings: Vec::new(),
        elapsed_seconds: 0,
        elapsed_milliseconds: 0,
        error: None,
        instance: None,
    };

    let mut pending = snapshot();
    pending.instances[0].pending = true;
    pending.instances[0].description = "Review environment".into();
    pending.instances[0].services.clear();
    let instance = rows::from_snapshot_with_operations(&pending, std::slice::from_ref(&operation))
        .into_iter()
        .find(|row| row.id == "instance:review")
        .unwrap();
    assert_eq!(instance.label, "review · Creating");
    assert_eq!(instance.status_detail.as_deref(), Some(progress));
    let text = instance.text("⠋", None);
    assert_eq!(instance.height(), 1);
    assert_eq!(
        text.to_string(),
        format!("⠋ review · Creating · {progress} · Review environment")
    );

    let mut activity = snapshot();
    activity.instances.clear();
    activity.activities.push(Activity {
        id: "activity-id".into(),
        name: "review".into(),
        template: Some("website".into()),
        service: None,
        action: "create_instance".into(),
        owner_pid: 1,
        started_at: 0,
        deadline: 0,
        error: None,
        finished: false,
    });
    let operation = rows::from_snapshot_with_operations(&activity, &[operation])
        .into_iter()
        .find(|row| row.id == "operation:activity-id:review")
        .unwrap();
    assert_eq!(operation.label, format!("review · Starting\n{progress}"));
    assert!(operation.hide_resources);
    assert_eq!(operation.height(), 1);
    assert_eq!(
        operation.text("⠋", None).to_string(),
        format!("⠋ review · Starting 󰡨 · {progress}")
    );

    let fallback = rows::from_snapshot_with_operations(&activity, &[])
        .into_iter()
        .find(|row| row.id == "operation:activity-id:review")
        .unwrap();
    assert_eq!(
        fallback.label,
        "review · Starting\nPreparing workspace and services"
    );
}

#[test]
fn instance_status_label_uses_its_semantic_color() {
    init_ui();
    let row = rows::from_snapshot(&snapshot())[1].clone();
    let text = row.text("⠋", None);
    assert_eq!(text.lines[0].spans[1].content, "review · ");
    assert_eq!(text.lines[0].spans[2].content, "Running");
    assert_eq!(
        text.lines[0].spans[2].style.fg,
        Some(tuicore::theme().info_fg())
    );
}

#[test]
fn instance_description_uses_muted_inline_text_and_a_subtle_placeholder() {
    init_ui();
    let mut row = rows::from_snapshot(&snapshot())[1].clone();
    row.description = "A long description\nfor this environment".into();
    let text = row.text("⠋", Some(70));
    assert_eq!(text.lines.len(), 1);
    assert_eq!(row.height(), 1);
    assert_eq!(
        text.to_string(),
        " review · Running · A long description for this environment"
    );
    assert_eq!(
        text.lines[0].spans.last().unwrap().style.fg,
        Some(tuicore::theme().muted_fg())
    );

    row.description.clear();
    let text = row.text("⠋", Some(70));
    assert_eq!(text.to_string(), " review · Running · (no description)");
    assert_eq!(
        text.lines[0].spans.last().unwrap().style.fg,
        Some(tuicore::theme().subtle_fg())
    );
}

#[test]
fn instance_rows_sort_running_then_transitioning_then_down_with_names_breaking_ties() {
    use crate::store::environments::Activity;

    let mut snapshot = snapshot();
    let instance = snapshot.instances[0].clone();
    snapshot.instances = [
        ("a-down", "down (exit 0)", None),
        ("z-running", "up", None),
        ("c-stopping", "up", Some("stop_instance")),
        ("d-stopping", "down (exit 0)", Some("stop_instance")),
        ("b-starting", "up", Some("create_instance")),
        ("a-starting", "down (exit 0)", Some("create_instance")),
        ("b-down", "down (exit 0)", None),
        ("y-running", "healthy", None),
    ]
    .into_iter()
    .map(|(name, status, action)| {
        let mut instance = instance.clone();
        instance.name = name.into();
        instance.services[0].status = status.into();
        instance.runtime.activity = action.map(|action| Activity {
            id: name.into(),
            name: name.into(),
            template: Some("website".into()),
            service: None,
            action: action.into(),
            owner_pid: std::process::id(),
            started_at: 1,
            deadline: u64::MAX,
            error: None,
            finished: false,
        });
        instance
    })
    .collect();
    let instance_ids = rows::from_snapshot(&snapshot)
        .into_iter()
        .filter(|row| row.instance.is_some())
        .map(|row| row.id)
        .collect::<Vec<_>>();
    assert_eq!(
        instance_ids,
        [
            "instance:y-running",
            "instance:z-running",
            "instance:a-starting",
            "instance:b-starting",
            "instance:c-stopping",
            "instance:d-stopping",
            "instance:a-down",
            "instance:b-down",
        ]
    );
}

#[test]
fn tree_details_share_one_line_with_the_label_and_keep_semantic_colors() {
    init_ui();
    for routed in [true, false] {
        let mut snapshot = snapshot();
        snapshot.home_directory = Some("/home/test".into());
        snapshot
            .cold_startup_averages_milliseconds
            .insert("website".into(), 84_000);
        snapshot
            .hot_startup_averages_milliseconds
            .insert("website".into(), 12_000);
        snapshot.instances[0].workspace = "/home/test/dev/review".into();
        if !routed {
            snapshot.instances[0].services[0].url = None;
            snapshot.instances[0].services[0].port = None;
        }
        let rows = rows::from_snapshot(&snapshot);
        let mut tree = Instances::new(instances::state(rows.clone()));
        expand_first_instance(&mut tree);
        let area = Rect::new(0, 0, 130, 18);
        tree.layout(area, &mut tuicore::LayoutCtx::new());
        let mut terminal = Terminal::new(TestBackend::new(area.width, area.height)).unwrap();
        terminal
            .draw(|frame| {
                let mut ctx = RenderCtx::new();
                tree.render(frame, area, &mut ctx);
                ctx.flush(frame);
            })
            .unwrap();
        let lines = rendered_lines(&terminal, area);
        let service_detail = if routed {
            " http://localhost:9876/review/web/ · 󰈀 8080"
        } else {
            "nginx:latest"
        };
        let visible_rows = [
            rows.iter().find(|row| row.parent.is_none()).unwrap(),
            rows.iter().find(|row| row.id == "instance:review").unwrap(),
            rows.iter()
                .find(|row| row.id == "service:review:web")
                .unwrap(),
        ];
        for (row, (detail, color)) in visible_rows.into_iter().zip([
            (" 1/1 · 󰜗 1m24s · 󰈸 12s", tuicore::theme().muted_fg()),
            ("(no description)", tuicore::theme().subtle_fg()),
            (service_detail, tuicore::theme().muted_fg()),
        ]) {
            let y = lines
                .iter()
                .position(|line| line.contains(row.label.lines().next().unwrap()))
                .unwrap();
            assert_eq!(row.height(), 1);
            assert_eq!(row.text("⠋", None).lines.len(), 1);
            assert!(lines[y].contains(&format!(" · {detail}")), "{lines:#?}");
            let detail_x = lines[y][..lines[y].find(detail).unwrap()].chars().count();
            assert_eq!(
                terminal
                    .backend()
                    .buffer()
                    .cell((detail_x as u16, y as u16))
                    .unwrap()
                    .fg,
                color
            );
        }
    }
}

#[test]
fn template_rows_show_running_counts_and_color_the_icon_when_any_instance_is_up() {
    init_ui();
    let mut snapshot = snapshot();
    let mut second = snapshot.instances[0].clone();
    second.name = "second".into();
    snapshot.instances.push(second);
    let template = rows::from_snapshot(&snapshot)[0].clone();
    assert_eq!(template.label, "website\n 2/2");
    assert_eq!(template.tone, rows::Tone::Success);
    assert_eq!(
        template.text("⠋", None).lines[0].spans[0].style.fg,
        Some(tuicore::theme().success_fg())
    );
    let templates = std::mem::take(&mut snapshot.templates);
    assert_eq!(
        rows::from_snapshot(&snapshot)[0].label,
        "website [missing]\n 2/2"
    );
    snapshot.templates = templates;
    for instance in &mut snapshot.instances {
        instance.services[0].status = "down (exit 0)".into();
    }
    let template = rows::from_snapshot(&snapshot)[0].clone();
    assert_eq!(template.label, "website\n 0/2");
    assert_eq!(template.tone, rows::Tone::Normal);
    snapshot.instances.clear();
    assert_eq!(rows::from_snapshot(&snapshot)[0].label, "website\n 0/0");
}

#[test]
fn template_capabilities_follow_compose_repositories_routes_guidance_order() {
    for (compose, repositories, routes, guidance, expected) in [
        (false, 0, false, false, ""),
        (false, 0, false, true, "󱓷"),
        (false, 1, false, false, "󰳏"),
        (false, 1, false, true, "󰳏 󱓷"),
        (false, 2, false, false, "󰳐"),
        (true, 0, false, false, "󰡨"),
        (true, 0, false, true, "󰡨 󱓷"),
        (true, 1, false, false, "󰡨 󰳏"),
        (true, 1, false, true, "󰡨 󰳏 󱓷"),
        (true, 0, true, false, "󰡨 󰈀"),
        (true, 1, true, true, "󰡨 󰳏 󰈀 󱓷"),
        (true, 2, true, true, "󰡨 󰳐 󰈀 󱓷"),
    ] {
        let mut snapshot = snapshot();
        let template = &mut snapshot.templates[0];
        if !compose {
            template.compose_file.clear();
            template.compose_source.clear();
        }
        template.manifest.repositories = (0..repositories)
            .map(|index| crate::store::environments::Repository {
                source: format!("/source/repo-{index}"),
                target: format!("repo-{index}"),
            })
            .collect();
        if routes {
            template.manifest.routes.insert(
                "web".into(),
                crate::store::environments::Route {
                    port: 8080,
                    strip_prefix: true,
                    readiness_path: String::new(),
                    readiness_contains: "ready".into(),
                },
            );
        }
        template.guidance_source = guidance.then(String::new);
        let tree = rows::from_snapshot(&snapshot);
        assert_eq!(tree[0].template_capabilities, expected);
        assert_eq!(
            tree[0].text("⠋", None).to_string(),
            if expected.is_empty() {
                "󰠲 website ·  1/1".to_owned()
            } else {
                format!("󰠲 website · {expected} ·  1/1")
            }
        );
        assert!(
            tree.iter()
                .skip(1)
                .all(|row| row.template_capabilities.is_empty())
        );

        snapshot.templates[0].error = Some("invalid manifest".into());
        assert!(
            rows::from_snapshot(&snapshot)[0]
                .template_capabilities
                .is_empty()
        );
        snapshot.templates.clear();
        assert!(
            rows::from_snapshot(&snapshot)[0]
                .template_capabilities
                .is_empty()
        );
    }
}

#[test]
fn template_startup_averages_show_cold_and_hot_compact_durations() {
    let mut snapshot = snapshot();
    for name in ["second", "third", "fourth"] {
        let mut instance = snapshot.instances[0].clone();
        instance.name = name.into();
        snapshot.instances.push(instance);
    }
    for (milliseconds, duration) in [
        (0, "0s"),
        (1, "1s"),
        (59_001, "1m"),
        (84_000, "1m24s"),
        (125_000, "2m05s"),
    ] {
        snapshot
            .cold_startup_averages_milliseconds
            .insert("website".into(), milliseconds);
        snapshot
            .hot_startup_averages_milliseconds
            .insert("website".into(), 12_000);
        assert_eq!(
            rows::from_snapshot(&snapshot)[0].label,
            format!("website\n 4/4 · 󰜗 {duration} · 󰈸 12s")
        );
    }
    snapshot.templates.clear();
    assert_eq!(
        rows::from_snapshot(&snapshot)[0].label,
        "website [missing]\n 4/4 · 󰜗 2m05s · 󰈸 12s"
    );
}
