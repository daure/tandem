use tokio::sync::oneshot::{Receiver, error::TryRecvError};
use tuicore::Notification;

use super::App;

pub(super) struct Creation {
    pub prompt: String,
    pub start_instance: bool,
    pub launches: Vec<(String, Receiver<Result<(), String>>)>,
}

impl Default for Creation {
    fn default() -> Self {
        Self {
            prompt: String::new(),
            start_instance: true,
            launches: Vec::new(),
        }
    }
}

impl Creation {
    pub fn opencode(&self) -> Option<Option<String>> {
        (!self.prompt.trim().is_empty()).then(|| Some(self.prompt.clone()))
    }

    pub fn reset_form(&mut self) {
        self.prompt.clear();
        self.start_instance = true;
    }
}

impl App {
    pub(super) fn poll_creation_launches(&mut self) -> bool {
        let mut errors = Vec::new();
        self.creation.launches.retain_mut(|(name, reply)| {
            let result = match reply.try_recv() {
                Ok(result) => result,
                Err(TryRecvError::Empty) => return true,
                Err(TryRecvError::Closed) => Err("OpenCode launch worker stopped".into()),
            };
            if let Err(error) = result {
                errors.push(format!("{name}: {error}"));
            }
            false
        });
        let changed = !errors.is_empty();
        for error in errors {
            self.notify(Notification::error("Cannot open OpenCode", error));
        }
        changed
    }
}
