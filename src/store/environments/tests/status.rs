use super::*;
use crate::store::environments::{ResourceUsage, UsageSummary};

fn service(name: &str, state: ContainerState, health: HealthState) -> InstanceService {
    InstanceService {
        name: name.into(),
        container_id: name.into(),
        runtime: ServiceRuntime {
            state,
            health,
            replica: 1,
            ..Default::default()
        },
        ..Default::default()
    }
}

fn instance(services: Vec<InstanceService>) -> Instance {
    Instance {
        services,
        runtime: InstanceRuntime {
            topology_known: true,
            ..Default::default()
        },
        ..Default::default()
    }
}

#[test]
fn stopped_exit_codes_are_evidence_not_instance_failure() {
    for code in [0, 1, 137, 143] {
        let mut stopped = service("web", ContainerState::Exited, HealthState::Unhealthy);
        stopped.runtime.exit_code = Some(code);
        let summary = instance(vec![stopped.clone()]).status_summary();
        assert_eq!(summary.status, Status::Stopped);
        assert_eq!(
            stopped.status_summary().detail_severity,
            if code == 0 {
                Severity::Muted
            } else {
                Severity::Warning
            }
        );
        stopped.runtime.requested_stop = true;
        assert_eq!(
            stopped.status_summary().detail.as_deref(),
            Some("requested stop")
        );
        stopped.runtime.oom_killed = true;
        assert_eq!(stopped.status_summary().detail_severity, Severity::Error);
        assert_eq!(
            stopped.status_summary().detail.as_deref(),
            Some("OOM killed")
        );
    }
}

#[test]
fn running_counts_include_unhealthy_containers_but_health_requires_every_probe() {
    let healthy = service("web", ContainerState::Running, HealthState::Healthy);
    let mut app = instance(vec![healthy.clone()]);
    assert_eq!(app.status_summary().status, Status::Healthy);
    app.services.push(service(
        "db",
        ContainerState::Running,
        HealthState::Unconfigured,
    ));
    assert_eq!(app.status_summary().status, Status::Running);
    app.services[1].runtime.health = HealthState::Checking;
    assert_eq!(app.status_summary().status, Status::Running);
    app.services[1].runtime.health = HealthState::Unhealthy;
    let summary = app.status_summary();
    assert_eq!(
        (summary.status, summary.running, summary.expected),
        (Status::Degraded, 2, 2)
    );
    assert_eq!(summary.detail_severity, Severity::Error);
    app.services[1].runtime.state = ContainerState::Missing;
    assert_eq!(app.status_summary().running, 1);
    assert_eq!(app.status_summary().status, Status::Degraded);
    app.runtime.topology_known = false;
    assert_eq!(app.status_summary().status, Status::Unknown);
    assert_eq!(instance(vec![]).status_summary().status, Status::Unknown);
}

#[test]
fn paused_created_and_mixed_inactive_instances_remain_distinct() {
    for (state, expected) in [
        (ContainerState::Created, Status::NotStarted),
        (ContainerState::Paused, Status::Paused),
        (ContainerState::Exited, Status::Stopped),
        (ContainerState::Dead, Status::Degraded),
    ] {
        let app = instance(vec![service("web", state, HealthState::Unconfigured)]);
        assert_eq!(app.status_summary().status, expected);
    }
    let mut app = instance(vec![
        service("web", ContainerState::Created, HealthState::Unconfigured),
        service("db", ContainerState::Exited, HealthState::Unconfigured),
    ]);
    assert_eq!(app.status_summary().status, Status::Degraded);
    app.runtime.whole_stop = true;
    assert_eq!(app.status_summary().status, Status::Stopped);
}

