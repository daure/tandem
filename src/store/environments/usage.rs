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
    pub memory_waiting: bool,
    pub cpu_waiting: bool,
    pub age_seconds: Option<u64>,
    pub paused: bool,
}

impl UsageSummary {
    pub fn service(service: &InstanceService) -> Self {
        Self::total(std::iter::once(service), false)
    }

    pub fn instance(instance: &Instance) -> Self {
        if instance.suppress_resources() {
            return Self {
                memory_waiting: true,
                cpu_waiting: true,
                ..Self::default()
            };
        }
        Self::total(instance.services.iter(), false)
    }

    pub fn instances<'a>(instances: impl Iterator<Item = &'a Instance>) -> Self {
        let instances: Vec<_> = instances.collect();
        let waiting = instances
            .iter()
            .any(|instance| instance.suppress_resources());
        Self::total(
            instances
                .into_iter()
                .filter(|instance| !instance.suppress_resources())
                .flat_map(|instance| &instance.services),
            waiting,
        )
    }

    fn total<'a>(services: impl Iterator<Item = &'a InstanceService>, waiting: bool) -> Self {
        let mut result = Self {
            memory_waiting: waiting,
            cpu_waiting: waiting,
            ..Self::default()
        };
        let mut memory = 0_u64;
        let mut cpu = 0_u64;
        let mut limit = 0_u64;
        let (mut memory_count, mut cpu_count) = (0, 0);
        let (mut memory_samples, mut cpu_samples) = (0, 0);
        let (mut memory_missing, mut cpu_missing, mut uncapped) = (false, false, false);
        let mut collection_error = false;
        for service in services
            .filter(|service| service.consumes_resources() && !service.runtime.resources_suppressed)
        {
            memory_count += 1;
            let failed = service.runtime.resource_error.is_some() || service.runtime.stale;
            collection_error |= failed;
            result.age_seconds = result.age_seconds.max(service.runtime.resource_age_seconds);
            if failed {
                memory_missing = true;
            } else if let Some(usage) = service.usage {
                if let Some(total) = memory.checked_add(usage.memory_bytes) {
                    memory = total;
                    memory_samples += 1;
                } else {
                    memory_missing = true;
                }
            } else {
                result.memory_waiting = true;
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
                if failed {
                    cpu_missing = true;
                } else if let Some(value) = service.usage.and_then(|usage| usage.cpu_basis_points) {
                    if let Some(total) = cpu.checked_add(value) {
                        cpu = total;
                        cpu_samples += 1;
                    } else {
                        cpu_missing = true;
                    }
                } else {
                    result.cpu_waiting = true;
                }
            }
        }
        result.memory_bytes = (memory_samples > 0).then_some(memory);
        result.cpu_basis_points = (cpu_samples > 0).then_some(cpu);
        result.memory_partial = memory_missing && memory_samples > 0;
        result.cpu_partial = cpu_missing && cpu_samples > 0;
        result.memory_limit_bytes = (memory_count > 0
            && !uncapped
            && !result.memory_partial
            && !result.memory_waiting
            && !collection_error)
            .then_some(limit);
        result.paused = memory_count > 0 && cpu_count == 0;
        result
    }
}
