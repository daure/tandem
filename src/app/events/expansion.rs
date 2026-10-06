use std::time::{Duration, Instant};

use super::{Events, FocusState, tree::Entry};
use crate::store::opencode::SessionLocation;

pub(super) struct CreatedSession {
    acceptance: i64,
    location: SessionLocation,
    deadline: Instant,
}

pub(in crate::app) fn expand_opencode_session(
    state: &FocusState,
    acceptance: i64,
    location: SessionLocation,
) {
    state.borrow_mut().created_session = Some(CreatedSession {
        acceptance,
        location,
        deadline: Instant::now() + Duration::from_secs(30),
    });
}

impl Events {
    pub(super) fn expand_created_session(&mut self) -> bool {
        let parent = {
            let mut requested = self.requested.borrow_mut();
            let Some(created) = &requested.created_session else {
                return false;
            };
            if Instant::now() >= created.deadline {
                requested.created_session = None;
                return false;
            }
            self.projected.iter().rev().find_map(|entry| match entry {
                Entry::Conversation { target, row }
                    if target.acceptance.id == created.acceptance
                        && row
                            .opencode
                            .as_ref()
                            .is_some_and(|target| target.matches_location(&created.location)) =>
                {
                    row.parent.clone()
                }
                _ => None,
            })
        };
        let Some(parent) = parent else {
            return false;
        };
        let data = self.view.second_mut();
        let mut ancestor = Some(parent);
        while let Some(parent) = ancestor {
            data.expand(&parent);
            ancestor = self
                .projected
                .iter()
                .find(|row| row.id() == parent)
                .and_then(Entry::parent);
        }
        self.requested.borrow_mut().created_session = None;
        true
    }
}
