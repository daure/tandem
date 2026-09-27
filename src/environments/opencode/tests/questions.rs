use super::*;

fn session<'a>(snapshot: &'a Snapshot, id: &str) -> &'a Session {
    snapshot
        .sessions
        .iter()
        .find(|session| session.id == id)
        .unwrap()
}

#[test]
fn pending_questions_pause_busy_sessions_until_answered_or_dismissed() {
    let root = tempfile::tempdir().unwrap();
    let server = Server::start();
    let observer = observer(root.path());
    presence(&observer, "one.json", "ses_busy", 7, &server.url);
    presence_in(
        &observer,
        "two.json",
        "ses_background",
        8,
        &server.url,
        "/work/review",
    );
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let roots = ["/work/review".into()];
    let busy = runtime
        .block_on(observer.observe(&roots, Snapshot::default()))
        .unwrap();
    server
        .pending_questions
        .lock()
        .unwrap()
        .insert("/work/review/repo".into(), vec!["ses_busy".into()]);
    let waiting = runtime
        .block_on(observer.observe(&roots, busy.clone()))
        .unwrap();
    let paused = session(&waiting, "ses_busy");
    assert_eq!(paused.label(), "attached · awaiting answer");
    assert!(paused.activity_started_at_milliseconds.is_none());
    assert!(waiting.completed_since(&busy));
    assert_eq!(session(&waiting, "ses_background").activity, Activity::Busy);
    let repeated = runtime
        .block_on(observer.observe(&roots, waiting.clone()))
        .unwrap();
    assert!(!repeated.completed_since(&waiting));
    assert_eq!(
        session(&repeated, "ses_busy").activity_elapsed_milliseconds,
        paused.activity_elapsed_milliseconds
    );

    server.pending_questions.lock().unwrap().clear();
    let resumed = runtime
        .block_on(observer.observe(&roots, repeated.clone()))
        .unwrap();
    assert_eq!(session(&resumed, "ses_busy").activity, Activity::Busy);
    assert!(!resumed.completed_since(&repeated));
    server.busy.store(false, Ordering::Relaxed);
    let finished = runtime
        .block_on(observer.observe(&roots, resumed.clone()))
        .unwrap();
    assert!(finished.completed_since(&resumed));
}

#[test]
fn failed_question_observations_do_not_create_completion_transitions() {
    let root = tempfile::tempdir().unwrap();
    let server = Server::start();
    let observer = observer(root.path());
    presence(&observer, "one.json", "ses_busy", 7, &server.url);
    server
        .pending_questions
        .lock()
        .unwrap()
        .insert("/work/review/repo".into(), vec!["ses_busy".into()]);
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let roots = ["/work/review".into()];
    let waiting = runtime
        .block_on(observer.observe(&roots, Snapshot::default()))
        .unwrap();
    server
        .failed_question_directories
        .lock()
        .unwrap()
        .insert("/work/review/repo".into());
    let failed = runtime
        .block_on(observer.observe(&roots, waiting.clone()))
        .unwrap();
    assert_eq!(session(&failed, "ses_busy").activity, Activity::Unknown);
    assert!(session(&failed, "ses_busy").stale);
    assert!(failed.error.as_deref().unwrap().contains("503"));
    assert!(!failed.completed_since(&waiting));
    server.failed_question_directories.lock().unwrap().clear();
    let recovered = runtime
        .block_on(observer.observe(&roots, failed.clone()))
        .unwrap();
    assert_eq!(
        session(&recovered, "ses_busy").label(),
        "attached · awaiting answer"
    );
    assert!(!recovered.completed_since(&failed));
}
