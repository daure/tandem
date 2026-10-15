use std::sync::Arc;

use super::AppService;
use crate::{
    environments::InstanceScope,
    store::memories::{Memory, MemoryInput, MemoryMatches, MemorySearch, MemorySummary},
};

impl AppService {
    pub(crate) async fn remember(
        &self,
        scope: Arc<InstanceScope>,
        input: MemoryInput,
    ) -> Result<MemorySummary, String> {
        let input = input.normalized()?;
        let service = self.clone();
        tokio::task::spawn_blocking(move || {
            let (instance, _lock) = service.environments.admit_instance_conclusion(&scope)?;
            let acceptance = service
                .rules
                .store
                .instance_acceptance(&instance)
                .map_err(|error| error.to_string())?;
            service
                .rules
                .store
                .remember(
                    &instance.name,
                    &scope.identity(&service.environments.config)?,
                    acceptance.map(|acceptance| acceptance.id),
                    input,
                )
                .map_err(|error| error.to_string())
        })
        .await
        .map_err(|error| format!("memory write worker failed: {error}"))?
    }

    pub(crate) async fn get_memory(&self, id: i64) -> Result<Memory, String> {
        let rules = self.rules.clone();
        tokio::task::spawn_blocking(move || {
            rules.store.memory(id).map_err(|error| error.to_string())
        })
        .await
        .map_err(|error| format!("memory read worker failed: {error}"))?
    }

    pub(crate) async fn search_memories(
        &self,
        input: MemorySearch,
    ) -> Result<MemoryMatches, String> {
        input.validate()?;
        let rules = self.rules.clone();
        tokio::task::spawn_blocking(move || {
            rules
                .store
                .search_memories(input)
                .map_err(|error| error.to_string())
        })
        .await
        .map_err(|error| format!("memory search worker failed: {error}"))?
    }
}
