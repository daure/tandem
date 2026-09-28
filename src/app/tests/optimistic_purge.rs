use super::*;
use crate::app::{instances, operations::Deletion};
use crate::store::{
    environments::OperationState,
    opencode::{Pane, Session, Snapshot},
};

#[test]
fn optimistic_purge_preserves_selection_and_restores_failures_in_both_tabs() {
    init_ui();
    for sessions_tab in [false, true] {
        let mut inventory = snapshot();
        for name in ["alpha", "zulu"] {
            let mut instance = inventory.instances[0].clone();
            instance.name = name.into();
            instance.workspace = format!("/tmp/workspaces/{name}");
            inventory.instances.push(instance);
        }
        let observation = Snapshot {
            sessions: inventory
                .instances
                .iter()
                .enumerate()
                .map(|(id, instance)| Session {
                    id: format!("ses_{}", instance.name),
                    directory: instance.workspace.clone(),
                    panes: vec![Pane {
                        session: "main".into(),
                        id: id as u32,
                        tab_id: 1,
                        tab_name: instance.name.clone(),
                    }],
                    ..Default::default()
                })
                .collect(),
            ..Default::default()
        };
        let mut app = root(AppService::for_tests());
        app.service.set_opencode_snapshot_for_tests(observation);
        app.handle_message(
            Msg::SetAttachedSessionsOnly(sessions_tab),
            &mut EventCtx::new(AnimationSettings::default()),
        );
        app.update_snapshot(inventory.clone());
        instances::select_instance(&app.instances, "review");
        let area = Rect::new(0, 0, 130, 40);
        app.layout(area, &mut tuicore::LayoutCtx::new());
        assert_eq!(app.selected().unwrap().id, "instance:review");
        let mut operation = super::operations::operation("delete_instance", "review");
        app.deletions
            .push(Deletion::new(operation.clone()).unwrap());
        for _ in 0..2 {
            let mut pending = inventory.clone();
            assert!(app.deletions[0].project(
                &mut pending,
                |_| Ok(operation.clone()),
                &mut Vec::new()
            ));
            app.update_snapshot(pending);
            app.layout(area, &mut tuicore::LayoutCtx::new());
            assert_eq!(app.selected().unwrap().id, "instance:zulu");
            assert!(!app.project_rows(&app.snapshot, &[]).iter().any(|row| {
                row.instance.as_deref() == Some("review")
                    || row.workspace.as_deref() == Some("/tmp/workspaces/review")
            }));
        }
        operation.state = OperationState::Failed;
        operation.error = Some("closure failed".into());
        let mut restored = inventory.clone();
        let mut notices = Vec::new();
        assert!(!app.deletions[0].project(&mut restored, |_| Ok(operation), &mut notices));
        app.deletions.clear();
        app.update_snapshot(restored);
        app.layout(area, &mut tuicore::LayoutCtx::new());
        assert_eq!(app.selected().unwrap().id, "instance:zulu");
        assert!(
            app.project_rows(&app.snapshot, &[])
                .iter()
                .any(|row| row.instance.as_deref() == Some("review"))
        );
        assert_eq!(notices.len(), 1);
        assert!(notices[0].body().contains("closure failed"));
    }
}
