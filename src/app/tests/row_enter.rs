use super::*;
use crate::app::{Intent, instances};

fn select(app: &mut App, id: &str) -> EventRoute {
    super::events::render(app, 130);
    instances::set_highlighted(&app.instances, Some(id.into()));
    let (layout, _) = super::events::render(app, 130);
    assert_eq!(app.selected().as_ref().map(|row| row.id.as_str()), Some(id));
    let target = layout
        .focus_targets()
        .iter()
        .find(|target| target.id.as_str() == crate::app::TREE_FOCUS)
        .unwrap();
    app.dispatch_focus(target, true, &mut tuicore::FocusCtx::default());
    EventRoute::new(target.path.clone())
}

fn activate(app: &mut App, id: &str, label: &str, menu: bool) {
    let route = select(app, id);
    let mut ctx = EventCtx::default();
    if menu {
        app.dispatch_event(&route, &TuiEvent::Key(Key::Char('.').into()), &mut ctx);
        let (_, text) = super::events::render(app, 130);
        let action = text.lines().find(|line| line.contains(label)).unwrap();
        assert!(action.contains("Enter"), "{action}");
        for character in label.chars() {
            app.event(&TuiEvent::Key(Key::Char(character).into()), &mut ctx);
        }
        app.event(&TuiEvent::Key(Key::Enter.into()), &mut ctx);
    } else {
        app.dispatch_event(&route, &TuiEvent::Key(Key::Enter.into()), &mut ctx);
    }
}

#[test]
fn sessions_instance_enter_and_menu_select_the_instance_on_the_instances_page() {
    init_ui();
    for menu in [false, true] {
        for status in ["running", "stopped", "workspace"] {
            let mut inventory = snapshot();
            let mut other = inventory.instances[0].clone();
            other.name = "other".into();
            other.workspace = "/tmp/workspaces/other".into();
            inventory.instances.push(other);
            match status {
                "stopped" => inventory.instances[0].services[0].status = "down (exit 0)".into(),
                "workspace" => {
                    inventory.instances[0].workspace_only = true;
                    inventory.instances[0].services.clear();
                }
                _ => {}
            }
            let mut app = root(AppService::for_tests());
            app.service
                .set_opencode_snapshot_for_tests(super::attached_sessions::observation());
            app.update_snapshot(inventory);
            let route = select(&mut app, "instance:other");
            for character in "/other".chars() {
                app.dispatch_event(
                    &route,
                    &TuiEvent::Key(Key::Char(character).into()),
                    &mut EventCtx::default(),
                );
            }
            app.dispatch_event(
                &route,
                &TuiEvent::Key(Key::Enter.into()),
                &mut EventCtx::default(),
            );
            app.handle_message(Msg::SetAttachedSessionsOnly(true), &mut EventCtx::default());
            activate(&mut app, "instance:review", "Goto instance", menu);
            let (_, text) = super::events::render(&mut app, 130);
            assert_eq!(app.tabs_mut().selected_index(), app.instances_tab_index());
            assert!(!app.attached_sessions_only);
            assert_eq!(app.selected().unwrap().id, "instance:review");
            assert_eq!(
                app.selected().unwrap().parent.as_deref(),
                Some("template:/tmp/templates/website")
            );
            assert!(text.contains("review"), "{text}");
            assert!(!app.view.is_active());
            assert!(!app.route_layer().is_active());
            assert!(app.service.opened_system_targets().is_empty());
            assert!(app.service.operations().is_empty());
            assert!(app.opencode_action.is_none());
        }
    }
}

#[test]
fn template_enter_and_menu_open_the_creation_form_without_starting_an_instance() {
    for menu in [false, true] {
        let mut app = root(AppService::for_tests());
        app.update_snapshot(snapshot());
        activate(
            &mut app,
            "template:/tmp/templates/website",
            "New instance",
            menu,
        );
        assert!(matches!(&app.intent, Some(Intent::CreateInstance(name)) if name == "website"));
        assert!(app.view.is_active());
        assert!(app.service.operations().is_empty());
    }
}

