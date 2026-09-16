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
        }],
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
    let request = environment.begin_resource_sample().unwrap();
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
                        Instant::now() + Duration::from_secs(60)
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
    assert!(environment.begin_resource_sample().is_none());

    let instances = environment.snapshot().instances;
    let request = environment
        .resources
        .lock()
        .unwrap()
        .begin(&instances, Instant::now() + Duration::from_secs(60))
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
fn failed_readings_release_the_warmup_guard_and_report_errors() {
    for fail_on in [1, 2] {
        let (_home, environment) = environment();
        let request = environment.begin_resource_sample().unwrap();
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
        assert!(environment.begin_resource_sample().is_none());
    }
}

#[test]
fn completed_samples_wait_one_minute_before_polling_again() {
    let mut cache = ResourceCache::default();
    let instances = instances();
    let now = Instant::now();
    let request = cache.begin(&instances, now).unwrap();
    assert!(request.warm_up);
    cache.finish(request, Ok(samples(stats(1, 100, 1000))));
    assert!(
        cache
            .begin(&instances, now + Duration::from_secs(59))
            .is_none()
    );
    assert!(
        !cache
            .begin(&instances, now + Duration::from_secs(60))
            .unwrap()
            .warm_up
    );
}

#[test]
fn one_shot_memory_is_immediate_and_cpu_uses_successive_samples() {
    let mut cache = ResourceCache::default();
    let mut instances = instances();
    let now = Instant::now();
    let request = cache.begin(&instances, now).unwrap();
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

    let request = cache.begin(&instances, now + SAMPLE_INTERVAL).unwrap();
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
    let request = environment.begin_resource_sample().unwrap();
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
            assert!(environment.begin_resource_sample().is_none());
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
            .begin(&instances, now + SAMPLE_INTERVAL * index as u32)
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
    let request = cache.begin(&instances, Instant::now()).unwrap();
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
    assert!(cache.begin(&[], now).is_none());
    let request = cache.begin(&instances, now).unwrap();
    assert!(cache.begin(&instances, now + SAMPLE_INTERVAL).is_none());
    cache.finish(request, Ok(samples(stats(1, 100, 1000))));
    assert!(
        cache
            .begin(&instances, now + Duration::from_secs(1))
            .is_none()
    );
    let request = cache.begin(&instances, now + SAMPLE_INTERVAL).unwrap();
    assert_eq!(
        cache.finish(request, Err("timeout".into())).as_deref(),
        Some("timeout")
    );
    cache.apply(&mut instances);
    assert_eq!(
        instances[0].services[0].usage.unwrap().memory_bytes,
        80 * 1048576
    );
    let request = cache.begin(&instances, now + SAMPLE_INTERVAL * 2).unwrap();
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
    let request = cache.begin(&instances, now).unwrap();
    cache.finish(request, Ok(samples(stats(1, 100, 1000))));
    instances[0].services[0].status = "down (exit 0)".into();
    cache.apply(&mut instances);
    assert!(instances[0].services[0].usage.is_none());
    instances[0].services[0].status = "healthy".into();
    instances[0].services[0].started_at = Some("second-start".into());
    cache.apply(&mut instances);
    assert!(instances[0].services[0].usage.is_none());
    let request = cache.begin(&instances, now + SAMPLE_INTERVAL).unwrap();
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
        environment.refresh();
        let request = environment
            .begin_resource_sample()
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
        assert!(environment.begin_resource_sample().is_none());
    }
}
