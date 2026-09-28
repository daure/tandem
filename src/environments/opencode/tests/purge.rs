use super::*;
use std::time::{Duration, Instant};

#[test]
fn purge_closes_workspace_panes_concurrently_and_preserves_shared_tab_panes() {
    let root = tempfile::tempdir().unwrap();
    let observer = observer(root.path());
    for (file, id, pane, directory) in [
        ("root.json", "ses_one", 7, "/work/review"),
        ("duplicate.json", "ses_one", 7, "/work/review"),
        ("empty.json", "", 8, "/work/review/repo"),
        ("other.json", "ses_other", 9, "/work/review-other"),
        ("outside.json", "ses_outside", 100, "/outside"),
    ] {
        presence_in(&observer, file, id, pane, "", directory);
    }
    fs::write(&observer.zellij, r#"#!/bin/sh
root=$(dirname "$0")
printf '%s\n' "$*" >> "$root/calls"
case "$*" in
  list-sessions*) printf 'main\n' ;;
  *list-panes*)
    printf '['
    separator=''
    for id in 7 8 9 100; do
      if [ ! -f "$root/closed-$id" ]; then
        printf '%s{"id":%s,"is_plugin":false,"exited":false,"tab_id":4,"tab_name":"Review"}' "$separator" "$id"
        separator=','
      fi
    done
    printf ']'
    ;;
  *close-pane*)
    case "$*" in
      *terminal_7) touch "$root/closed-7" ;;
      *terminal_8) touch "$root/closed-8" ;;
      *) exit 1 ;;
    esac
    attempts=0
    while [ ! -f "$root/closed-7" ] || [ ! -f "$root/closed-8" ]; do
      attempts=$((attempts + 1))
      [ "$attempts" -lt 50 ] || exit 1
      sleep 0.02
    done ;;
esac
"#).unwrap();
    tokio::runtime::Runtime::new()
        .unwrap()
        .block_on(observer.close_workspace("/work/review", Instant::now() + Duration::from_secs(3)))
        .unwrap();
    let calls = fs::read_to_string(root.path().join("calls")).unwrap();
    let mut closures = calls
        .lines()
        .filter(|line| line.contains("close-pane"))
        .collect::<Vec<_>>();
    closures.sort();
    assert_eq!(
        closures,
        [
            "--session main action close-pane --pane-id terminal_7",
            "--session main action close-pane --pane-id terminal_8",
        ]
    );
    assert!(root.path().join("closed-7").exists());
    assert!(root.path().join("closed-8").exists());
    assert!(!root.path().join("closed-9").exists());
    assert!(!calls.contains("close-tab"));
}

#[test]
fn closing_the_last_pane_accepts_an_empty_zellij_session_inventory() {
    let root = tempfile::tempdir().unwrap();
    let observer = observer(root.path());
    presence(&observer, "client.json", "ses_one", 7, "");
    fs::write(
        root.path().join("panes.json"),
        json!([{"id":7,"is_plugin":false,"exited":false,"tab_id":4,"tab_name":"Review"}])
            .to_string(),
    )
    .unwrap();
    fs::write(
        &observer.zellij,
        r#"#!/bin/sh
root=$(dirname "$0")
case "$*" in
  list-sessions*)
    if [ -f "$root/closed" ]; then
      printf 'No active zellij sessions found.\n' >&2
      exit 1
    fi
    printf 'main\n' ;;
  *list-panes*) cat "$root/panes.json" ;;
  *close-pane*) touch "$root/closed" ;;
esac
"#,
    )
    .unwrap();
    tokio::runtime::Runtime::new()
        .unwrap()
        .block_on(observer.close_workspace("/work/review", Instant::now() + Duration::from_secs(3)))
        .unwrap();
    assert!(root.path().join("closed").exists());
    fs::write(
        &observer.zellij,
        "#!/bin/sh\nprintf 'permission denied\\n' >&2\nexit 1\n",
    )
    .unwrap();
    let error = tokio::runtime::Runtime::new()
        .unwrap()
        .block_on(observer.close_workspace("/work/review", Instant::now() + Duration::from_secs(3)))
        .unwrap_err();
    assert!(error.contains("permission denied"), "{error}");
}

#[test]
fn purge_requires_verified_closure_even_after_client_receipts_disappear() {
    let root = tempfile::tempdir().unwrap();
    let observer = observer(root.path());
    presence(&observer, "client.json", "ses_one", 7, "");
    fs::write(
        &observer.zellij,
        r#"#!/bin/sh
root=$(dirname "$0")
case "$*" in
  list-sessions*) printf 'main\n' ;;
  *list-panes*) cat "$root/panes.json" ;;
  *close-pane*) rm "$root/presence/client.json" ;;
esac
"#,
    )
    .unwrap();
    let error = tokio::runtime::Runtime::new()
        .unwrap()
        .block_on(
            observer.close_workspace("/work/review", Instant::now() + Duration::from_millis(300)),
        )
        .unwrap_err();
    assert!(
        error.contains("Timed out closing OpenCode clients"),
        "{error}"
    );
}

#[test]
fn purge_blocks_on_unclosable_clients_and_ignores_already_closed_panes() {
    let root = tempfile::tempdir().unwrap();
    let observer = observer(root.path());
    let runtime = tokio::runtime::Runtime::new().unwrap();
    presence(&observer, "client.json", "ses_one", 7, "");
    let receipt = observer.presence.join("client.json");
    let mut value: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&receipt).unwrap()).unwrap();
    value["pane_id"] = serde_json::Value::Null;
    fs::write(&receipt, value.to_string()).unwrap();
    let error = runtime
        .block_on(observer.close_workspace("/work/review", Instant::now() + Duration::from_secs(2)))
        .unwrap_err();
    assert!(error.contains("outside Zellij"), "{error}");
    assert!(!root.path().join("calls").exists());
    value["pane_id"] = 42.into();
    fs::write(&receipt, value.to_string()).unwrap();
    runtime
        .block_on(observer.close_workspace("/work/review", Instant::now() + Duration::from_secs(2)))
        .unwrap();
    assert!(
        !fs::read_to_string(root.path().join("calls"))
            .unwrap()
            .contains("close-pane")
    );
}
