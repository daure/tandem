use super::*;
use crate::app::instances::{self, Instances};

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
        assert_eq!(tree[1].label, format!("review\n{display}"));
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
fn tree_secondary_lines_are_muted_and_align_with_the_row_icon() {
    tuicore::init();
    for routed in [true, false] {
        let mut snapshot = snapshot();
        snapshot.home_directory = Some("/home/test".into());
        snapshot.instances[0].workspace = "/home/test/dev/review".into();
        if !routed {
            snapshot.instances[0].services[0].url = None;
            snapshot.instances[0].services[0].port = None;
        }
        let rows = rows::from_snapshot(&snapshot);
        let mut tree = Instances::new(instances::state(rows.clone()));
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
            "http://localhost:9876/review/web/ · port 8080"
        } else {
            "nginx:latest"
        };
        for (row, detail) in rows
            .iter()
            .zip(["1 instance", "~/dev/review", service_detail])
        {
            let y = lines
                .iter()
                .position(|line| line.contains(row.icon))
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
    assert_eq!(
        rows::from_snapshot(&snapshot)[0].label,
        "website\n2 instances"
    );
    let templates = std::mem::take(&mut snapshot.templates);
    assert_eq!(
        rows::from_snapshot(&snapshot)[0].label,
        "website [missing]\n2 instances"
    );
    snapshot.templates = templates;
    snapshot.instances.clear();
    assert_eq!(
        rows::from_snapshot(&snapshot)[0].label,
        "website\n0 instances"
    );
}
