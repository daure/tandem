use super::*;
use crate::store::environments::InstanceService;

fn instances() -> Vec<Instance> {
    vec![Instance {
        name: "review".into(),
        template: "website".into(),
        template_directory: "/templates/website".into(),
        workspace: "/workspaces/review".into(),
        project: "tandem-review".into(),
        pending: false,
        services: vec![InstanceService {
            name: "web".into(),
            container_id: "abc123".into(),
            status: "healthy".into(),
            one_shot: false,
            image: None,
            health: None,
            restart_policy: None,
            restart_count: 0,
            created_at: None,
            started_at: Some("first-start".into()),
            port: None,
            url: None,
            usage: None,
            memory_limit_bytes: None,
            volumes: Vec::new(),
            ..Default::default()
        }],
        runtime: crate::store::environments::InstanceRuntime {
            topology_known: true,
            ..Default::default()
        },
        ..Default::default()
    }]
}

fn stats(second: u32, cpu: u64, system: u64) -> Stats {
    serde_json::from_value(serde_json::json!({
        "id": "abc123",
        "read": format!("2026-09-16T00:00:{second:02}Z"),
        "cpu_stats": {
            "cpu_usage": {"total_usage": cpu},
            "system_cpu_usage": system,
            "online_cpus": 4
        },
        "memory_stats": {"usage": 104857600, "stats": {"inactive_file": 20971520}}
    }))
    .unwrap()
}

fn samples(stats: Stats) -> SampleResults {
    [(stats.id.clone(), Ok(stats))].into()
}

fn environment() -> (tempfile::TempDir, Environments) {
    let home = tempfile::tempdir().unwrap();
    let config =
        crate::environments::config::Config::at(home.path().to_owned(), "tandem-test".into(), 9876)
            .unwrap();
    let environment = Environments::new(config);
    {
        let mut snapshot = environment.snapshot.lock().unwrap();
        snapshot.instances = instances();
        snapshot.loading = false;
    }
    (home, environment)
}

#[test]
fn warmup_publishes_memory_then_cpu_with_a_one_second_gap_before_minute_polling() {
    let (_home, environment) = environment();
    let request = environment.begin_resource_sample(false).unwrap();
    let mut reads = 0;
    environment.sample_resources_with(
        request,
        |_| {
            reads += 1;
            Ok(samples(stats(
                reads,
                u64::from(reads) * 100,
                u64::from(reads) * 1000,
            )))
        },
        |gap| {
            assert_eq!(gap, Duration::from_secs(1));
            let snapshot = environment.snapshot();
            let usage = snapshot.instances[0].services[0].usage.unwrap();
            assert_eq!(usage.memory_bytes, 80 * 1048576);
            assert_eq!(usage.cpu_basis_points, None);
            assert!(
                environment
                    .resources
                    .lock()
                    .unwrap()
                    .begin(
                        &snapshot.instances,
                        Instant::now() + Duration::from_secs(60),
                        false,
                    )
                    .is_none()
            );
        },
    );
    assert_eq!(reads, 2);
    assert_eq!(
        environment.snapshot().instances[0].services[0]
            .usage
            .unwrap()
            .cpu_basis_points,
        Some(4000)
    );
    assert!(environment.begin_resource_sample(false).is_none());

    let instances = environment.snapshot().instances;
    let request = environment
        .resources
        .lock()
        .unwrap()
        .begin(&instances, Instant::now() + Duration::from_secs(60), false)
        .unwrap();
    environment.sample_resources_with(
        request,
        |_| {
            reads += 1;
            Ok(samples(stats(3, 300, 3000)))
        },
        |_| panic!("established CPU baselines use a single reading"),
    );
    assert_eq!(reads, 3);
}

