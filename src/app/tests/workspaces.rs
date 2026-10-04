use super::*;
use crate::store::environments::{RepositoryCheckout, StartupKind, StartupTiming};

#[test]
fn workspace_only_instances_remain_visible_in_both_views_without_opencode_clients() {
    init_ui();
    let mut inventory = snapshot();
    inventory.instances[0].services[0].status = "exited 0".into();
    let mut template = inventory.templates[0].clone();
    template.name = "guidance-only".into();
    template.directory = "/tmp/templates/guidance-only".into();
    template.compose_file.clear();
    template.compose_source.clear();
    template.guidance_source = Some("Inspect the workspace".into());
    let mut instance = inventory.instances[0].clone();
    instance.name = "folder".into();
    instance.template = template.name.clone();
    instance.template_directory = template.directory.clone();
    instance.workspace = "/tmp/workspaces/folder".into();
    instance.workspace_only = true;
    instance.runtime.workspace_ready = true;
    instance.services.clear();
    inventory.templates.push(template);
    inventory.instances.push(instance);
    for sessions in [false, true] {
        for running_only in [true, false] {
            for show_saved in [false, true] {
                let projected = crate::app::opencode::project_rows(
                    &inventory,
                    &[],
                    &Default::default(),
                    running_only,
                    show_saved,
                    sessions,
                );
                let folder = projected
                    .iter()
                    .find(|row| row.id == "instance:folder")
                    .unwrap();
                assert_eq!(folder.icon, rows::WORKSPACE_ICON);
                assert_eq!(folder.tone, rows::Tone::Success);
                if running_only {
                    assert!(!projected.iter().any(|row| row.id == "instance:review"));
                }
                if sessions {
                    assert!(folder.parent.is_none());
                } else {
                    assert_eq!(
                        folder.parent.as_deref(),
                        Some("template:/tmp/templates/guidance-only")
                    );
                    let template = projected
                        .iter()
                        .find(|row| row.id == "template:/tmp/templates/guidance-only")
                        .unwrap();
                    assert_eq!(template.tone, rows::Tone::Success);
                    assert_eq!(
                        template.text("", None).lines[0].spans[0].style.fg,
                        Some(tuicore::theme().success_fg())
                    );
                }
            }
        }
    }
}

#[test]
fn workspace_template_icons_are_green_with_instances_and_keep_empty_or_invalid_states() {
    init_ui();
    let mut inventory = snapshot();
    inventory.templates[0].compose_file.clear();
    inventory.templates[0].compose_source.clear();
    inventory.instances[0].services.clear();
    inventory.instances[0].workspace_only = true;
    inventory.instances[0].runtime.workspace_ready = true;
    for (has_instance, invalid, tone) in [
        (false, false, rows::Tone::Normal),
        (true, false, rows::Tone::Success),
        (true, true, rows::Tone::Error),
    ] {
        let mut inventory = inventory.clone();
        if !has_instance {
            inventory.instances.clear();
        }
        inventory.templates[0].error = invalid.then(|| "Invalid manifest".into());
        let projected = rows::from_snapshot(&inventory);
        assert_eq!(projected[0].icon, rows::TEMPLATE_ICON);
        assert_eq!(projected[0].tone, tone);
    }
}

#[test]
fn repository_setup_rows_precede_jobs_and_copy_absolute_checkout_paths() {
    init_ui();
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
fn workspace_rows_only_include_populated_groups() {
    init_ui();
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
    assert_eq!(instance.icon, "󱥾");
    assert_eq!(instance.tone, crate::app::rows::Tone::Success);
    let instance_text = instance.text("", None);
    assert_eq!(
        instance_text.lines[0].to_string(),
        "󱥾 review · (no description)"
    );
    assert_eq!(
        instance_text.lines[0].spans[0].style.fg,
        Some(tuicore::theme().success_fg())
    );
    assert!(instance.resource_text().to_string().is_empty());
    assert!(
        !rows
            .iter()
            .any(|row| row.parent.as_deref() == Some("instance:review"))
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
fn workspace_icons_are_green_and_stopped_icons_follow_live_opencode_sessions() {
    use crate::store::opencode::{Activity, Pane, Session, Snapshot};

    init_ui();
    let mut inventory = snapshot();
    let instance = &mut inventory.instances[0];
    instance.services.clear();
    instance.workspace_only = true;
    instance.runtime.workspace_ready = true;
    let workspace = instance.workspace.clone();
    for (activity, attached, stale, owned, green) in [
        (Activity::Idle, true, false, true, true),
        (Activity::Busy, false, false, true, true),
        (Activity::Idle, false, false, true, false),
        (Activity::Unknown, false, false, true, false),
        (Activity::Busy, true, true, true, false),
        (Activity::Busy, true, false, false, false),
    ] {
        let observation = Snapshot {
            sessions: vec![Session {
                id: "ses_workspace".into(),
                directory: if owned {
                    format!("{workspace}/repo")
                } else {
                    "/outside/workspace".into()
                },
                activity,
                stale,
                panes: if attached {
                    vec![Pane {
                        session: "main".into(),
                        id: 17,
                        tab_id: 4,
                        tab_name: "review".into(),
                    }]
                } else {
                    Vec::new()
                },
                ..Default::default()
            }],
            ..Default::default()
        };
        for (show_saved, workspace_only) in
            [(false, false), (false, true), (true, false), (true, true)]
        {
            inventory.instances[0].workspace_only = workspace_only;
            inventory.instances[0].services = if workspace_only {
                Vec::new()
            } else {
                vec![InstanceService {
                    status: "exited 0".into(),
                    ..Default::default()
                }]
            };
            let mut projected = rows::from_snapshot(&inventory);
            let inactive_tone = projected
                .iter()
                .find(|row| row.id == "instance:review")
                .unwrap()
                .tone;
            super::super::opencode::append_rows(&mut projected, &observation, show_saved);
            let row = projected
                .iter()
                .find(|row| row.id == "instance:review")
                .unwrap();
            let text = row.text("", None);
            assert_eq!(
                text.lines[0].to_string(),
                if workspace_only {
                    "󱥾 review · (no description)"
                } else {
                    " review · Stopped · (no description)"
                }
            );
            assert_eq!(
                text.lines[0].spans[0].style.fg,
                Some(if workspace_only || green {
                    tuicore::theme().success_fg()
                } else {
                    inactive_tone.color()
                }),
                "activity={activity:?}, attached={attached}, stale={stale}, owned={owned}, show_saved={show_saved}, workspace_only={workspace_only}"
            );
            assert_eq!(text.lines[0].spans[1].style.fg, None);
        }
    }
}

#[test]
fn completed_setup_icon_is_muted_when_the_instance_is_stopped() {
    init_ui();
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
