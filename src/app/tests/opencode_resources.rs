use super::*;
use crate::{
    app::opencode as projection,
    store::{
        environments::{ResourceUsage, UsageSummary},
        opencode::{Client, Pane, Session, Snapshot, resources::ProcessResource},
    },
};

fn pane(id: u32) -> Pane {
    Pane {
        session: "main".into(),
        id,
        tab_id: 1,
        tab_name: "work".into(),
    }
}

fn process(pid: u32, session_id: &str, directory: &str, memory_mib: u64) -> ProcessResource {
    ProcessResource {
        pid,
        session_id: session_id.into(),
        directory: directory.into(),
        zellij_session: "main".into(),
        pane_id: Some(pid),
        usage: UsageSummary {
            memory_bytes: Some(memory_mib * 1_048_576),
            cpu_basis_points: Some(memory_mib * 10),
            ..Default::default()
        },
        ..Default::default()
    }
}

fn observations() -> (EnvironmentSnapshot, Snapshot) {
    let mut inventory = snapshot();
    inventory.instances[0].services[0].usage = Some(ResourceUsage {
        memory_bytes: 10 * 1_048_576,
        cpu_basis_points: Some(100),
        ..Default::default()
    });
    let mut job = inventory.instances[0].services[0].clone();
    job.name = "setup".into();
    job.container_id = "job".into();
    job.one_shot = true;
    job.usage = Some(ResourceUsage {
        memory_bytes: 2 * 1_048_576,
        cpu_basis_points: Some(20),
        ..Default::default()
    });
    inventory.instances[0].services.push(job);
    let mut other = inventory.instances[0].clone();
    other.name = "other".into();
    other.workspace = "/tmp/workspaces/other".into();
    other.workspace_only = true;
    other.services.clear();
    inventory.instances.push(other);
    let mut observation = Snapshot {
        sessions: vec![
            Session {
                id: "ses_owned".into(),
                directory: "/tmp/workspaces/review/repo".into(),
                panes: vec![pane(7), pane(8)],
                ..Default::default()
            },
            Session {
                id: "ses_external".into(),
                directory: "/outside".into(),
                panes: vec![pane(21)],
                ..Default::default()
            },
            Session {
                id: "ses_other".into(),
                directory: "/tmp/workspaces/other".into(),
                panes: vec![pane(30)],
                ..Default::default()
            },
        ],
        clients: vec![
            Client {
                title: "OpenCode".into(),
                directory: "/tmp/workspaces/review".into(),
                server: String::new(),
                pane: pane(13),
                stale: false,
                awaiting_presence_since: None,
            },
            Client {
                title: "OpenCode".into(),
                directory: "/outside/blank".into(),
                server: String::new(),
                pane: pane(22),
                stale: false,
                awaiting_presence_since: None,
            },
        ],
        resources: vec![
            process(7, "ses_owned", "/tmp/workspaces/review/repo", 10),
            process(8, "ses_owned", "/tmp/workspaces/review/repo", 20),
            process(13, "", "/tmp/workspaces/review", 5),
            process(21, "ses_external", "/outside", 40),
            process(22, "", "/outside/blank", 50),
            process(30, "ses_other", "/tmp/workspaces/other", 60),
        ],
        ..Default::default()
    };
    observation.resources.push(observation.resources[0].clone());
    (inventory, observation)
}

fn assert_usage(rows: &[rows::Row], id: &str, memory_mib: u64) {
    let row = rows
        .iter()
        .find(|row| row.id == id)
        .unwrap_or_else(|| panic!("Missing {id}"));
    assert!(!row.hide_resources, "{id}");
    assert_eq!(
        row.metrics.memory_bytes,
        Some(memory_mib * 1_048_576),
        "{id}"
    );
    assert_eq!(row.metrics.cpu_basis_points, Some(memory_mib * 10), "{id}");
}