#[test]
fn manual_sampling_refreshes_cached_memory_and_cpu_without_waiting_or_overlapping() {
    let (_home, environment) = environment();
    let initial = environment.begin_resource_sample(false).unwrap();
    let mut reads = 0;
    environment.sample_resources_with(
        initial,
        |_| {
            reads += 1;
            Ok(samples(stats(
                reads,
                u64::from(reads) * 100,
                u64::from(reads) * 1000,
            )))
        },
        |_| {},
    );
    assert!(environment.begin_resource_sample(false).is_none());
    let request = environment.begin_resource_sample(true).unwrap();
    assert!(request.warm_up);
    assert!(environment.begin_resource_sample(true).is_none());
    environment.sample_resources_with(
        request,
        |_| {
            reads += 1;
            let mut sample = stats(
                reads,
                if reads == 3 { 500 } else { 550 },
                u64::from(reads) * 1000,
            );
            sample.memory_stats.usage = if reads == 3 { 200 } else { 220 } * 1048576;
            Ok(samples(sample))
        },
        |gap| {
            assert_eq!(gap, Duration::from_secs(1));
            assert_eq!(
                environment.snapshot().instances[0].services[0]
                    .usage
                    .unwrap()
                    .memory_bytes,
                180 * 1048576
            );
            assert!(environment.begin_resource_sample(true).is_none());
            assert!(environment.begin_resource_sample(false).is_none());
        },
    );
    assert_eq!(reads, 4);
    let usage = environment.snapshot().instances[0].services[0]
        .usage
        .unwrap();
    assert_eq!(usage.memory_bytes, 200 * 1048576);
    assert_eq!(usage.cpu_basis_points, Some(2000));
    assert!(environment.begin_resource_sample(false).is_none());
}

#[test]
fn starting_instances_hide_cached_usage_and_sample_only_after_readiness_succeeds() {
    let (_home, environment) = environment();
    let initial = environment.begin_resource_sample(false).unwrap();
    environment.sample_resources_with(initial, |_| Ok(samples(stats(1, 100, 1000))), |_| {});
    let existing = environment.snapshot().instances[0].services[0].usage;
    let operation = environment
        .begin("create_instance", "second", Some("website".into()))
        .unwrap();
    {
        let mut snapshot = environment.snapshot.lock().unwrap();
        let mut discovered = snapshot.instances[0].clone();
        discovered.name = "second".into();
        discovered.services[0].container_id = "second-id".into();
        snapshot.instances[1] = discovered;
    }
    let snapshot = environment.snapshot();
    assert_eq!(snapshot.instances[0].services[0].usage, existing);
    assert!(snapshot.instances[1].services[0].usage.is_none());
    assert!(environment.begin_resource_sample(false).is_none());
    let manual = environment.begin_resource_sample(true).unwrap();
    assert_eq!(manual.containers.keys().collect::<Vec<_>>(), ["abc123"]);
    environment.sample_resources_with(manual, |_| Ok(samples(stats(2, 200, 2000))), |_| {});
    environment
        .operations
        .lock()
        .unwrap()
        .get_mut(&operation.id)
        .unwrap()
        .operation
        .state = crate::store::environments::OperationState::Succeeded;
    let ready = environment.begin_resource_sample(false).unwrap();
    assert_eq!(ready.containers.keys().collect::<Vec<_>>(), ["second-id"]);
    let mut reads = 2;
    environment.sample_resources_with(
        ready,
        |_| {
            reads += 1;
            let mut reading = stats(reads, u64::from(reads) * 100, u64::from(reads) * 1000);
            reading.id = "second-id".into();
            Ok(samples(reading))
        },
        |_| {},
    );
    assert_eq!(
        environment.snapshot().instances[1].services[0]
            .usage
            .unwrap()
            .cpu_basis_points,
        Some(4000)
    );
}

#[test]
fn successful_startup_publishes_ready_services_before_clearing_startup_state() {
    let (_home, environment) = environment();
    let operation = environment
        .begin("create_instance", "review", Some("website".into()))
        .unwrap();
    let mut ready = environment.snapshot().instances[0].clone();
    let mut api = ready.services[0].clone();
    api.name = "api".into();
    api.container_id = "api-id".into();
    ready.services.push(api);
    let mut starting = ready.clone();
    starting.services[1].status = "boot".into();
    environment.snapshot.lock().unwrap().instances = vec![starting.clone()];
    assert!(environment.snapshot().startup.contains_key("review"));
    let old_revision = environment
        .instance_revision
        .load(std::sync::atomic::Ordering::SeqCst);
    environment.finish_operation(&operation.id, Ok(Some(ready.clone())));
    let snapshot = environment.snapshot();
    assert!(snapshot.startup.is_empty());
    assert_eq!(snapshot.instances[0].services.len(), 2);
    assert!(
        snapshot.instances[0]
            .services
            .iter()
            .all(InstanceService::ready)
    );
    let ready = snapshot.instances[0].clone();
    environment.publish_instances(Ok(vec![starting.clone()]), old_revision);
    assert_eq!(environment.snapshot().instances, vec![ready]);
    let revision = environment
        .instance_revision
        .load(std::sync::atomic::Ordering::SeqCst);
    starting.services[1].status = "unhealthy".into();
    environment.publish_instances(Ok(vec![starting.clone()]), revision);
    assert_eq!(
        environment.snapshot().instances[0].services[1].status,
        "unhealthy"
    );
}

