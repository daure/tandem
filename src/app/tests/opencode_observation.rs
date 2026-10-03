use super::*;
use crate::store::opencode::{Activity, Pane, Session, Snapshot};

fn missing_workspace(attached: bool) -> Snapshot {
    let mut snapshot = Snapshot {
        sessions: vec![Session {
            id: "ses_work".into(),
            title: "Unfinished work".into(),
            directory: "/work/missing".into(),
            server: "http://127.0.0.1:4199".into(),
            activity: Activity::Unknown,
            stale: true,
            last_question: Some("Finish the task".into()),
            panes: if attached {
                vec![Pane {
                    session: "main".into(),
                    id: 7,
                    tab_id: 4,
                    tab_name: "work".into(),
                }]
            } else {
                Vec::new()
            },
            ..Default::default()
        }],
        ..Default::default()
    };
    snapshot
        .observation
        .missing_directories
        .insert("/work/missing".into());
    snapshot
        .observation
        .unfinished_sessions
        .insert("ses_work".into());
    if attached {
        snapshot
            .observation
            .verified_panes
            .insert(("main".into(), 7));
    }
    snapshot
}

#[test]
fn unfinished_missing_workspaces_show_warning_glyphs_in_both_trees_without_history() {
    init_ui();
    for attached in [true, false] {
        let service = AppService::for_tests();
        service.set_opencode_snapshot_for_tests(missing_workspace(attached));
        let observation = service.opencode_snapshot();
        for sessions in [false, true] {
            let rows = super::super::opencode::project_rows(
                &EnvironmentSnapshot::default(),
                &[],
                &observation,
                true,
                false,
                sessions,
            );
            let row = rows
                .iter()
                .find(|row| row.id.ends_with(":ses_work"))
                .unwrap();
            assert_eq!(row.icon, "󰚩");
            assert_eq!(row.secondary_icon, "");
            assert_eq!(row.tone, rows::Tone::Warning);
            assert_eq!(row.secondary_tone, rows::Tone::Warning);
            assert!(row.workspace_missing);
            assert!(!row.secondary_loading);
            assert!(
                row.details
                    .iter()
                    .any(|property| property.name == "Workspace"
                        && property.value == "Folder missing")
            );
        }
    }
}

#[test]
fn missing_detached_sessions_explain_unavailable_open_new_and_close_hotkeys() {
    init_ui();
    let mut app = root(AppService::for_tests());
    app.service
        .set_opencode_snapshot_for_tests(missing_workspace(false));
    app.update_snapshot(EnvironmentSnapshot::default());
    let id = "opencode:external:/work/missing:ses_work";
    super::super::instances::set_highlighted(&app.instances, Some(id.into()));
    let row = app.selected().unwrap();
    for action in ["open", "new", "close"] {
        let mut ctx = EventCtx::new(AnimationSettings::default());
        let handled = match action {
            "open" => app.activate_opencode(&row, &mut ctx),
            "new" => app.create_opencode_session(&row, &mut ctx),
            _ => app.close_opencode(&row, &mut ctx),
        };
        assert!(handled);
        assert_eq!(ctx.notifications().len(), 1);
        assert!(app.opencode_action.is_none());
    }
    assert!(
        app.service
            .open_opencode("ses_work", None)
            .unwrap_err()
            .contains("folder is missing")
    );
    assert!(
        app.service
            .new_opencode_session("/work/missing", None)
            .unwrap_err()
            .contains("directory is unavailable")
    );
}
