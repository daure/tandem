use super::*;
use crate::environments::command::Progress;
use std::{sync::mpsc, time::Instant};

#[test]
fn simultaneous_instance_start_locks_share_a_template_and_block_template_mutations() {
    let (_home, config) = fixture();
    let template = templates::create(&config, "website").unwrap();
    let first_instance = gateway::lock(&config, "instance-first").unwrap();
    let first_template = gateway::shared_lock(&config, "template-website").unwrap();
    let second_instance = gateway::lock(&config, "instance-second").unwrap();
    let second_template = gateway::shared_lock(&config, "template-website").unwrap();
    assert!(gateway::lock(&config, "instance-first").is_err());
    assert!(gateway::lock(&config, "template-website").is_err());
    assert!(templates::update_manifest(&config, "website", template.manifest.clone()).is_err());
    drop((first_instance, first_template));
    assert!(gateway::lock(&config, "template-website").is_err());
    drop((second_instance, second_template));
    assert!(templates::update_manifest(&config, "website", template.manifest).is_ok());
    let exclusive = gateway::lock(&config, "template-website").unwrap();
    assert!(gateway::shared_lock(&config, "template-website").is_err());
    drop(exclusive);
}

#[test]
fn gateway_contention_waits_for_the_lock_within_the_startup_deadline() {
    let (_home, config) = fixture();
    let held = gateway::lock(&config, "gateway").unwrap();
    let (waiting, receiver) = mpsc::channel();
    let progress: Progress = Arc::new(move |line| waiting.send(line).unwrap());
    std::thread::scope(|scope| {
        let worker = scope.spawn(|| {
            gateway::lock_until(
                &config,
                "gateway",
                Instant::now() + Duration::from_secs(5),
                &progress,
            )
        });
        assert_eq!(
            receiver.recv_timeout(Duration::from_secs(2)).unwrap(),
            "Waiting for gateway lock"
        );
        drop(held);
        let acquired = worker.join().unwrap().unwrap();
        assert!(gateway::lock(&config, "gateway").is_err());
        drop(acquired);
    });
    let _held = gateway::lock(&config, "gateway").unwrap();
    let progress: Progress = Arc::new(|_| {});
    assert_eq!(
        gateway::lock_until(&config, "gateway", Instant::now(), &progress).unwrap_err(),
        "timed out waiting for gateway lock"
    );
}
