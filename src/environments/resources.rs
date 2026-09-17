use std::{
    collections::{BTreeMap, BTreeSet},
    time::{Duration, Instant},
};

use serde::Deserialize;

use super::{Environments, stats::StatsClient};
use crate::store::environments::{ContainerState, Instance, InstanceService, ResourceUsage};

const SAMPLE_INTERVAL: Duration = Duration::from_secs(60);
const WARMUP_INTERVAL: Duration = Duration::from_secs(1);
const SAMPLE_TIMEOUT: Duration = Duration::from_secs(10);

type SampleResults = BTreeMap<String, Result<Stats, String>>;

#[derive(Clone)]
pub(crate) struct ResourceRequest {
    containers: BTreeMap<String, Option<String>>,
    warm_up: bool,
    paused: BTreeSet<String>,
}

#[derive(Default)]
pub(super) struct ResourceCache {
    last_attempt: Option<Instant>,
    last_containers: BTreeMap<String, Option<String>>,
    in_flight: bool,
    samples: BTreeMap<String, CachedSample>,
    errors: BTreeMap<String, String>,
    last_paused: BTreeSet<String>,
}

struct CachedSample {
    started_at: Option<String>,
    stats: Stats,
    usage: ResourceUsage,
    paused: bool,
}

impl ResourceCache {
    fn begin(
        &mut self,
        instances: &[Instance],
        now: Instant,
        force: bool,
    ) -> Option<ResourceRequest> {
        if self.in_flight {
            return None;
        }
        let containers: BTreeMap<_, _> = instances
            .iter()
            .flat_map(|instance| &instance.services)
            .filter(|service| service.consumes_resources() && !service.container_id.is_empty())
            .map(|service| (service.container_id.clone(), service.started_at.clone()))
            .collect();
        let paused: BTreeSet<_> = instances
            .iter()
            .flat_map(|instance| &instance.services)
            .filter(|service| service.state() == ContainerState::Paused)
            .map(|service| service.container_id.clone())
            .collect();
        self.samples
            .retain(|id, sample| containers.get(id) == Some(&sample.started_at));
        self.last_containers
            .retain(|id, _| containers.contains_key(id));
        self.errors.retain(|id, _| {
            containers.contains_key(id) && self.last_containers.get(id) == containers.get(id)
        });
        let full_sample = force
            || self
                .last_attempt
                .is_none_or(|last| now.duration_since(last) >= SAMPLE_INTERVAL);
        let targets: BTreeMap<_, _> = containers
            .iter()
            .filter(|(id, started_at)| {
                full_sample
                    || self.last_containers.get(*id) != Some(*started_at)
                    || self.last_paused.contains(*id) != paused.contains(*id)
            })
            .map(|(id, started_at)| (id.clone(), started_at.clone()))
            .collect();
        if targets.is_empty() {
            return None;
        }
        let warm_up = force
            || targets.iter().any(|(id, started_at)| {
                self.samples.get(id).is_none_or(|sample| {
                    sample.started_at != *started_at || sample.paused != paused.contains(id)
                })
            });
        if full_sample {
            self.last_attempt = Some(now);
        }
        self.last_containers = containers;
        self.last_paused = paused.clone();
        self.in_flight = true;
        Some(ResourceRequest {
            containers: targets,
            warm_up,
            paused,
        })
    }

    pub(super) fn apply(&self, instances: &mut [Instance]) {
        for service in instances
            .iter_mut()
            .flat_map(|instance| &mut instance.services)
        {
            service.usage = self
                .samples
                .get(&service.container_id)
                .filter(|sample| {
                    service.consumes_resources() && sample.started_at == service.started_at
                })
                .map(|sample| {
                    let mut usage = sample.usage;
                    if service.state() == ContainerState::Paused
                        || sample.paused != (service.state() == ContainerState::Paused)
                    {
                        usage.cpu_basis_points = None;
                    }
                    usage
                });
            service.runtime.resource_error = self
                .errors
                .get(&service.container_id)
                .filter(|_| {
                    service.consumes_resources()
                        && self.last_containers.get(&service.container_id)
                            == Some(&service.started_at)
                })
                .cloned();
        }
    }