#[test]
fn template_scan_errors_allow_resource_sampling_but_runtime_discovery_errors_block_it() {
    let (_home, environment) = environment();
    {
        let mut snapshot = environment.snapshot.lock().unwrap();
        environment.set_inventory_error(
            &mut snapshot,
            0,
            Some("Template directory unavailable".into()),
        );
    }
    let request = environment.begin_resource_sample(true).unwrap();
    environment.sample_resources_with(
        request,
        |_| Err("Stats unavailable".into()),
        |_| unreachable!(),
    );
    assert!(!environment.resources.lock().unwrap().in_flight);
    {
        let mut snapshot = environment.snapshot.lock().unwrap();
        environment.set_inventory_error(&mut snapshot, 1, Some("Docker unavailable".into()));
    }
    assert!(environment.begin_resource_sample(true).is_none());
}

#[test]
fn failed_readings_release_the_warmup_guard_and_report_errors() {
    for fail_on in [1, 2] {
        let (_home, environment) = environment();
        let request = environment.begin_resource_sample(false).unwrap();
        let mut reads = 0;
        let mut waits = 0;
        environment.sample_resources_with(
            request,
            |_| {
                reads += 1;
                if reads == fail_on {
                    Err("unavailable".into())
                } else {
                    Ok(samples(stats(1, 100, 1000)))
                }
            },
            |_| waits += 1,
        );
        assert_eq!(reads, fail_on);
        assert_eq!(waits, fail_on - 1);
        assert_eq!(
            environment.snapshot().resource_error.as_deref(),
            Some("unavailable")
        );
        assert!(!environment.resources.lock().unwrap().in_flight);
        assert!(environment.begin_resource_sample(false).is_none());
    }
}

#[test]
fn completed_samples_wait_one_minute_before_polling_again() {
    let mut cache = ResourceCache::default();
    let instances = instances();
    let now = Instant::now();
    let request = cache.begin(&instances, now, false).unwrap();
    assert!(request.warm_up);
    cache.finish(request, Ok(samples(stats(1, 100, 1000))));
    assert!(
        cache
            .begin(&instances, now + Duration::from_secs(59), false)
            .is_none()
    );
    assert!(
        !cache
            .begin(&instances, now + Duration::from_secs(60), false)
            .unwrap()
            .warm_up
    );
}

#[test]
fn new_instances_sample_only_their_containers_and_preserve_cached_totals_and_cadence() {
    let (_home, environment) = environment();
    let now = Instant::now();
    let initial = environment
        .resources
        .lock()
        .unwrap()
        .begin(&instances(), now, false)
        .unwrap();
    let mut reads = 0;
    environment.sample_resources_with(
        initial,
        |_| {
            reads += 1;
            Ok(samples(stats(
                reads,
                u64::from(reads) * 100,
                u64::from(reads) * 1000,
            )))
        },
        |_| {},
    );
    let original = environment.snapshot().instances[0].services[0]
        .usage
        .unwrap();
    {
        let mut snapshot = environment.snapshot.lock().unwrap();
        let mut instance = snapshot.instances[0].clone();
        instance.name = "new-instance".into();
        instance.services[0].container_id = "def456".into();
        instance.services[0].started_at = Some("second-start".into());
        instance.services[0].usage = None;
        snapshot.instances.push(instance);
    }
    let instances = environment.snapshot().instances;
    let request = environment
        .resources
        .lock()
        .unwrap()
        .begin(&instances, now + Duration::from_secs(30), false)
        .unwrap();
    assert!(request.warm_up);
    environment.sample_resources_with(
        request,
        |request| {
            assert_eq!(request.containers.keys().collect::<Vec<_>>(), ["def456"]);
            reads += 1;
            let mut reading = stats(reads, u64::from(reads) * 100, u64::from(reads) * 1000);
            reading.id = "def456".into();
            Ok(samples(reading))
        },
        |_| {
            let snapshot = environment.snapshot();
            assert_eq!(snapshot.instances[0].services[0].usage, Some(original));
            assert_eq!(
                snapshot.instances[1].services[0]
                    .usage
                    .unwrap()
                    .cpu_basis_points,
                None
            );
        },
    );
    assert_eq!(reads, 4);
    let instances = environment.snapshot().instances;
    assert_eq!(instances[0].services[0].usage, Some(original));
    let total =
        ResourceUsage::total(instances.iter().flat_map(|instance| &instance.services)).unwrap();
    assert_eq!(total.memory_bytes, 160 * 1048576);
    assert_eq!(total.cpu_basis_points, Some(8000));
    let mut cache = environment.resources.lock().unwrap();
    assert!(
        cache
            .begin(&instances, now + Duration::from_secs(59), false)
            .is_none()
    );
    let periodic = cache
        .begin(&instances, now + SAMPLE_INTERVAL, false)
        .unwrap();
    assert!(!periodic.warm_up);
    assert_eq!(
        periodic.containers.keys().collect::<Vec<_>>(),
        ["abc123", "def456"]
    );
}

