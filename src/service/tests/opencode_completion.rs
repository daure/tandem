use std::{
    future::Future,
    pin::Pin,
    sync::{Arc, Mutex, mpsc},
    task::{Context, Wake, Waker},
    time::Duration,
};

use crate::{service::AppService, store::opencode::CloseScope};

struct SubmitOnWake {
    service: AppService,
    result: Mutex<Option<mpsc::Sender<Result<super::CloseOpencodeOutcome, String>>>>,
}

impl Wake for SubmitOnWake {
    fn wake(self: Arc<Self>) {
        self.wake_by_ref();
    }

    fn wake_by_ref(self: &Arc<Self>) {
        if let Some(sender) = self.result.lock().unwrap().take() {
            let _ = sender.send(
                self.service
                    .close_opencode_scope(&CloseScope::ExternalWorkspaces),
            );
        }
    }
}

#[test]
fn action_replies_allow_followup_actions_during_the_receivers_wakeup() {
    let service = AppService::for_tests();
    for expected in [Ok(()), Err("navigation failed".to_owned())] {
        let (finish, finished) = tokio::sync::oneshot::channel();
        let result = expected.clone();
        let mut reply = service
            .opencode
            .spawn_navigation(&service.runtime, "busy", async move {
                finished.await.unwrap();
                result
            })
            .unwrap();
        assert_eq!(
            service
                .close_opencode_scope(&CloseScope::ExternalWorkspaces)
                .unwrap_err(),
            "OpenCode action is already in progress"
        );
        let (sender, followup) = mpsc::channel();
        let waker = Waker::from(Arc::new(SubmitOnWake {
            service: service.clone(),
            result: Mutex::new(Some(sender)),
        }));
        assert!(
            Pin::new(&mut reply)
                .poll(&mut Context::from_waker(&waker))
                .is_pending()
        );
        finish.send(()).unwrap();
        let followup = followup
            .recv_timeout(Duration::from_secs(5))
            .unwrap()
            .unwrap();
        assert_eq!(service.runtime.block_on(reply).unwrap(), expected);
        service.runtime.block_on(followup.reply).unwrap().unwrap();
    }
}

#[test]
fn reset_cancels_pending_actions_and_preserves_the_next_actions_admission() {
    let service = AppService::for_tests();
    let (entered, started) = mpsc::channel();
    let (finish, finished) = mpsc::channel();
    let reply = service
        .opencode
        .spawn_navigation(&service.runtime, "busy", async move {
            entered.send(()).unwrap();
            finished.recv_timeout(Duration::from_secs(5)).unwrap();
            Ok(())
        })
        .unwrap();
    started.recv_timeout(Duration::from_secs(5)).unwrap();
    service.opencode.reset();
    let (finish_next, next_finished) = tokio::sync::oneshot::channel();
    let next = service
        .opencode
        .spawn_navigation(&service.runtime, "busy", async move {
            next_finished.await.unwrap();
            Ok(())
        })
        .unwrap();
    finish.send(()).unwrap();
    assert!(service.runtime.block_on(reply).is_err());
    assert_eq!(
        service
            .close_opencode_scope(&CloseScope::ExternalWorkspaces)
            .unwrap_err(),
        "OpenCode action is already in progress"
    );
    finish_next.send(()).unwrap();
    service.runtime.block_on(next).unwrap().unwrap();
    let followup = service
        .close_opencode_scope(&CloseScope::ExternalWorkspaces)
        .unwrap();
    service.runtime.block_on(followup.reply).unwrap().unwrap();
}