#[test]
fn instance_enter_and_menu_open_its_routes_without_changing_runtime_state() {
    for menu in [false, true] {
        for multiple in [false, true] {
            let mut inventory = snapshot();
            if multiple {
                inventory.instances[0].services.push(InstanceService {
                    name: "api".into(),
                    url: Some("http://localhost:9876/review/api/".into()),
                    ..Default::default()
                });
            }
            let mut app = root(AppService::for_tests());
            app.update_snapshot(inventory);
            activate(&mut app, "instance:review", "Open routes", menu);
            assert_eq!(app.route_layer().is_active(), multiple);
            if multiple {
                let (_, text) = super::events::render(&mut app, 130);
                assert!(
                    text.contains("api - http://localhost:9876/review/api/"),
                    "{text}"
                );
                assert!(app.service.opened_system_targets().is_empty());
            } else {
                assert_eq!(
                    app.service.opened_system_targets(),
                    ["http://localhost:9876/review/web/"]
                );
            }
            assert!(app.intent.is_none());
            assert!(app.service.operations().is_empty());
        }
    }
}

#[test]
fn routed_service_enter_and_menu_open_the_exact_service_url() {
    for menu in [false, true] {
        let mut app = root(AppService::for_tests());
        app.update_snapshot(snapshot());
        activate(&mut app, "service:review:web", "Open in browser", menu);
        assert_eq!(
            app.service.opened_system_targets(),
            ["http://localhost:9876/review/web/"]
        );
        assert!(!app.view.is_active());
        assert!(app.service.operations().is_empty());
    }
}

#[test]
fn failed_cleanup_enter_and_menu_inspect_without_retrying_deletion() {
    for menu in [false, true] {
        let mut inventory = snapshot();
        inventory.instances.clear();
        inventory
            .activities
            .push(crate::store::environments::Activity {
                id: "cleanup".into(),
                name: "review".into(),
                template: Some("website".into()),
                service: None,
                action: "delete_instance".into(),
                owner_pid: 1,
                started_at: 1,
                deadline: u64::MAX,
                error: Some("cleanup failed".into()),
                finished: true,
            });
        let mut app = root(AppService::for_tests());
        app.snapshot = inventory.clone();
        app.set_rows_for_tests(rows::from_snapshot(&inventory));
        activate(&mut app, "operation:cleanup:review", "View details", menu);
        assert!(app.view.is_active());
        assert!(app.details_open);
        assert!(app.intent.is_none());
        assert!(app.service.operations().is_empty());
    }
}

#[test]
fn enter_submits_instance_search_without_activating_the_highlighted_template() {
    let mut app = root(AppService::for_tests());
    app.update_snapshot(snapshot());
    let route = select(&mut app, "template:/tmp/templates/website");
    for character in "/website".chars() {
        app.dispatch_event(
            &route,
            &TuiEvent::Key(Key::Char(character).into()),
            &mut EventCtx::default(),
        );
    }
    assert!(instances::is_searching(&app.instances));
    app.dispatch_event(
        &route,
        &TuiEvent::Key(Key::Enter.into()),
        &mut EventCtx::default(),
    );
    assert!(!instances::is_searching(&app.instances));
    assert!(!app.view.is_active());
    assert!(app.intent.is_none());
}

#[test]
fn enter_leaves_routeless_instances_services_and_groups_unchanged() {
    let mut inventory = snapshot();
    inventory.instances[0].services[0].url = None;
    inventory.instances[0]
        .repositories
        .push(crate::store::environments::RepositoryCheckout {
            target: "app".into(),
            path: "/tmp/workspaces/review/app".into(),
            cloned: true,
        });
    let ids: Vec<_> = rows::from_snapshot(&inventory)
        .iter()
        .filter(|row| !row.is_template())
        .map(|row| row.id.clone())
        .collect();
    let mut app = root(AppService::for_tests());
    app.update_snapshot(inventory);
    for id in ids {
        let route = select(&mut app, &id);
        let (_, before) = super::events::render(&mut app, 130);
        app.dispatch_event(
            &route,
            &TuiEvent::Key(Key::Enter.into()),
            &mut EventCtx::default(),
        );
        let (_, after) = super::events::render(&mut app, 130);
        assert_eq!(before, after, "{id}");
        assert!(!app.view.is_active(), "{id}");
        assert!(!app.route_layer().is_active(), "{id}");
        assert!(app.service.opened_system_targets().is_empty(), "{id}");
    }
}