    fn finish(
        &mut self,
        request: ResourceRequest,
        result: Result<SampleResults, String>,
    ) -> Option<String> {
        self.in_flight = false;
        match result {
            Ok(samples) => {
                for (id, result) in samples {
                    if let Some(started_at) = request.containers.get(&id) {
                        let stats = match result {
                            Ok(stats) => stats,
                            Err(error) => {
                                self.errors.insert(id.clone(), error);
                                continue;
                            }
                        };
                        self.errors.remove(&id);
                        let paused = request.paused.contains(&id);
                        let baseline = self.samples.get(&id).filter(|sample| {
                            sample.started_at == *started_at && sample.paused == paused
                        });
                        let usage = stats.usage(baseline.map(|sample| &sample.stats));
                        self.samples.insert(
                            id,
                            CachedSample {
                                started_at: started_at.clone(),
                                stats,
                                usage,
                                paused,
                            },
                        );
                    }
                }
                (!self.errors.is_empty()).then(|| {
                    self.errors
                        .iter()
                        .map(|(id, error)| format!("{id}: {error}"))
                        .collect::<Vec<_>>()
                        .join("\n")
                })
            }
            Err(error) => {
                for id in request.containers.keys() {
                    self.errors.insert(id.clone(), error.clone());
                }
                Some(error)
            }
        }
    }
}

impl Environments {
    pub(crate) fn begin_resource_sample(&self, force: bool) -> Option<ResourceRequest> {
        let snapshot = self.snapshot();
        if snapshot.loading
            || self
                .inventory_errors
                .lock()
                .unwrap_or_else(|error| error.into_inner())[1]
                .is_some()
        {
            return None;
        }
        let ready = snapshot
            .instances
            .iter()
            .filter(|instance| {
                !instance.suppress_resources() && !snapshot.startup.contains_key(&instance.name)
            })
            .cloned()
            .collect::<Vec<_>>();
        self.resources
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .begin(&ready, Instant::now(), force)
    }

    pub(crate) fn sample_resources(&self, request: ResourceRequest) {
        self.sample_resources_with(request, sample, std::thread::sleep);
    }

    fn sample_resources_with(
        &self,
        request: ResourceRequest,
        mut collect: impl FnMut(&ResourceRequest) -> Result<SampleResults, String>,
        wait: impl FnOnce(Duration),
    ) {
        let started = Instant::now();
        let result = collect(&request);
        let warm_up = request.warm_up
            && result
                .as_ref()
                .is_ok_and(|samples| samples.values().any(Result::is_ok));
        self.publish_resources(request.clone(), result, started, warm_up);
        if warm_up {
            wait(WARMUP_INTERVAL);
            let result = collect(&request);
            self.publish_resources(request, result, started, false);
        }
    }

    fn publish_resources(
        &self,
        mut request: ResourceRequest,
        mut result: Result<SampleResults, String>,
        started: Instant,
        in_flight: bool,
    ) {
        let eligible = self
            .snapshot()
            .instances
            .into_iter()
            .filter(|instance| !instance.suppress_resources())
            .flat_map(|instance| instance.services)
            .filter(InstanceService::consumes_resources)
            .map(|service| {
                (
                    service.container_id.clone(),
                    (
                        service.started_at.clone(),
                        service.state() == ContainerState::Paused,
                    ),
                )
            })
            .collect::<BTreeMap<_, _>>();
        let mut snapshot = self
            .snapshot
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let mut cache = self
            .resources
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        request.containers.retain(|id, started| {
            snapshot.instances.iter().any(|instance| {
                eligible.get(id) == Some(&(started.clone(), request.paused.contains(id)))
                    && instance.services.iter().any(|service| {
                        service.container_id == *id
                            && service.started_at == *started
                            && service.consumes_resources()
                            && (service.state() == ContainerState::Paused)
                                == request.paused.contains(id)
                    })
            })
        });
        if let Ok(samples) = &mut result {
            samples.retain(|id, _| request.containers.contains_key(id));
        }
        snapshot.resource_sample_duration_ms = Some(started.elapsed().as_millis() as u64);
        snapshot.resource_error = cache.finish(request, result);
        cache.in_flight = in_flight;
        cache.apply(&mut snapshot.instances);
    }
}

