use schemars::JsonSchema;
use serde::Serialize;

use super::{ContainerState, Instance, InstanceService};

#[derive(Clone, Debug, Default, Serialize, JsonSchema, PartialEq, Eq)]
pub(crate) struct UsageSummary {
    pub memory_bytes: Option<u64>,
    pub cpu_basis_points: Option<u64>,
    pub memory_limit_bytes: Option<u64>,
    pub memory_partial: bool,
    pub cpu_partial: bool,
    pub memory_stale: bool,
    pub cpu_stale: bool,
    pub age_seconds: Option<u64>,
    pub cpu_age_seconds: Option<u64>,
    pub paused: bool,
}

impl UsageSummary {
    pub fn service(service: &InstanceService) -> Self {
        let mut result = Self::total(std::iter::once(service), false);
        result.memory_partial = false;
        result.cpu_partial = false;
        result
    }

    pub fn instance(instance: &Instance) -> Self {
        if instance.suppress_resources() {
            return Self::default();
        }
        Self::total(instance.services.iter(), false)
    }

    pub fn instances<'a>(instances: impl Iterator<Item = &'a Instance>) -> Self {
        let instances: Vec<_> = instances.collect();
        let partial = instances
            .iter()
            .any(|instance| instance.suppress_resources());
        Self::total(
            instances
                .into_iter()
                .filter(|instance| !instance.suppress_resources())
                .flat_map(|instance| &instance.services),
            partial,
        )
    }

    fn total<'a>(services: impl Iterator<Item = &'a InstanceService>, excluded: bool) -> Self {
        let mut result = Self {
            memory_partial: excluded,
            cpu_partial: excluded,
            ..Self::default()
        };
        let mut memory = 0_u64;
        let mut cpu = 0_u64;
        let mut limit = 0_u64;
        let (mut memory_count, mut cpu_count) = (0, 0);
        let (mut memory_missing, mut cpu_missing, mut uncapped) = (false, false, false);
        for service in services
            .filter(|service| service.consumes_resources() && !service.runtime.resources_suppressed)
        {
            memory_count += 1;
            result.memory_stale |= service.runtime.resources_stale || service.runtime.stale;
            result.age_seconds = result.age_seconds.max(service.runtime.resource_age_seconds);
            if let Some(usage) = service.usage {
                if let Some(total) = memory.checked_add(usage.memory_bytes) {
                    memory = total;
                } else {
                    memory_missing = true;
                }
            } else {
                memory_missing = true;
            }
            match service
                .memory_limit_bytes
                .filter(|limit| *limit > 0)
                .and_then(|cap| limit.checked_add(cap))
            {
                Some(total) => limit = total,
                None => uncapped = true,
            }
            if service.state() == ContainerState::Running {
                cpu_count += 1;
                result.cpu_stale |= service.runtime.resources_stale || service.runtime.stale;
                if service
                    .usage
                    .is_some_and(|usage| usage.cpu_basis_points.is_some())
                {
                    result.cpu_age_seconds = result
                        .cpu_age_seconds
                        .max(service.runtime.resource_age_seconds);
                }
                match service
                    .usage
                    .and_then(|usage| usage.cpu_basis_points)
                    .and_then(|value| cpu.checked_add(value))
                {
                    Some(total) => cpu = total,
                    None => cpu_missing = true,
                }
            }
        }
        result.memory_bytes = (memory_count > 0 && !memory_missing).then_some(memory);
        result.cpu_basis_points = (cpu_count > 0 && !cpu_missing).then_some(cpu);
        result.memory_partial |= memory_missing;
        result.cpu_partial |= cpu_missing;
        result.memory_limit_bytes =
            (memory_count > 0 && !uncapped && !result.memory_partial && !result.memory_stale)
                .then_some(limit);
        result.paused = memory_count > 0 && cpu_count == 0;
        result
    }
}