#[test]
fn failed_new_container_samples_keep_other_usage_and_wait_for_the_regular_retry() {
    let mut cache = ResourceCache::default();
    let mut instances = instances();
    let now = Instant::now();
    let request = cache.begin(&instances, now, false).unwrap();
    cache.finish(request, Ok(samples(stats(1, 100, 1000))));
    let mut service = instances[0].services[0].clone();
    service.container_id = "new".into();
    instances[0].services.push(service);
    let request = cache
        .begin(&instances, now + Duration::from_secs(1), false)
        .unwrap();
    assert_eq!(request.containers.keys().collect::<Vec<_>>(), ["new"]);
    cache.finish(request, Err("unavailable".into()));
    cache.apply(&mut instances);
    assert_eq!(
        instances[0].services[0].usage.unwrap().memory_bytes,
        80 * 1048576
    );
    assert!(instances[0].services[1].usage.is_none());
    assert!(
        cache
            .begin(&instances, now + Duration::from_secs(2), false)
            .is_none()
    );
    assert_eq!(
        cache
            .begin(&instances, now + SAMPLE_INTERVAL, false)
            .unwrap()
            .containers
            .len(),
        2
    );
}

#[test]
fn one_shot_memory_is_immediate_and_cpu_uses_successive_samples() {
    let mut cache = ResourceCache::default();
    let mut instances = instances();
    let now = Instant::now();
    let request = cache.begin(&instances, now, false).unwrap();
    assert!(
        cache
            .finish(request, Ok(samples(stats(1, 100, 1000))))
            .is_none()
    );
    cache.apply(&mut instances);
    let usage = instances[0].services[0].usage.unwrap();
    assert_eq!(usage.memory_bytes, 80 * 1048576);
    assert_eq!(usage.cpu_basis_points, None);
    assert_eq!(
        ResourceUsage::total(instances[0].services.iter()),
        Some(usage)
    );

    let request = cache
        .begin(&instances, now + SAMPLE_INTERVAL, false)
        .unwrap();
    cache.finish(request, Ok(samples(stats(3, 725, 3000))));
    cache.apply(&mut instances);
    assert_eq!(
        instances[0].services[0].usage.unwrap().cpu_basis_points,
        Some(12500)
    );
}

#[test]
fn failed_containers_are_isolated_across_batches_and_cpu_warmup() {
    let (_home, environment) = environment();
    {
        let mut snapshot = environment.snapshot.lock().unwrap();
        let service = snapshot.instances[0].services[0].clone();
        snapshot.instances[0].services = (0..10)
            .map(|index| {
                let mut service = service.clone();
                service.container_id = format!("{index:02x}");
                service
            })
            .collect();
    }
    let request = environment.begin_resource_sample(false).unwrap();
    let mut reads = 0;
    environment.sample_resources_with(
        request,
        |request| {
            reads += 1;
            Ok(sample_with(request, |id| match id {
                "00" => Err("container disappeared".into()),
                "08" => Err("invalid Docker stats response".into()),
                _ => {
                    let mut stats = stats(reads, u64::from(reads) * 100, u64::from(reads) * 1000);
                    stats.id = id.into();
                    Ok(stats)
                }
            }))
        },
        |_| {
            let snapshot = environment.snapshot();
            assert_eq!(
                snapshot.instances[0].services[9]
                    .usage
                    .unwrap()
                    .memory_bytes,
                80 * 1048576
            );
            assert!(environment.begin_resource_sample(false).is_none());
        },
    );
    assert_eq!(reads, 2);
    let snapshot = environment.snapshot();
    let errors = snapshot.resource_error.unwrap();
    assert!(errors.contains("00: container disappeared"));
    assert!(errors.contains("08: invalid Docker stats response"));
    for (index, service) in snapshot.instances[0].services.iter().enumerate() {
        if index == 0 || index == 8 {
            assert!(service.usage.is_none());
        } else {
            assert_eq!(service.usage.unwrap().cpu_basis_points, Some(4000));
        }
    }
    assert!(!environment.resources.lock().unwrap().in_flight);
}

