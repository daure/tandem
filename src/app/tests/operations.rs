use std::time::Duration;

use super::*;
use crate::app::{Instances, instances, operations::Deletion};
use crate::store::environments::{Operation, OperationState};

fn operation(action: &str, name: &str) -> Operation {
    Operation {
        id: "test-operation".into(),
        action: action.into(),
        name: name.into(),
        template: None,
        service: None,
        state: OperationState::Running,
        progress: Vec::new(),
        warnings: Vec::new(),
        elapsed_seconds: 0,
        elapsed_milliseconds: 0,
        error: None,
        instance: None,
    }
}

#[test]
fn template_operation_scope_ignores_unrelated_instances_with_the_same_name() {
    let mut other = snapshot().instances.remove(0);
    other.name = "website".into();
    other.template = "other".into();
    assert!(!operation("stop_template", "website").targets(&other));
    assert!(!operation("create_template", "website").targets(&other));
    assert!(operation("stop_instance", "website").targets(&other));
    other.template = "website".into();
    assert!(operation("stop_template", "website").targets(&other));
}

#[test]
fn successful_deletion_with_close_failure_shows_a_warning_once() {
    let mut op = operation("delete_instance", "review");
    let mut deletion = Deletion::new(op.clone()).unwrap();
    op.state = OperationState::Succeeded;
    op.warnings
        .push("Close command for review failed; continuing deletion".into());
    let mut notifications = Vec::new();
    for _ in 0..2 {
        deletion.project(&mut snapshot(), |_| Ok(op.clone()), &mut notifications);
    }
    assert_eq!(notifications.len(), 1);
    assert_eq!(notifications[0].kind(), tuicore::NotificationKind::Warning);
    assert_eq!(notifications[0].title(), "Instance deleted");
    assert!(
        notifications[0]
            .body()
            .contains("Close command for review failed")
    );
}

#[test]
fn instance_operations_only_block_their_target_instance() {
    tuicore::init();
    let service = AppService::for_tests();
    service.queue_instance_for_tests("review", "website");
    let mut app = root(service);
    let mut ctx = EventCtx::new(AnimationSettings::default());

    app.intent = Some(crate::app::Intent::Stop("other".into()));
    app.handle_message(Msg::Submit, &mut ctx);
    assert_eq!(app.service.operations().len(), 2);
    assert!(!app.view.first().is_active());

    app.intent = Some(crate::app::Intent::Stop("review".into()));
    app.handle_message(Msg::Submit, &mut ctx);
    assert_eq!(app.service.operations().len(), 2);
    let notification = app.notifications.center().history().last().unwrap();
    assert_eq!(notification.title(), "Operation in progress");
    assert!(notification.body().contains("this instance's operation"));
}

#[test]
fn pending_container_operations_keep_notifications_silent() {
    tuicore::init();
    for action in [
        "restart_instance",
        "restart_service",
        "start_service",
        "stop_service",
    ] {
        let service = AppService::for_tests();
        let mut operation = service.queue_instance_for_tests("review", "website");
        operation.action = action.into();
        let mut app = root(service);
        app.operation_accepted(operation);
        app.sync_environment();
        assert_eq!(app.container_operations.len(), 1);
        assert_eq!(app.notifications.center().history().len(), 0);
    }
}

#[test]
fn deletes_remain_visible_until_completion_and_notify_once() {
    tuicore::init();
    for (action, name, title, template_count) in [
        ("delete_instance", "review", "Instance deleted", 1),
        ("remove_template", "website", "Template deleted", 0),
        ("delete_template", "website", "Instances purged", 1),
    ] {
        let mut op = operation(action, name);
        let mut deletion = Deletion::new(op.clone()).unwrap();
        let mut notifications = Vec::new();
        for _ in 0..2 {
            let mut inventory = snapshot();
            assert!(deletion.project(&mut inventory, |_| Ok(op.clone()), &mut notifications));
            assert_eq!(inventory.instances.len(), 1);
            assert_eq!(inventory.templates.len(), 1);
            assert!(notifications.is_empty());
        }
        op.state = OperationState::Succeeded;
        for _ in 0..2 {
            let mut inventory = snapshot();
            assert!(deletion.project(&mut inventory, |_| Ok(op.clone()), &mut notifications));
            assert!(inventory.instances.is_empty());
            assert_eq!(inventory.templates.len(), template_count);
        }
        assert_eq!(notifications.len(), 1);
        assert_eq!(notifications[0].title(), title);
        assert_eq!(notifications[0].kind(), tuicore::NotificationKind::Success);
        let mut refreshed = snapshot();
        refreshed.instances.clear();
        if template_count == 0 {
            refreshed.templates.clear();
        }
        assert!(!deletion.project(
            &mut refreshed,
            |_| panic!("already completed"),
            &mut notifications
        ));
        assert_eq!(notifications.len(), 1);
    }
}