#[test]
fn setup_outcomes_are_visible_and_excluded_from_runtime_counts() {
    let mut job = service("migrate", ContainerState::Exited, HealthState::Unconfigured);
    job.one_shot = true;
    job.runtime.exit_code = Some(0);
    assert_eq!(
        instance(vec![job.clone()]).status_summary().status,
        Status::Completed
    );
    job.runtime.exit_code = Some(1);
    assert_eq!(
        instance(vec![job.clone()]).status_summary().status,
        Status::Failed
    );
    job.runtime.requested_stop = true;
    assert_eq!(
        instance(vec![job.clone()]).status_summary().status,
        Status::Interrupted
    );
    let app = instance(vec![
        job,
        service("web", ContainerState::Running, HealthState::Healthy),
    ]);
    assert_eq!(
        (app.status_summary().status, app.status_summary().expected),
        (Status::Degraded, 1)
    );
}

#[test]
fn lifecycle_actions_follow_target_roles_and_state_instead_of_metrics() {
    let mut job = service(
        "migrate",
        ContainerState::Running,
        HealthState::Unconfigured,
    );
    job.one_shot = true;
    assert!(!job.can_stop());
    let mut app = instance(vec![job]);
    assert!(app.can_stop());
    assert!(!app.can_restart());
    app.services[0].runtime.state = ContainerState::Exited;
    assert!(app.can_start());
    assert!(!app.can_stop());
    let mut app = instance(vec![service(
        "web",
        ContainerState::Running,
        HealthState::Healthy,
    )]);
    app.services[0].runtime.resources_stale = true;
    app.services[0].runtime.resource_error = Some("stats unavailable".into());
    assert!(app.can_stop());
    assert!(app.can_restart());
    app.services[0].runtime.state = ContainerState::Paused;
    assert!(!app.can_start());
    assert!(!app.can_restart());
}

#[test]
fn scoped_activity_preserves_failure_and_staleness_qualifiers() {
    let mut app = instance(vec![service(
        "web",
        ContainerState::Running,
        HealthState::Unhealthy,
    )]);
    app.runtime.stale = true;
    app.runtime.activity = Some(Activity {
        id: "restart".into(),
        name: "review".into(),
        template: None,
        service: Some("web".into()),
        action: "restart_service".into(),
        owner_pid: 1,
        started_at: 0,
        deadline: 60,
        error: None,
        finished: false,
    });
    let summary = app.status_summary();
    assert_eq!(summary.label, "Restarting web");
    assert!(summary.busy);
    assert!(summary.detail.as_ref().unwrap().contains("Unhealthy"));
    assert!(summary.detail.as_ref().unwrap().contains("stale"));
    assert_eq!(summary.detail_severity, Severity::Error);
}

#[test]
fn memory_and_cpu_coverage_are_independent_and_paused_memory_counts() {
    let mut web = service("web", ContainerState::Running, HealthState::Healthy);
    web.usage = Some(ResourceUsage {
        memory_bytes: 80,
        cpu_basis_points: None,
        sampled_at_unix_seconds: 42,
    });
    let mut db = service("db", ContainerState::Paused, HealthState::Healthy);
    db.usage = Some(ResourceUsage {
        memory_bytes: 20,
        cpu_basis_points: Some(9999),
        sampled_at_unix_seconds: 42,
    });
    let app = instance(vec![web, db]);
    let usage = UsageSummary::instance(&app);
    assert_eq!(usage.memory_bytes, Some(100));
    assert_eq!(usage.cpu_basis_points, None);
    assert!(!usage.memory_partial);
    assert!(usage.cpu_partial);
    let mut app = app;
    app.services[0].usage.as_mut().unwrap().cpu_basis_points = Some(15000);
    app.services[1].runtime.resources_stale = true;
    let usage = UsageSummary::instance(&app);
    assert_eq!(usage.cpu_basis_points, Some(15000));
    assert!(usage.memory_stale);
    assert!(!usage.cpu_stale);
    let mut pending = instance(vec![]);
    pending.pending = true;
    assert!(UsageSummary::instances([&app, &pending].into_iter()).memory_partial);
}
