use super::*;
use crate::store::environments::RepositoryCheckout;

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
    assert_eq!(group.label, "Setup · 3 completed");
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
fn workspace_rows_have_ready_status_and_an_informational_service_placeholder() {
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
    assert_eq!(instance.status.as_deref(), Some("Workspace ready"));
    assert!(instance.resource_text().to_string().is_empty());
    let empty = rows
        .iter()
        .find(|row| row.id == "no-services:review")
        .unwrap();
    assert_eq!(empty.parent.as_deref(), Some("instance:review"));
    assert_eq!(empty.text("", None).to_string(), "(no services configured)");
    assert_eq!(empty.height(), 1);
    assert!(empty.informational);
    assert_eq!(
        empty.text("", None).lines[0].spans[0].style.fg,
        Some(tuicore::theme().muted_fg())
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