#[test]
fn failed_deletes_restore_inventory_and_report_the_failure() {
    tuicore::init();
    for (action, name) in [
        ("delete_instance", "review"),
        ("remove_template", "website"),
        ("delete_template", "website"),
    ] {
        let mut op = operation(action, name);
        let mut deletion = Deletion::new(op.clone()).unwrap();
        let mut notifications = Vec::new();
        let mut inventory = snapshot();
        assert!(deletion.project(&mut inventory, |_| Ok(op.clone()), &mut notifications));
        op.state = OperationState::Failed;
        op.error = Some("container is locked".into());
        let mut inventory = snapshot();
        assert!(!deletion.project(&mut inventory, |_| Ok(op), &mut notifications));
        assert_eq!(inventory, snapshot());
        assert_eq!(notifications.len(), 1);
        assert_eq!(notifications[0].kind(), tuicore::NotificationKind::Error);
        assert!(notifications[0].body().contains("container is locked"));
    }
}

#[test]
fn completed_purge_hides_matching_instances_and_preserves_other_templates() {
    tuicore::init();
    let mut inventory = snapshot();
    let mut second = inventory.instances[0].clone();
    second.name = "second".into();
    inventory.instances.push(second.clone());
    second.name = "unrelated".into();
    second.template = "other".into();
    second.template_directory = "/tmp/templates/other".into();
    inventory.instances.push(second);
    let mut op = operation("delete_template", "website");
    op.state = OperationState::Succeeded;
    let mut deletion = Deletion::new(op.clone()).unwrap();
    assert!(deletion.project(&mut inventory, |_| Ok(op), &mut Vec::new()));
    assert_eq!(inventory.instances.len(), 1);
    assert_eq!(inventory.instances[0].name, "unrelated");
    assert_eq!(inventory.templates.len(), 1);
}

#[test]
fn created_items_are_selected_on_arrival_without_reselecting_on_refresh() {
    tuicore::init();
    for (action, name, target) in [
        ("create_instance", "review", "instance:review"),
        (
            "create_template",
            "website",
            "template:/tmp/templates/website",
        ),
    ] {
        let mut initial = snapshot();
        initial.instances.clear();
        if action == "create_template" {
            initial.templates.clear();
        }
        let state = instances::state(rows::from_snapshot(&initial));
        let mut tree = Instances::new(state.clone());
        tree.focus(None, true, &mut tuicore::FocusCtx::default());
        let settings = AnimationSettings::default();
        tree.tick(Duration::ZERO, settings);
        instances::select_created(&state, &operation(action, name));
        tree.tick(Duration::ZERO, settings);
        instances::replace_rows(&state, rows::from_snapshot(&snapshot()));
        tree.tick(Duration::ZERO, settings);
        assert_eq!(instances::selected(&state).unwrap().id, target);
        if action == "create_template" {
            tree.event(
                &TuiEvent::Key(KeyEvent::from(Key::Right)),
                &mut EventCtx::new(settings),
            );
        }
        tree.event(
            &TuiEvent::Key(KeyEvent::from(if action == "create_instance" {
                Key::Up
            } else {
                Key::Down
            })),
            &mut EventCtx::new(settings),
        );
        let selected = instances::selected(&state).unwrap().id;
        assert_ne!(selected, target);
        instances::replace_rows(&state, rows::from_snapshot(&snapshot()));
        tree.tick(Duration::ZERO, settings);
        assert_eq!(instances::selected(&state).unwrap().id, selected);
    }
}