#[test]
fn partial_failures_preserve_valid_baselines_and_refresh_successful_containers() {
    let mut cache = ResourceCache::default();
    let mut instances = instances();
    let mut second = instances[0].services[0].clone();
    second.container_id = "def456".into();
    instances[0].services.push(second);
    let now = Instant::now();
    for (index, failing) in [false, true, false].into_iter().enumerate() {
        let request = cache
            .begin(&instances, now + SAMPLE_INTERVAL * index as u32, false)
            .unwrap();
        let reading = (index + 1) as u32;
        let result = sample_with(&request, |id| {
            if failing && id == "abc123" {
                return Err("temporarily unavailable".into());
            }
            let mut stats = stats(reading, u64::from(reading) * 100, u64::from(reading) * 1000);
            stats.id = id.into();
            Ok(stats)
        });
        assert_eq!(cache.finish(request, Ok(result)).is_some(), failing);
        cache.apply(&mut instances);
        assert!(
            instances[0]
                .services
                .iter()
                .all(|service| service.usage.is_some())
        );
        assert_eq!(
            cache.samples["def456"].stats.read,
            stats(reading, 0, 0).read
        );
    }
    assert!(
        instances[0]
            .services
            .iter()
            .all(|service| service.usage.unwrap().cpu_basis_points == Some(4000))
    );
}

#[test]
fn unstarted_containers_do_not_block_running_container_totals_or_sampling() {
    let mut instances = instances();
    for status in ["created", "restarting"] {
        let mut service = instances[0].services[0].clone();
        service.container_id = status.into();
        service.status = crate::environments::docker::service_status(
            &serde_json::json!({"Status": status}),
            false,
        );
        service.started_at = None;
        instances[0].services.push(service);
    }
    let mut cache = ResourceCache::default();
    let request = cache.begin(&instances, Instant::now(), false).unwrap();
    assert_eq!(request.containers.keys().collect::<Vec<_>>(), ["abc123"]);
    cache.finish(request, Ok(samples(stats(1, 100, 1000))));
    cache.apply(&mut instances);
    assert_eq!(
        ResourceUsage::total(instances[0].services.iter())
            .unwrap()
            .memory_bytes,
        80 * 1048576
    );
}

#[test]
fn cpu_handles_idle_reset_and_invalid_baselines() {
    let previous = stats(1, 100, 1000);
    assert_eq!(stats(3, 100, 2000).cpu_percentage(&previous), Some(0));
    assert_eq!(stats(3, 99, 2000).cpu_percentage(&previous), None);
    assert_eq!(stats(3, 200, 999).cpu_percentage(&previous), None);
    assert_eq!(stats(3, 200, 1000).cpu_percentage(&previous), None);
    assert_eq!(stats(1, 200, 2000).cpu_percentage(&previous), None);
    let mut current = stats(3, 200, 2000);
    current.cpu_stats.online_cpus = 0;
    assert_eq!(current.cpu_percentage(&previous), None);
    current.cpu_stats.cpu_usage.percpu_usage = vec![0, 0];
    assert_eq!(current.cpu_percentage(&previous), Some(2000));
}

