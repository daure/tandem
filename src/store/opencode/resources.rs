use std::{
    collections::{BTreeMap, BTreeSet},
    time::Instant,
};

use super::{Snapshot, workspace_owner};
use crate::store::environments::UsageSummary;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ProcessSample {
    pub started_ticks: u64,
    pub cpu_ticks: u64,
    pub observed_at: Instant,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct ProcessResource {
    pub pid: u32,
    pub session_id: String,
    pub directory: String,
    pub zellij_session: String,
    pub pane_id: Option<u32>,
    pub sample: Option<ProcessSample>,
    pub usage: UsageSummary,
    pub error: Option<String>,
}

impl ProcessResource {
    pub fn mark_stale(&mut self, error: String) {
        self.error = Some(error);
        self.usage.memory_partial = true;
        self.usage.cpu_partial = true;
        self.usage.memory_waiting = false;
        self.usage.cpu_waiting = false;
        self.usage.age_seconds = self
            .sample
            .as_ref()
            .map(|sample| sample.observed_at.elapsed().as_secs());
    }
}

#[derive(Clone, Debug)]
pub(crate) struct Owner {
    pub name: String,
    pub workspace: String,
    pub template_directory: String,
}

#[derive(Default)]
pub(crate) struct Totals {
    pub overall: UsageSummary,
    pub external: UsageSummary,
    pub instances: BTreeMap<String, UsageSummary>,
    pub templates: BTreeMap<String, UsageSummary>,
    pub workspaces: BTreeMap<String, UsageSummary>,
    pub sessions: BTreeMap<String, UsageSummary>,
    pub panes: BTreeMap<(String, u32), UsageSummary>,
}

impl Totals {
    pub fn new(snapshot: &Snapshot, owners: &[Owner]) -> Self {
        let mut totals = Self::default();
        let mut seen = BTreeSet::new();
        for process in &snapshot.resources {
            if !seen.insert(process.pid) {
                continue;
            }
            let usage = &process.usage;
            totals.overall.merge(usage);
            totals
                .workspaces
                .entry(process.directory.clone())
                .or_default()
                .merge(usage);
            if !process.session_id.is_empty() {
                totals
                    .sessions
                    .entry(process.session_id.clone())
                    .or_default()
                    .merge(usage);
            }
            if let Some(pane) = process.pane_id {
                totals
                    .panes
                    .entry((process.zellij_session.clone(), pane))
                    .or_default()
                    .merge(usage);
            }
            if let Some(name) = workspace_owner(
                &process.directory,
                owners
                    .iter()
                    .map(|owner| (owner.name.as_str(), owner.workspace.as_str())),
            ) {
                totals
                    .instances
                    .entry(name.to_owned())
                    .or_default()
                    .merge(usage);
                let owner = owners
                    .iter()
                    .find(|owner| owner.name == name)
                    .expect("matched owner");
                totals
                    .templates
                    .entry(owner.template_directory.clone())
                    .or_default()
                    .merge(usage);
            } else {
                totals.external.merge(usage);
            }
        }
        totals
    }
}
