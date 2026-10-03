use std::collections::BTreeSet;

use crate::store::{events::Record, providers::Snapshot};

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(in crate::app) struct StreamKey {
    pub provider: String,
    pub stream: String,
}

impl StreamKey {
    pub(in crate::app) fn new(provider: String, stream: String) -> Self {
        Self { provider, stream }
    }

    pub(super) fn from_record(record: &Record) -> Self {
        Self::new(record.provider.clone(), record.event.stream.clone())
    }

    pub(super) fn label(&self) -> String {
        format!(
            "{} · {}",
            super::clean(&self.provider),
            super::clean(&self.stream)
        )
    }

    pub(super) fn matches(&self, record: &Record) -> bool {
        self.provider == record.provider && self.stream == record.event.stream
    }
}

pub(super) fn inventory(
    providers: &Snapshot,
    records: &[Record],
    selected: &[StreamKey],
    pinned: Option<&Record>,
) -> Vec<StreamKey> {
    let mut sources: BTreeSet<_> = selected.iter().cloned().collect();
    for provider in &providers.providers {
        if let Some(manifest) = &provider.manifest {
            for name in manifest
                .streams
                .iter()
                .chain(provider.streams.iter().map(|stream| &stream.name))
            {
                sources.insert(StreamKey::new(manifest.name.clone(), name.clone()));
            }
        }
    }
    sources.extend(records.iter().chain(pinned).map(StreamKey::from_record));
    sources.into_iter().collect()
}
