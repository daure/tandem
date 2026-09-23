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
    for (elapsed, expected) in [
        (6_001, "󰜗 Starting 12s"),
        (18_000, "󰜗 Starting taking longer than usual"),
        (31_001, "󰜗 Starting 14s over estimate"),
    ] {
        snapshot.startup.insert(
            "review".into(),
            StartupTiming {
                elapsed_milliseconds: elapsed,
                estimate_milliseconds: Some(18_000),
                kind: StartupKind::Cold,
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
fn startup_detail_separator_uses_the_normal_text_color() {
    tuicore::init();
    let mut snapshot = snapshot();
    snapshot.startup.insert(
        "review".into(),
        StartupTiming {
            elapsed_milliseconds: 6_001,
            estimate_milliseconds: Some(18_000),
            kind: StartupKind::Hot,
        },
    );

    let text = rows::from_snapshot(&snapshot)[1].text("⠋", None);
    assert_eq!(text.lines[0].spans[3].content, " · ");
    assert_eq!(
        text.lines[0].spans[3].style.fg,
        Some(tuicore::theme().text_fg())
    );
}

#[test]
fn active_startup_rows_show_the_latest_progress_beneath_the_status() {
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
    assert!(text.lines[0].to_string().contains(progress));
    assert_eq!(text.lines[1].to_string(), "Review environment");

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
    tuicore::init();
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
fn instance_description_always_uses_the_muted_second_line() {
    tuicore::init();
    let mut row = rows::from_snapshot(&snapshot())[1].clone();
    row.description = "A long description for this environment".into();
    let text = row.text("⠋", Some(70));
    assert_eq!(text.lines[1].to_string(), row.description);
    assert_eq!(
        text.lines[1].spans[0].style.fg,
        Some(tuicore::theme().muted_fg())
    );

    row.description.clear();
    let text = row.text("⠋", Some(70));
    assert_eq!(text.lines[1].to_string(), "(no description)");
    assert_eq!(
        text.lines[1].spans[0].style.fg,
        Some(tuicore::theme().subtle_fg())
    );
}

#[test]
fn instance_rows_sort_by_name_before_rendering() {
    let mut snapshot = snapshot();
    let mut alpha = snapshot.instances[0].clone();
    alpha.name = "alpha".into();
    snapshot.instances.push(alpha);
    let instance_ids = rows::from_snapshot(&snapshot)
        .into_iter()
        .filter(|row| row.instance.is_some())
        .map(|row| row.id)
        .collect::<Vec<_>>();
    assert_eq!(instance_ids, ["instance:alpha", "instance:review"]);
}

#[test]
fn tree_secondary_lines_use_semantic_colors_and_align_with_the_row_icon() {
    tuicore::init();
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
        for (row, (detail, color)) in rows.iter().zip([
            (" 1 · 󰜗 1m24s · 󰈸 12s", tuicore::theme().muted_fg()),
            ("(no description)", tuicore::theme().subtle_fg()),
            (service_detail, tuicore::theme().muted_fg()),
        ]) {
            let y = lines
                .iter()
                .position(|line| line.contains(row.label.lines().next().unwrap()))
                .unwrap();
            let x = lines[y][..lines[y].find(row.icon).unwrap()].chars().count();
            let secondary_x = lines[y + 1][..lines[y + 1].find(detail).unwrap()]
                .chars()
                .count();
            assert_eq!(secondary_x, x, "{}: {lines:#?}", row.id);
            assert_eq!(
                terminal
                    .backend()
                    .buffer()
                    .cell((x as u16, y as u16 + 1))
                    .unwrap()
                    .fg,
                color
            );
        }
    }
}

#[test]
fn template_secondary_counts_cover_empty_plural_and_missing_templates() {
    let mut snapshot = snapshot();
    let mut second = snapshot.instances[0].clone();
    second.name = "second".into();
    snapshot.instances.push(second);
    assert_eq!(rows::from_snapshot(&snapshot)[0].label, "website\n 2");
    let templates = std::mem::take(&mut snapshot.templates);
    assert_eq!(
        rows::from_snapshot(&snapshot)[0].label,
        "website [missing]\n 2"
    );
    snapshot.templates = templates;
    snapshot.instances.clear();
    assert_eq!(rows::from_snapshot(&snapshot)[0].label, "website\n 0");
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
        (59_001, "1m00s"),
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
            format!("website\n 4 · 󰜗 {duration} · 󰈸 12s")
        );
    }
    snapshot.templates.clear();
    assert_eq!(
        rows::from_snapshot(&snapshot)[0].label,
        "website [missing]\n 4 · 󰜗 2m05s · 󰈸 12s"
    );
}
