use super::*;
use crate::{
    app::{Intent, instances, opencode},
    store::{
        environments::RepositoryCheckout,
        opencode::{Activity, Client, Pane, Session, Snapshot},
    },
};

fn tree(running: bool) -> Vec<rows::Row> {
    let mut inventory = snapshot();
    inventory.instances[0].services[0].status = if running { "up" } else { "exited 0" }.into();
    inventory.instances[0]
        .repositories
        .push(RepositoryCheckout {
            target: "app".into(),
            path: "/tmp/workspaces/review/app".into(),
            cloned: true,
        });
    inventory.instances[0].services.push(InstanceService {
        name: "migrate".into(),
        status: "exited 0".into(),
        one_shot: true,
        ..Default::default()
    });
    let pane = Pane {
        session: "main".into(),
        id: 7,
        tab_id: 4,
        tab_name: "review".into(),
    };
    let observation = Snapshot {
        sessions: vec![Session {
            id: "ses_review".into(),
            directory: "/tmp/workspaces/review/app".into(),
            activity: Activity::Idle,
            panes: vec![
                pane.clone(),
                Pane {
                    id: 8,
                    ..pane.clone()
                },
            ],
            ..Default::default()
        }],
        clients: vec![Client {
            title: "OpenCode".into(),
            directory: "/tmp/workspaces/review".into(),
            server: String::new(),
            pane: Pane { id: 9, ..pane },
            stale: false,
            awaiting_presence_since: None,
        }],
        ..Default::default()
    };
    let mut rows = rows::from_snapshot(&inventory);
    opencode::append_rows(&mut rows, &observation, true);
    rows
}

#[test]
fn nested_lifecycle_hotkeys_confirm_the_instance_or_selected_service() {
    init_ui();
    for running in [false, true] {
        let rows = tree(running);
        for row in rows.iter().filter(|row| row.parent.is_some()) {
            for (index, key) in [(6, 'p'), (3, 's'), (7, 'r')] {
                let mut app = root(AppService::for_tests());
                app.set_rows_for_tests(rows.clone());
                instances::set_highlighted(&app.instances, Some(row.id.clone()));
                let mut ctx = EventCtx::new(AnimationSettings::default());
                app.event(&TuiEvent::Key(KeyEvent::from(Key::Char(key))), &mut ctx);

                let service = row.service_name.as_deref();
                let expected = match (index, service) {
                    (6, _) => matches!(&app.intent, Some(Intent::Purge(name)) if name == "review"),
                    (_, Some("migrate")) => app.intent.is_none(),
                    (3, Some(service)) => matches!(
                        &app.intent,
                        Some(Intent::ServiceState { name, service: target, running: start })
                            if name == "review" && target == service && *start != running
                    ),
                    (3, None) if running => {
                        matches!(&app.intent, Some(Intent::Stop(name)) if name == "review")
                    }
                    (3, None) => matches!(
                        &app.intent,
                        Some(Intent::Resume { name, template })
                            if name == "review" && template == "website"
                    ),
                    (7, service) => matches!(
                        &app.intent,
                        Some(Intent::Restart { name, service: target })
                            if name == "review" && target.as_deref() == service
                    ),
                    _ => unreachable!(),
                };
                assert!(expected, "row={}, key={key}, running={running}", row.id);
                assert_eq!(
                    app.view.is_active(),
                    index == 6 || service != Some("migrate")
                );
                assert_eq!(app.selected().unwrap().id, row.id);
                assert!(app.service.operations().is_empty());
            }
        }
    }
}

#[test]
fn nested_lifecycle_actions_use_configured_keys_and_instance_capabilities() {
    init_ui();
    for (index, key) in [(6, 'q'), (3, 'z'), (7, 'w')] {
        for enabled in [false, true] {
            let mut rows = tree(true);
            let instance = rows.iter_mut().find(|row| row.instance.is_some()).unwrap();
            instance.can_start = false;
            instance.can_stop = enabled;
            instance.can_restart = enabled;
            let mut app = root(AppService::for_tests());
            app.set_rows_for_tests(rows);
            app.keys[index] = tuicore::KeySpec::plain(key);
            instances::set_highlighted(&app.instances, Some("services:review".into()));
            app.event(
                &TuiEvent::Key(KeyEvent::from(Key::Char(key))),
                &mut EventCtx::new(AnimationSettings::default()),
            );
            assert_eq!(app.view.is_active(), enabled || index == 6);
            assert!(app.service.operations().is_empty());
        }
    }
}

#[test]
fn external_workspace_rows_cannot_target_instance_lifecycle_actions() {
    init_ui();
    let mut rows = rows::from_snapshot(&snapshot());
    opencode::append_rows(
        &mut rows,
        &Snapshot {
            sessions: vec![Session {
                id: "ses_external".into(),
                directory: "/work/external".into(),
                activity: Activity::Idle,
                ..Default::default()
            }],
            ..Default::default()
        },
        true,
    );
    let external = rows.iter().filter(|row| row.opencode.is_some());
    assert!(external.clone().count() >= 3);
    for row in external {
        for key in ['p', 's', 'r'] {
            let mut app = root(AppService::for_tests());
            app.set_rows_for_tests(rows.clone());
            instances::set_highlighted(&app.instances, Some(row.id.clone()));
            app.event(
                &TuiEvent::Key(KeyEvent::from(Key::Char(key))),
                &mut EventCtx::new(AnimationSettings::default()),
            );
            assert!(app.intent.is_none(), "row={}, key={key}", row.id);
            assert!(!app.view.is_active());
            assert!(app.service.operations().is_empty());
        }
    }
}