#[test]
fn every_resource_parent_sums_its_scope_without_counting_panes_or_pids_twice() {
    init_ui();
    let (inventory, observation) = observations();
    let mut rows = rows::from_snapshot(&inventory);
    projection::append_rows(&mut rows, &observation, false);
    for (id, mib) in [
        ("service:review:web", 10),
        ("services:review", 10),
        ("service:review:setup", 2),
        ("setup:review", 2),
        ("opencode:review:ses_owned:main:7", 10),
        ("opencode:review:ses_owned:main:8", 20),
        ("opencode:review:ses_owned", 30),
        ("opencode-client:review:main:13", 5),
        ("sessions:review", 35),
        ("instance:review", 47),
        ("sessions:other", 60),
        ("instance:other", 60),
        ("template:/tmp/templates/website", 107),
        ("opencode-workspace:/outside", 40),
        ("opencode-workspace:/outside/blank", 50),
        ("opencode-workspaces", 90),
    ] {
        assert_usage(&rows, id, mib);
    }

    let agents = projection::attached_rows(rows::from_snapshot(&inventory), &observation);
    for (id, mib) in [
        ("instance:review", 47),
        ("instance:other", 60),
        ("opencode:review:ses_owned", 30),
        ("opencode-workspace:/outside", 40),
        ("opencode-workspace:/outside/blank", 50),
    ] {
        assert_usage(&agents, id, mib);
    }
    let total = crate::app::toolbar::State::from_snapshots(&inventory, &observation).totals;
    assert_eq!(total.memory_bytes, Some(197 * 1_048_576));
    assert_eq!(total.cpu_basis_points, Some(1970));
}

#[test]
fn totals_publish_client_usage_on_the_shared_resource_batch_and_survive_view_filters() {
    init_ui();
    let (mut inventory, mut observation) = observations();
    inventory.resource_revision = Some(1);
    inventory.available_memory_bytes = Some(8 * 1_073_741_824);
    inventory.cpu_temperature_millicelsius = Some(65_000);
    let service = AppService::for_tests();
    service.set_opencode_snapshot_for_tests(observation.clone());
    let mut app = crate::app::root(service);
    app.update_snapshot(inventory.clone());
    assert_eq!(
        app.toolbar_state.borrow().totals.memory_bytes,
        Some(197 * 1_048_576)
    );
    observation.resources[0].usage.memory_bytes = Some(15 * 1_048_576);
    observation.resources[0].usage.cpu_basis_points = Some(150);
    app.service
        .set_opencode_snapshot_for_tests(observation.clone());
    assert!(app.update_snapshot(inventory.clone()));
    assert_eq!(
        app.toolbar_state.borrow().totals.memory_bytes,
        Some(197 * 1_048_576)
    );
    inventory.resource_revision = Some(2);
    inventory.available_memory_bytes = Some(7 * 1_073_741_824);
    inventory.cpu_temperature_millicelsius = Some(70_000);
    assert!(app.update_snapshot(inventory.clone()));
    assert_eq!(
        app.toolbar_state.borrow().totals.memory_bytes,
        Some(202 * 1_048_576)
    );
    assert_eq!(
        app.toolbar_state.borrow().totals.cpu_basis_points,
        Some(2020)
    );
    assert_eq!(
        app.toolbar_state.borrow().available_memory_bytes,
        Some(7 * 1_073_741_824)
    );
    assert_eq!(
        app.toolbar_state.borrow().cpu_temperature_millicelsius,
        Some(70_000)
    );
    for attached in [true, false] {
        app.attached_sessions_only = attached;
        app.running_only = true;
        app.opencode_history = true;
        let rows = app.project_rows(&inventory, &[]);
        let instance = rows.iter().find(|row| row.id == "instance:review").unwrap();
        assert_eq!(instance.metrics.memory_bytes, Some(52 * 1_048_576));
        if !attached {
            let template = rows
                .iter()
                .find(|row| row.id.starts_with("template:"))
                .unwrap();
            assert_eq!(template.metrics.memory_bytes, Some(112 * 1_048_576));
        }
        assert_eq!(
            app.toolbar_state.borrow().totals.memory_bytes,
            Some(202 * 1_048_576)
        );
    }
    tokio::runtime::Runtime::new()
        .unwrap()
        .block_on(app.service.set_opencode_enabled(false).unwrap())
        .unwrap()
        .unwrap();
    app.update_snapshot(inventory);
    assert_eq!(
        app.toolbar_state.borrow().totals.memory_bytes,
        Some(12 * 1_048_576)
    );
}

