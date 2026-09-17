use super::*;
use crate::app::instances::{self, Instances};
use crate::store::environments::StartupTiming;

#[test]
fn workspace_labels_abbreviate_only_paths_inside_home() {
    for (home, workspace, display) in [
        (Some("/home/test"), "/home/test/dev/review", "~/dev/review"),
        (Some("/home/test"), "/home/test", "~"),
        (
            Some("/home/test"),
            "/home/testing/review",
            "/home/testing/review",
        ),
        (Some("/home/test"), "/var/tmp/review", "/var/tmp/review"),
        (None, "/home/test/dev/review", "/home/test/dev/review"),
    ] {
        let mut snapshot = snapshot();
        snapshot.home_directory = home.map(str::to_owned);
        snapshot.instances[0].workspace = workspace.into();
        let tree = rows::from_snapshot(&snapshot);
        assert_eq!(tree[1].label, format!("review · Running\n{display}"));
        assert_eq!(tree[1].workspace.as_deref(), Some(workspace));
        assert_eq!(
            tree[1]
                .details
                .iter()
                .find(|row| row.name == "Workspace")
                .unwrap()
                .value,
            workspace
        );
    }
}

#[test]
fn starting_instances_show_a_bounded_countdown_then_an_overrun() {
    let mut snapshot = snapshot();
    let mut service = snapshot.instances[0].services.remove(0);
    snapshot.startup.insert(
        "review".into(),
        StartupTiming {
            elapsed_milliseconds: 6_001,
            estimate_milliseconds: Some(18_000),
        },
    );
    assert_eq!(
        rows::from_snapshot(&snapshot)[1]
            .label
            .lines()
            .next()
            .unwrap(),
        "review · Creating · 12s"
    );
    service.status = "boot".into();
    snapshot.instances[0].services.push(service);
    for (elapsed, expected) in [
        (6_001, "Starting · 12s"),
        (18_000, "Starting · taking longer than usual"),
        (31_001, "Starting · 14s over estimate"),
    ] {
        snapshot.startup.insert(
            "review".into(),
            StartupTiming {
                elapsed_milliseconds: elapsed,
                estimate_milliseconds: Some(18_000),
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
}

#[test]
fn instance_status_label_uses_its_semantic_color() {
    tuicore::init();
    let row = rows::from_snapshot(&snapshot())[1].clone();
    let text = row.text("⠋");
    assert_eq!(text.lines[0].spans[1].content, "review · ");
    assert_eq!(text.lines[0].spans[2].content, "Running");
    assert_eq!(
        text.lines[0].spans[2].style.fg,
        Some(tuicore::theme().info_fg())
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
fn tree_secondary_lines_are_muted_and_align_with_the_row_icon() {
    tuicore::init();
    for routed in [true, false] {
        let mut snapshot = snapshot();
        snapshot.home_directory = Some("/home/test".into());
        snapshot
            .startup_averages_milliseconds
            .insert("website".into(), 84_000);
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
            " http://localhost:9876/review/web/ · port 8080"
        } else {
            "nginx:latest"
        };
        for (row, detail) in rows
            .iter()
            .zip([" 1 ·  1m24s", "~/dev/review", service_detail])
        {
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
                tuicore::theme().muted_fg()
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
fn template_startup_averages_use_compact_durations_for_loaded_and_missing_templates() {
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
            .startup_averages_milliseconds
            .insert("website".into(), milliseconds);
        assert_eq!(
            rows::from_snapshot(&snapshot)[0].label,
            format!("website\n 4 ·  {duration}")
        );
    }
    snapshot.templates.clear();
    assert_eq!(
        rows::from_snapshot(&snapshot)[0].label,
        "website [missing]\n 4 ·  2m05s"
    );
}