#[test]
fn memory_matches_docker_working_set_for_both_cgroup_versions() {
    let mut current = stats(1, 100, 1000);
    current.memory_stats.stats = [("total_inactive_file".into(), 1048576)].into();
    assert_eq!(current.usage(None).memory_bytes, 99 * 1048576);
    current.memory_stats.stats.clear();
    assert_eq!(current.usage(None).memory_bytes, 100 * 1048576);
    current
        .memory_stats
        .stats
        .insert("inactive_file".into(), u64::MAX);
    assert_eq!(current.usage(None).memory_bytes, 100 * 1048576);
    assert!(serde_json::from_str::<Stats>(r#"{"id":"abc123"}"#).is_err());
}

#[test]
fn failed_readings_keep_the_last_valid_resource_usage() {
    let mut cache = ResourceCache::default();
    let mut instances = instances();
    let now = Instant::now();
    assert!(cache.begin(&[], now, false).is_none());
    let request = cache.begin(&instances, now, false).unwrap();
    assert!(
        cache
            .begin(&instances, now + SAMPLE_INTERVAL, false)
            .is_none()
    );
    cache.finish(request, Ok(samples(stats(1, 100, 1000))));
    assert!(
        cache
            .begin(&instances, now + Duration::from_secs(1), false)
            .is_none()
    );
    let request = cache
        .begin(&instances, now + SAMPLE_INTERVAL, false)
        .unwrap();
    assert_eq!(
        cache.finish(request, Err("timeout".into())).as_deref(),
        Some("timeout")
    );
    cache.apply(&mut instances);
    assert_eq!(
        instances[0].services[0].usage.unwrap().memory_bytes,
        80 * 1048576
    );
    let request = cache
        .begin(&instances, now + SAMPLE_INTERVAL * 2, false)
        .unwrap();
    cache.finish(request, Ok(samples(stats(5, 200, 2000))));
    cache.apply(&mut instances);
    assert_eq!(
        instances[0].services[0].usage.unwrap().cpu_basis_points,
        Some(4000)
    );
}

#[test]
fn cache_rejects_restarted_replaced_and_stopped_containers() {
    let mut cache = ResourceCache::default();
    let mut instances = instances();
    let now = Instant::now();
    let request = cache.begin(&instances, now, false).unwrap();
    cache.finish(request, Ok(samples(stats(1, 100, 1000))));
    instances[0].services[0].status = "down (exit 0)".into();
    cache.apply(&mut instances);
    assert!(instances[0].services[0].usage.is_none());
    instances[0].services[0].status = "healthy".into();
    instances[0].services[0].started_at = Some("second-start".into());
    cache.apply(&mut instances);
    assert!(instances[0].services[0].usage.is_none());
    let request = cache
        .begin(&instances, now + SAMPLE_INTERVAL, false)
        .unwrap();
    assert!(request.warm_up);
    cache.finish(request, Ok(samples(stats(3, 200, 2000))));
    cache.apply(&mut instances);
    assert_eq!(
        instances[0].services[0].usage.unwrap().cpu_basis_points,
        None
    );
    instances[0].services[0].container_id = "replacement".into();
    cache.apply(&mut instances);
    assert!(instances[0].services[0].usage.is_none());
}

#[test]
#[ignore = "read-only Docker benchmark; set TANDEM_BENCH_NAMESPACE"]
fn live_resource_sampling_benchmark() {
    let namespace = std::env::var("TANDEM_BENCH_NAMESPACE").expect("set TANDEM_BENCH_NAMESPACE");
    let home = tempfile::tempdir().unwrap();
    let config =
        crate::environments::config::Config::at(home.path().to_owned(), namespace, 9876).unwrap();
    for _ in 0..3 {
        let environment = Environments::new(config.clone());
        environment.refresh_templates();
        environment.refresh_instances();
        let request = environment
            .begin_resource_sample(false)
            .expect("running containers required");
        let count = request.containers.len();
        let started = Instant::now();
        environment.sample_resources(request);
        eprintln!(
            "startup memory + CPU: {count} containers, {:.3}s",
            started.elapsed().as_secs_f64()
        );
        let snapshot = environment.snapshot();
        assert!(
            snapshot.resource_error.is_none(),
            "{:?}",
            snapshot.resource_error
        );
        let usage = ResourceUsage::total(
            snapshot
                .instances
                .iter()
                .flat_map(|instance| &instance.services),
        )
        .unwrap();
        assert!(usage.memory_bytes > 0);
        assert!(usage.cpu_basis_points.is_some());
        assert!(environment.begin_resource_sample(false).is_none());
    }
}

#[test]
fn pause_and_resume_trigger_targeted_cpu_baselines_without_resetting_periodic_due_time() {
    let mut cache = ResourceCache::default();
    let mut inventory = instances();
    let now = Instant::now();
    let first = cache.begin(&inventory, now, false).unwrap();
    cache.finish(first, Ok(samples(stats(1, 100, 1000))));
    inventory[0].services[0].status = "paused".into();
    let paused = cache
        .begin(&inventory, now + Duration::from_secs(2), false)
        .unwrap();
    assert!(paused.warm_up);
    cache.finish(paused, Ok(samples(stats(2, 100, 2000))));
    cache.apply(&mut inventory);
    assert_eq!(
        inventory[0].services[0].usage.unwrap().memory_bytes,
        80 * 1048576
    );
    assert_eq!(
        inventory[0].services[0].usage.unwrap().cpu_basis_points,
        None
    );
    inventory[0].services[0].status = "healthy".into();
    let resumed = cache
        .begin(&inventory, now + Duration::from_secs(3), false)
        .unwrap();
    assert!(resumed.warm_up);
    cache.finish(resumed, Ok(samples(stats(3, 200, 3000))));
    cache.apply(&mut inventory);
    assert_eq!(
        inventory[0].services[0].usage.unwrap().cpu_basis_points,
        None
    );
    assert_eq!(cache.last_attempt, Some(now));
}

#[test]
fn late_samples_cannot_publish_into_a_restarted_run() {
    let (_home, environment) = environment();
    let request = environment.begin_resource_sample(false).unwrap();
    environment.snapshot.lock().unwrap().instances[0].services[0].started_at =
        Some("new-run".into());
    environment.sample_resources_with(request, |_| Ok(samples(stats(1, 100, 1000))), |_| {});
    assert!(
        environment.snapshot().instances[0].services[0]
            .usage
            .is_none()
    );
    assert!(environment.begin_resource_sample(false).unwrap().warm_up);
}

#[test]
fn resource_staleness_uses_focus_budget_without_changing_health() {
    let mut app = instances().remove(0);
    app.services[0].usage = Some(stats(1, 100, 1000).usage(None));
    let now = app.services[0].usage.unwrap().sampled_at_unix_seconds + 121;
    crate::environments::project_instance(&mut app, false, now, false);
    assert!(!app.services[0].runtime.resources_stale);
    crate::environments::project_instance(&mut app, false, now, true);
    assert!(app.services[0].runtime.resources_stale);
    assert_eq!(
        app.summary.status,
        crate::store::environments::Status::Healthy
    );
    app.services[0].runtime.resource_error = Some("stats timeout".into());
    crate::environments::project_instance(&mut app, false, now, false);
    assert!(app.services[0].runtime.resources_stale);
}

#[test]
fn failed_and_expired_startup_release_metric_suppression() {
    for expired in [false, true] {
        let (_home, environment) = environment();
        let operation = environment
            .begin("create_instance", "review", Some("website".into()))
            .unwrap();
        assert!(environment.begin_resource_sample(false).is_none());
        if expired {
            environment
                .operations
                .lock()
                .unwrap()
                .get_mut(&operation.id)
                .unwrap()
                .timeout_seconds = 0;
        } else {
            environment.finish_operation(&operation.id, Err("readiness failed".into()));
        }
        assert!(environment.begin_resource_sample(false).is_some());
        assert_eq!(
            environment.operation(&operation.id).unwrap().state,
            crate::store::environments::OperationState::Failed
        );
    }
}

#[test]
fn historical_failed_jobs_do_not_resurrect_after_recovery_and_deletion() {
    let (_home, environment) = environment();
    let failed = environment
        .begin("create_instance", "review", Some("website".into()))
        .unwrap();
    environment.finish_operation(&failed.id, Err("startup failed".into()));
    let recovered = environment
        .begin("create_instance", "review", Some("website".into()))
        .unwrap();
    environment.finish_operation(&recovered.id, Ok(Some(instances().remove(0))));
    let deleted = environment
        .begin("delete_instance", "review", None)
        .unwrap();
    environment.finish_operation(&deleted.id, Ok(None));
    environment.snapshot.lock().unwrap().instances.clear();
    assert!(environment.snapshot().activities.is_empty());
    let created = environment
        .begin("create_instance", "review", Some("website".into()))
        .unwrap();
    let snapshot = environment.snapshot();
    assert_eq!(snapshot.activities.len(), 1);
    assert_eq!(snapshot.activities[0].id, created.id);
    assert_eq!(
        snapshot.instances[0].summary.status,
        crate::store::environments::Status::Creating
    );
}