#[test]
fn all_open_sessions_contribute_to_parent_resource_totals() {
    init_ui();
    let (inventory, mut observation) = observations();
    for id in 100..125 {
        observation.sessions.push(Session {
            id: format!("ses_{id}"),
            directory: "/tmp/workspaces/review/repo".into(),
            panes: vec![pane(id)],
            updated: u64::from(id),
            ..Default::default()
        });
        observation.resources.push(process(
            id,
            &format!("ses_{id}"),
            "/tmp/workspaces/review/repo",
            1,
        ));
    }
    let mut rows = rows::from_snapshot(&inventory);
    projection::append_rows(&mut rows, &observation, false);
    assert_eq!(rows.iter().filter(|row| row.parent.as_deref() == Some("sessions:review") && row.opencode.is_some()).count(), 27);
    assert_usage(&rows, "sessions:review", 60);
    assert_usage(&rows, "instance:review", 72);
    assert_usage(&rows, "template:/tmp/templates/website", 132);
}

#[test]
fn client_failures_preserve_partial_coverage_and_uncapped_totals() {
    init_ui();
    let (mut inventory, mut observation) = observations();
    inventory.instances[0].services[0].memory_limit_bytes = Some(20 * 1_048_576);
    observation.resources[0].mark_stale("permission denied".into());
    let mut rows = rows::from_snapshot(&inventory);
    projection::append_rows(&mut rows, &observation, false);
    for id in [
        "opencode:review:ses_owned",
        "sessions:review",
        "instance:review",
        "template:/tmp/templates/website",
    ] {
        let row = rows.iter().find(|row| row.id == id).unwrap();
        assert!(
            row.metrics.memory_partial && row.metrics.cpu_partial,
            "{id}"
        );
        assert_eq!(row.metrics.memory_limit_bytes, None, "{id}");
    }
    let totals = crate::app::toolbar::State::from_snapshots(&inventory, &observation).totals;
    assert!(totals.memory_partial && totals.cpu_partial);
}

#[test]
fn agent_view_renders_client_and_instance_usage_in_the_right_columns() {
    init_ui();
    let mut inventory = snapshot();
    inventory.instances[0].services[0].usage = Some(ResourceUsage {
        memory_bytes: 10 * 1_048_576,
        cpu_basis_points: Some(100),
        ..Default::default()
    });
    let service = AppService::for_tests();
    service.set_opencode_snapshot_for_tests(Snapshot {
        sessions: vec![Session {
            id: "ses_owned".into(),
            title: "Inspect resources".into(),
            directory: "/tmp/workspaces/review".into(),
            panes: vec![pane(7)],
            ..Default::default()
        }],
        resources: vec![process(7, "ses_owned", "/tmp/workspaces/review", 10)],
        ..Default::default()
    });
    let mut app = crate::app::root(service);
    app.update_snapshot(inventory);
    for width in [60, 130] {
        let area = Rect::new(0, 0, width, 20);
        app.layout(area, &mut tuicore::LayoutCtx::new());
        let mut terminal = Terminal::new(TestBackend::new(width, 20)).unwrap();
        terminal
            .draw(|frame| {
                let mut ctx = RenderCtx::new();
                app.render(frame, area, &mut ctx);
                ctx.flush(frame);
            })
            .unwrap();
        let lines = rendered_lines(&terminal, area);
        let session = lines
            .iter()
            .find(|line| line.contains("Inspect resources"))
            .unwrap();
        assert!(session.ends_with(" 10 MiB 1% "), "{session}");
        let instance = lines.iter().find(|line| line.contains("review")).unwrap();
        assert!(instance.ends_with(" 20 MiB 2% "), "{instance}");
    }
}