#[test]
fn completed_instance_deletion_moves_to_the_next_visible_row_then_the_previous() {
    tuicore::init();
    let mut inventory = snapshot();
    for name in ["alpha", "zulu"] {
        let mut instance = inventory.instances[0].clone();
        instance.name = name.into();
        inventory.instances.push(instance);
    }
    let state = instances::state(rows::from_snapshot(&inventory));
    let mut tree = Instances::new(state.clone());
    instances::select_created(&state, &operation("create_instance", "review"));
    tree.tick(Duration::ZERO, AnimationSettings::default());
    assert_eq!(instances::selected(&state).unwrap().id, "instance:review");
    for (name, expected) in [
        ("review", "instance:zulu"),
        ("zulu", "instance:alpha"),
        ("alpha", "template:/tmp/templates/website"),
    ] {
        let mut op = operation("delete_instance", name);
        op.state = OperationState::Succeeded;
        let mut deletion = Deletion::new(op.clone()).unwrap();
        deletion.project(&mut inventory, |_| Ok(op), &mut Vec::new());
        instances::replace_rows(&state, rows::from_snapshot(&inventory));
        tree.tick(Duration::ZERO, AnimationSettings::default());
        assert_eq!(instances::selected(&state).unwrap().id, expected);
    }
}

#[test]
fn duplicate_new_instance_focuses_existing_and_notifies_without_another_operation() {
    tuicore::init();
    let service = AppService::for_tests();
    let existing = service.queue_instance_for_tests("review", "website");
    service.complete_instance_for_tests(&existing.id, snapshot().instances.remove(0));
    let mut app = root(service);
    let mut ctx = EventCtx::new(AnimationSettings::default());
    for template in ["website", "different-template"] {
        app.intent = Some(crate::app::Intent::CreateInstance(template.into()));
        app.open_name_entry(&mut ctx);
        app.handle_message(Msg::NameChanged("review".into()), &mut ctx);
        app.handle_message(Msg::Submit, &mut ctx);
        app.layout(Rect::new(0, 0, 130, 40), &mut tuicore::LayoutCtx::new());
        assert_eq!(app.selected().unwrap().id, "instance:review");
        assert!(!app.view.first().is_active());
        assert!(app.intent.is_none());
        let operations = app.service.operations();
        assert_eq!(operations.len(), 1);
        assert_eq!(operations[0].id, existing.id);
        assert_eq!(operations[0].state, OperationState::Succeeded);
        assert_eq!(operations[0].progress, ["Queued"]);
        let notice = app.notifications.center().history().last().unwrap();
        assert_eq!(notice.title(), "Instance already exists");
        assert_eq!(notice.body(), "review already exists.");
        assert_eq!(notice.kind(), tuicore::NotificationKind::Info);
    }
}

#[test]
fn failed_instance_cleanup_rows_offer_confirmed_deletion_but_active_and_template_rows_do_not() {
    tuicore::init();
    for (action, finished, enabled) in [
        ("delete_instance", true, true),
        ("delete_instance", false, false),
        ("remove_template", true, false),
    ] {
        let mut inventory = snapshot();
        inventory.instances.clear();
        inventory
            .activities
            .push(crate::store::environments::Activity {
                id: "cleanup".into(),
                name: "review".into(),
                template: Some("website".into()),
                service: None,
                action: action.into(),
                owner_pid: 1,
                started_at: 1,
                deadline: u64::MAX,
                error: finished.then(|| "instance not found".into()),
                finished,
            });
        let mut app = root(AppService::for_tests());
        app.snapshot = inventory.clone();
        app.set_rows_for_tests(rows::from_snapshot(&inventory));
        instances::set_highlighted(&app.instances, Some("operation:cleanup:review".into()));
        app.action(5, &mut EventCtx::new(AnimationSettings::default()));
        assert_eq!(
            matches!(&app.intent, Some(crate::app::Intent::Delete(name)) if name == "review"),
            enabled,
            "{action}, finished={finished}"
        );
        assert_eq!(app.view.first().is_active(), enabled);
        assert!(app.service.operations().is_empty());
        if enabled {
            app.handle_message(Msg::Close, &mut EventCtx::new(AnimationSettings::default()));
            let area = Rect::new(0, 0, 130, 40);
            app.layout(area, &mut tuicore::LayoutCtx::new());
            instances::set_highlighted(&app.instances, Some("operation:cleanup:review".into()));
            assert!(app.open_action_menu(&mut EventCtx::new(AnimationSettings::default())));
            app.layout(area, &mut tuicore::LayoutCtx::new());
            let mut terminal = Terminal::new(TestBackend::new(area.width, area.height)).unwrap();
            terminal
                .draw(|frame| {
                    let mut render = RenderCtx::new();
                    app.render(frame, area, &mut render);
                    render.flush(frame);
                })
                .unwrap();
            assert!(
                rendered_lines(&terminal, area)
                    .iter()
                    .any(|line| line.contains("Retry cleanup"))
            );
        }
    }
}
