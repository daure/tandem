use super::*;
use crate::{
    app::{instances, opencode as projection},
    environments::opencode::Observer,
    store::opencode::Snapshot,
};
use serde_json::json;
use std::{
    fs,
    os::unix::fs::PermissionsExt,
    time::{SystemTime, UNIX_EPOCH},
};

fn roots(rows: &[rows::Row]) -> Vec<&str> {
    rows.iter()
        .filter(|row| row.parent.is_none())
        .map(|row| row.id.as_str())
        .collect()
}

#[test]
fn workspace_groups_follow_zellij_positions_and_keep_children_and_selection_on_reorder() {
    init_ui();
    let root = tempfile::tempdir().unwrap();
    let observer = Observer {
        presence: root.path().join("presence"),
        daemons: root.path().join("daemons"),
        zellij: root.path().join("zellij"),
        excluded: Default::default(),
    };
    fs::create_dir(&observer.presence).unwrap();
    fs::write(
        &observer.zellij,
        r#"#!/bin/sh
root=$(dirname "$0")
case "$*" in
  list-sessions*) echo main ;;
  '--session main action list-panes --all --json') cat "$root/panes.json" ;;
  '--session main action list-tabs --json') cat "$root/tabs.json" ;;
  *) exit 1 ;;
esac
"#,
    )
    .unwrap();
    fs::set_permissions(&observer.zellij, fs::Permissions::from_mode(0o700)).unwrap();
    fs::write(
        root.path().join("panes.json"),
        json!([
            {"id":10,"is_plugin":false,"exited":false,"tab_id":3,"tab_name":"chezmoi"},
            {"id":20,"is_plugin":false,"exited":false,"tab_id":90,"tab_name":"tandem"},
            {"id":30,"is_plugin":false,"exited":false,"tab_id":8,"tab_name":"later"},
            {"id":40,"is_plugin":false,"exited":false,"tab_id":90,"tab_name":"tandem"}
        ])
        .to_string(),
    )
    .unwrap();
    for (id, pane, directory) in [
        ("ses_chezmoi", 10, "/work/chezmoi"),
        ("ses_tandem", 20, "/work/tandem"),
        ("ses_tandem_later", 30, "/work/tandem"),
        ("", 40, "/work/z-new"),
    ] {
        fs::write(observer.presence.join(format!("{pane}.json")), json!({
            "pid":std::process::id(),
            "observed_at":SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_millis() as u64,
            "id":id,"title":id,"directory":directory,"server":"","activity":"idle",
            "zellij_session":"main","pane_id":pane
        }).to_string()).unwrap();
    }
    let tabs = root.path().join("tabs.json");
    fs::write(
        &tabs,
        json!([
            {"tab_id":3,"position":1},
            {"tab_id":8,"position":2},
            {"tab_id":90,"position":0}
        ])
        .to_string(),
    )
    .unwrap();
    tokio::runtime::Runtime::new().unwrap().block_on(async {
        let snapshot = observer
            .observe_changes(&[], Snapshot::default(), false)
            .await
            .unwrap();
        assert_eq!(snapshot.error, None);
        let initial = projection::attached_rows(Vec::new(), &snapshot);
        assert_eq!(
            roots(&initial),
            [
                "opencode-workspace:/work/tandem",
                "opencode-workspace:/work/z-new",
                "opencode-workspace:/work/chezmoi",
            ]
        );
        let state = instances::state(initial.clone());
        instances::set_attached_sessions_only(&state, true);
        let mut tree = instances::Instances::new(state.clone());
        let selected = "opencode:external:/work/tandem:ses_tandem";
        tree.highlight_for_tests(selected);

        fs::write(
            &tabs,
            json!([
                {"tab_id":90,"position":2},
                {"tab_id":8,"position":1},
                {"tab_id":3,"position":0}
            ])
            .to_string(),
        )
        .unwrap();
        let snapshot = observer
            .observe_changes(&[], snapshot, false)
            .await
            .unwrap();
        let reordered = projection::attached_rows(Vec::new(), &snapshot);
        assert_eq!(
            roots(&reordered),
            [
                "opencode-workspace:/work/chezmoi",
                "opencode-workspace:/work/tandem",
                "opencode-workspace:/work/z-new",
            ]
        );
        for workspace in roots(&initial) {
            let children = |rows: &[rows::Row]| {
                rows.iter()
                    .filter(|row| row.parent.as_deref() == Some(workspace))
                    .map(|row| row.id.clone())
                    .collect::<Vec<_>>()
            };
            assert_eq!(children(&initial), children(&reordered));
            let parent = reordered
                .iter()
                .position(|row| row.id == workspace)
                .unwrap();
            assert!(
                reordered[parent + 1..]
                    .iter()
                    .take(children(&reordered).len())
                    .all(|row| row.parent.as_deref() == Some(workspace))
            );
        }
        instances::replace_rows(&state, reordered.clone());
        tree.layout(Rect::new(0, 0, 120, 40), &mut tuicore::LayoutCtx::new());
        assert_eq!(instances::selected(&state).unwrap().id, selected);

        fs::write(&tabs, "invalid inventory").unwrap();
        let retained = observer
            .observe_changes(&[], snapshot, false)
            .await
            .unwrap();
        assert_eq!(retained.error, None);
        assert_eq!(
            roots(&projection::attached_rows(Vec::new(), &retained)),
            roots(&reordered)
        );
    });
}
