use super::*;
use std::time::Duration;

fn stat(pid: u32, started: u64, ticks: u64, pages: u64) -> String {
    format!(
        "{pid} (client ) with spaces) S 1 2 3 4 5 6 7 8 9 10 {ticks} 0 0 0 0 0 1 0 {started} 999999 {pages}"
    )
}

#[test]
fn process_samples_measure_rss_and_multicore_cpu_between_observations() {
    let now = Instant::now();
    let (first, usage) = sample(&stat(42, 9, 100, 20), 42, 4096, 100, now, None)
        .unwrap()
        .unwrap();
    assert_eq!(usage.memory_bytes, Some(81_920));
    assert_eq!(usage.cpu_basis_points, None);
    assert!(usage.cpu_waiting);
    let previous = ProcessResource {
        sample: Some(first),
        usage,
        ..Default::default()
    };
    let (_, next) = sample(
        &stat(42, 9, 400, 30),
        42,
        4096,
        100,
        now + Duration::from_secs(2),
        Some(&previous),
    )
    .unwrap()
    .unwrap();
    assert_eq!(next.memory_bytes, Some(122_880));
    assert_eq!(next.cpu_basis_points, Some(15_000));
    assert!(!next.cpu_waiting);
    let (_, reused) = sample(
        &stat(42, 10, 500, 30),
        42,
        4096,
        100,
        now + Duration::from_secs(2),
        Some(&previous),
    )
    .unwrap()
    .unwrap();
    assert_eq!(reused.cpu_basis_points, None);
    let (_, reset) = sample(
        &stat(42, 9, 99, 30),
        42,
        4096,
        100,
        now + Duration::from_secs(2),
        Some(&previous),
    )
    .unwrap()
    .unwrap();
    assert_eq!(reset.cpu_basis_points, None);
}

#[test]
fn invalid_and_exited_processes_do_not_report_usage() {
    let now = Instant::now();
    for text in [
        "42 (short) S",
        "bad",
        &stat(43, 9, 100, 20),
        &stat(42, 9, 100, u64::MAX),
    ] {
        assert!(sample(text, 42, 4096, 100, now, None).is_err());
    }
    assert!(
        sample(
            &stat(42, 9, 100, 20).replace(") S", ") Z"),
            42,
            4096,
            100,
            now,
            None
        )
        .unwrap()
        .is_none()
    );
    assert!(sample(&stat(42, 9, 100, 20), 42, -1, 100, now, None).is_err());
    let root = tempfile::tempdir().unwrap();
    assert_eq!(
        read_process(root.path(), 42, 4096, 100, None)
            .unwrap_err()
            .kind(),
        io::ErrorKind::NotFound
    );
}

#[test]
fn collection_reads_each_pid_once_and_tracks_the_latest_route() {
    let presence = |id: &str, observed_at| {
        serde_json::from_value::<Presence>(serde_json::json!({
            "pid": std::process::id(), "observed_at": observed_at, "id": id,
            "title": "Session", "directory": "/work", "server": "http://localhost:1",
            "activity": "idle", "zellij_session": "main", "pane_id": 7
        }))
        .unwrap()
    };
    let processes = collect(
        &[presence("ses_old", 1), presence("ses_current", 2)],
        vec![],
    );
    assert_eq!(processes.len(), 1);
    assert_eq!(processes[0].session_id, "ses_current");
    assert!(processes[0].usage.memory_bytes.unwrap() > 0);
    assert!(processes[0].error.is_none());
    assert!(collect(&[], processes).is_empty());
}

#[test]
#[ignore = "Development-only process sampling cost measurement"]
fn process_sampling_cost() {
    let pid = std::process::id();
    let ticks = unsafe { libc::sysconf(libc::_SC_CLK_TCK) };
    let page_size = unsafe { libc::sysconf(libc::_SC_PAGESIZE) };
    let start = Instant::now();
    for _ in 0..10_000 {
        std::hint::black_box(
            read_process(Path::new("/proc"), pid, page_size, ticks, None)
                .unwrap()
                .unwrap(),
        );
    }
    eprintln!(
        "10,000 client /proc samples: {:?}; average: {:?}",
        start.elapsed(),
        start.elapsed() / 10_000
    );
}