fn sample(request: &ResourceRequest) -> Result<SampleResults, String> {
    let deadline = Instant::now() + SAMPLE_TIMEOUT;
    let client = StatsClient::connect(deadline)?;
    Ok(sample_with(request, |id| client.sample(id, deadline)))
}

fn sample_with(
    request: &ResourceRequest,
    collect: impl Fn(&str) -> Result<Stats, String> + Sync,
) -> SampleResults {
    let ids: Vec<_> = request.containers.keys().collect();
    let mut samples = BTreeMap::new();
    for batch in ids.chunks(8) {
        let results = std::thread::scope(|scope| {
            let workers: Vec<_> = batch
                .iter()
                .map(|id| {
                    let collect = &collect;
                    (
                        id,
                        scope.spawn(move || {
                            let stats = collect(id)?;
                            if stats.id != **id {
                                return Err("Docker stats container ID mismatch".into());
                            }
                            Ok(stats)
                        }),
                    )
                })
                .collect();
            workers
                .into_iter()
                .map(|(id, worker)| {
                    let result = worker
                        .join()
                        .unwrap_or_else(|_| Err("resource sampling worker failed".to_owned()));
                    ((*id).clone(), result)
                })
                .collect::<Vec<_>>()
        });
        samples.extend(results);
    }
    samples
}

#[derive(Deserialize)]
struct Stats {
    id: String,
    read: chrono::DateTime<chrono::Utc>,
    cpu_stats: CpuStats,
    memory_stats: MemoryStats,
}

#[derive(Deserialize)]
struct CpuStats {
    cpu_usage: CpuUsage,
    system_cpu_usage: u64,
    #[serde(default)]
    online_cpus: u64,
}

#[derive(Deserialize)]
struct CpuUsage {
    total_usage: u64,
    #[serde(default)]
    percpu_usage: Vec<u64>,
}

#[derive(Deserialize)]
struct MemoryStats {
    usage: u64,
    stats: BTreeMap<String, u64>,
}

impl Stats {
    fn usage(&self, previous: Option<&Self>) -> ResourceUsage {
        let memory = &self.memory_stats;
        let cache = memory
            .stats
            .get("total_inactive_file")
            .or_else(|| memory.stats.get("inactive_file"))
            .copied()
            .unwrap_or_default();
        ResourceUsage {
            cpu_basis_points: previous.and_then(|previous| self.cpu_percentage(previous)),
            memory_bytes: if cache < memory.usage {
                memory.usage - cache
            } else {
                memory.usage
            },
            sampled_at_unix_seconds: self.read.timestamp().max(0) as u64,
        }
    }

    fn cpu_percentage(&self, previous: &Self) -> Option<u64> {
        if self.read <= previous.read {
            return None;
        }
        let current = &self.cpu_stats;
        let previous = &previous.cpu_stats;
        let cpu = current
            .cpu_usage
            .total_usage
            .checked_sub(previous.cpu_usage.total_usage)?;
        let system = current
            .system_cpu_usage
            .checked_sub(previous.system_cpu_usage)?;
        let cores = if current.online_cpus > 0 {
            current.online_cpus
        } else {
            current.cpu_usage.percpu_usage.len() as u64
        };
        if system == 0 || cores == 0 {
            return None;
        }
        let scaled = u128::from(cpu)
            .checked_mul(u128::from(cores))?
            .checked_mul(10_000)?;
        u64::try_from((scaled + u128::from(system) / 2) / u128::from(system)).ok()
    }
}

#[cfg(test)]
#[path = "tests/resources.rs"]
mod tests;
