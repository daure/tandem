use super::*;
use crate::store::environments::{RepositoryCheckout, StartupKind, StartupTiming};

#[test]
fn repository_setup_rows_precede_jobs_and_copy_absolute_checkout_paths() {
    tuicore::init();
    let mut inventory = snapshot();
    inventory.instances[0].repositories = ["z-ui", "a-api"]
        .map(|target| RepositoryCheckout {
            target: target.into(),
            path: format!("/tmp/workspaces/review/{target}"),
            cloned: target == "a-api",
        })
        .into();
    inventory.instances[0].services.push(InstanceService {
        name: "migrate".into(),
        status: "exited 0".into(),
        one_shot: true,
        ..Default::default()
    });
    let rows = rows::from_snapshot(&inventory);
    let group = rows.iter().find(|row| row.id == "setup:review").unwrap();
    assert_eq!(group.label, "Setup");
    assert_eq!(group.status_detail.as_deref(), Some("3 completed"));
    assert_eq!(group.detail_tone, crate::app::rows::Tone::Muted);
    assert_eq!(
        group.text("", None).lines[0].spans[3].style.fg,
        Some(tuicore::theme().muted_fg())
    );
    let children: Vec<_> = rows
        .iter()
        .filter(|row| row.parent.as_deref() == Some(&group.id))
        .collect();
    assert_eq!(
        children
            .iter()
            .map(|row| row.id.as_str())
            .collect::<Vec<_>>(),
        [
            "repository:review:a-api",
            "repository:review:z-ui",
            "service:review:migrate"
        ]
    );
    for row in &children[..2] {
        assert_eq!(row.height(), 1);
        assert_eq!(row.icon, "");
        assert_eq!(
            row.text("", None).lines[0].spans[0].style.fg,
            Some(tuicore::theme().success_fg())
        );
        assert!(row.tone == crate::app::rows::Tone::Success);
        assert!(row.resource_text().to_string().is_empty());
        assert!(!row.can_start && !row.can_stop && !row.can_restart);
    }
    let mut app = root(AppService::for_tests());
    app.set_rows_for_tests(rows);
    crate::app::instances::set_highlighted(&app.instances, Some("repository:review:a-api".into()));
    let mut ctx = EventCtx::new(AnimationSettings::default());
    app.handle_message(Msg::CopyName, &mut ctx);
    assert_eq!(
        ctx.clipboard_request(),
        Some("/tmp/workspaces/review/a-api")
    );
}

#[test]
fn workspace_rows_append_the_empty_state_to_a_childless_services_group() {
    tuicore::init();
    let mut inventory = snapshot();
    inventory.templates[0].compose_file.clear();
    inventory.templates[0].compose_source.clear();
    let instance = &mut inventory.instances[0];
    instance.services.clear();
    instance.workspace_only = true;
    instance.runtime.workspace_ready = true;
    let rows = rows::from_snapshot(&inventory);
    let instance = rows.iter().find(|row| row.id == "instance:review").unwrap();
    assert_eq!(instance.status, None);
    assert_eq!(instance.icon, "");
    assert_eq!(instance.tone, crate::app::rows::Tone::Normal);
    let instance_text = instance.text("", None);
    assert_eq!(instance_text.lines[0].to_string(), " review");
    assert_eq!(
        instance_text.lines[0].spans[0].style.fg,
        Some(tuicore::theme().text_fg())
    );
    assert!(instance.resource_text().to_string().is_empty());
    let services = rows.iter().find(|row| row.id == "services:review").unwrap();
    assert_eq!(services.parent.as_deref(), Some("instance:review"));
    assert_eq!(
        services.text("", None).to_string(),
        "󰒋 Services · (no services configured)"
    );
    assert_eq!(services.height(), 1);
    assert!(
        !rows
            .iter()
            .any(|row| row.parent.as_deref() == Some("services:review"))
    );
    assert_eq!(
        services.text("", None).lines[0].spans[3].style.fg,
        Some(tuicore::theme().subtle_fg())
    );
    let mut app = root(AppService::for_tests());
    app.set_rows_for_tests(rows);
    crate::app::instances::set_highlighted(&app.instances, Some("instance:review".into()));
    app.action(1, &mut EventCtx::new(AnimationSettings::default()));
    assert!(matches!(
        app.intent,
        Some(crate::app::Intent::CreateInstance(_))
    ));
}

#[test]
fn completed_setup_icon_is_muted_when_the_instance_is_stopped() {
    tuicore::init();
    let mut inventory = snapshot();
    inventory.instances[0].services[0].status = "exited 0".into();
    inventory.instances[0]
        .repositories
        .push(RepositoryCheckout {
            target: "app".into(),
            path: "/tmp/workspaces/review/app".into(),
            cloned: true,
        });
    let mut migration = inventory.instances[0].services[0].clone();
    migration.name = "migrate".into();
    migration.one_shot = true;
    migration.status = "exited 0".into();
    inventory.instances[0].services.push(migration);

    let rows = rows::from_snapshot(&inventory);
    let setup = rows.iter().find(|row| row.id == "setup:review").unwrap();
    let repository = rows
        .iter()
        .find(|row| row.id == "repository:review:app")
        .unwrap();
    let migration = rows
        .iter()
        .find(|row| row.id == "service:review:migrate")
        .unwrap();

    assert_eq!(setup.icon, "");
    assert_eq!(setup.tone, crate::app::rows::Tone::Muted);
    assert_eq!(repository.tone, crate::app::rows::Tone::Muted);
    assert_eq!(migration.tone, crate::app::rows::Tone::Muted);
}

#[test]
fn active_startup_uses_info_for_completed_setup_and_service_groups() {
    let mut inventory = snapshot();
    inventory.instances[0]
        .repositories
        .push(RepositoryCheckout {
            target: "app".into(),
            path: "/tmp/workspaces/review/app".into(),
            cloned: true,
        });
    inventory.startup.insert(
        "review".into(),
        StartupTiming {
            elapsed_milliseconds: 1_000,
            estimate_milliseconds: Some(10_000),
            kind: StartupKind::Cold,
        },
    );

    let rows = rows::from_snapshot(&inventory);
    let setup = rows.iter().find(|row| row.id == "setup:review").unwrap();
    let services = rows.iter().find(|row| row.id == "services:review").unwrap();

    assert_eq!(setup.icon, "");
    assert_eq!(setup.tone, crate::app::rows::Tone::Info);
    assert_eq!(services.tone, crate::app::rows::Tone::Info);
}
