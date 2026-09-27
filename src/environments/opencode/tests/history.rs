use super::{history_server::Server, *};

fn setup(root: &Path, server: &Server) -> Observer {
    let observer = observer(root);
    let station = observer.daemons.join("station");
    fs::create_dir_all(station.join("dirs")).unwrap();
    fs::write(station.join("port"), server.url.rsplit(':').next().unwrap()).unwrap();
    fs::write(station.join("dirs/work.dir"), "/work/review\n").unwrap();
    observer
}

fn clear(observer: &Observer) -> Result<(), String> {
    tokio::runtime::Runtime::new()
        .unwrap()
        .block_on(super::super::history::clear_with(observer, "/work/review"))
}

#[test]
fn cleanup_deletes_exact_directory_history_and_children_without_touching_neighbours() {
    let root = tempfile::tempdir().unwrap();
    let server = Server::start();
    let observer = setup(root.path(), &server);
    server.session("ses_old", "/work/review", None);
    server.session("ses_child", "/work/review", Some("ses_old"));
    server.session("ses_neighbour", "/work/review-other", None);
    server.session("ses_subfolder", "/work/review/repo", None);
    clear(&observer).unwrap();
    let data = server.data.lock().unwrap();
    assert_eq!(data.deleted, ["ses_child", "ses_old"]);
    assert_eq!(
        data.sessions.keys().cloned().collect::<Vec<_>>(),
        ["ses_neighbour", "ses_subfolder"]
    );
}

#[test]
fn cleanup_rejects_active_attached_or_unverifiable_history_before_deleting_anything() {
    for reason in [
        "busy",
        "question",
        "attached",
        "status",
        "directory",
        "child",
        "cycle",
    ] {
        let root = tempfile::tempdir().unwrap();
        let server = Server::start();
        let observer = setup(root.path(), &server);
        server.session("ses_old", "/work/review", None);
        match reason {
            "busy" => server.data.lock().unwrap().busy = true,
            "question" => server.data.lock().unwrap().awaiting_answer = true,
            "attached" => presence(&observer, "client.json", "ses_old", 7, &server.url),
            "status" => server.data.lock().unwrap().status_failure = true,
            "directory" => {
                server.session("ses_other", "/work/review-other", None);
                server.data.lock().unwrap().unfiltered_list = true;
            }
            "child" => server.session("ses_child", "/work/review-other", Some("ses_old")),
            "cycle" => server.session("ses_old", "/work/review", Some("ses_old")),
            _ => unreachable!(),
        }
        if reason == "attached" {
            let path = observer.presence.join("client.json");
            let mut record: serde_json::Value =
                serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
            record["directory"] = json!("/work/review");
            fs::write(path, record.to_string()).unwrap();
        }
        assert!(clear(&observer).is_err(), "{reason}");
        let data = server.data.lock().unwrap();
        assert!(data.deleted.is_empty(), "{reason}");
        assert!(data.sessions.contains_key("ses_old"), "{reason}");
    }
}

#[test]
fn cleanup_reports_delete_errors_and_verifies_successful_responses() {
    for lies in [false, true] {
        let root = tempfile::tempdir().unwrap();
        let server = Server::start();
        let observer = setup(root.path(), &server);
        server.session("ses_old", "/work/review", None);
        {
            let mut data = server.data.lock().unwrap();
            data.delete_failure = !lies;
            data.delete_lies = lies;
        }
        let error = clear(&observer).unwrap_err();
        assert!(
            error.contains(if lies { "did not delete" } else { "500" }),
            "{error}"
        );
        assert!(server.data.lock().unwrap().sessions.contains_key("ses_old"));
    }
}
