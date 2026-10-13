use super::*;

#[test]
fn live_bursts_cross_queue_and_replay_limits_without_blocking_fast_readers() {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            let (_home, config, store) = fixture();
            let listener = Listener::bind(&config).unwrap();
            let path = socket_path(&config).unwrap();
            let server = tokio::spawn(serve(store.clone(), listener));
            let slow = UnixStream::connect(&path).await.unwrap();
            let mut fast = BufReader::new(UnixStream::connect(&path).await.unwrap());
            let initial = next(&mut fast).await;
            let mut snapshot = state_stream::StateSnapshot::empty();
            let description = "x".repeat(12 * 1024);
            snapshot.instances["instances"] = json!(
                (0..256)
                    .map(|index| {
                        json!({"name":format!("instance-{index:04}"),"description":description})
                    })
                    .collect::<Vec<_>>()
            );
            let publisher = store.clone();
            tokio::task::spawn_blocking(move || publisher.publish(snapshot))
                .await
                .unwrap()
                .unwrap();
            let started = std::time::Instant::now();
            let mut cursor = sequence(initial["cursor"].as_str().unwrap());
            for index in 0..256 {
                let event = next(&mut fast).await;
                assert_eq!(event["type"], "instance.changed");
                assert_eq!(event["data"]["name"], format!("instance-{index:04}"));
                assert_eq!(event["data"]["description"], description);
                let next_cursor = sequence(event["cursor"].as_str().unwrap());
                assert_eq!(next_cursor, cursor + 1);
                cursor = next_cursor;
            }
            eprintln!(
                "256 live events / 3 MiB delivered in {:?}",
                started.elapsed()
            );
            let mut resumed = UnixStream::connect(&path).await.unwrap();
            resumed
                .write_all(format!("{}\n", json!({"since":initial["cursor"]})).as_bytes())
                .await
                .unwrap();
            let reset = next(&mut BufReader::new(resumed)).await;
            assert_eq!(reset["type"], "snapshot");
            assert_eq!(reset["reset"], true);
            assert_eq!(
                reset["data"]["instances"]["instances"]
                    .as_array()
                    .unwrap()
                    .len(),
                256
            );
            drop(slow);
            server.abort();
            let _ = server.await;
        });
}

#[test]
fn disconnected_clients_hold_their_slot_until_pending_database_reads_finish() {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .max_blocking_threads(1)
        .build()
        .unwrap()
        .block_on(async {
            let (_home, _config, store) = fixture();
            let (started, running) = std::sync::mpsc::channel();
            let (release, blocked) = std::sync::mpsc::channel();
            let blocker = tokio::task::spawn_blocking(move || {
                started.send(()).unwrap();
                blocked.recv().unwrap();
            });
            running.recv_timeout(Duration::from_secs(1)).unwrap();
            let semaphore = Arc::new(Semaphore::new(1));
            let permit = Arc::new(semaphore.clone().acquire_owned().await.unwrap());
            let pending = Arc::downgrade(&permit);
            let (mut peer, stream) = UnixStream::pair().unwrap();
            peer.write_all(b"{\"since\":null}\n").await.unwrap();
            let (_updates, receiver) = watch::channel(String::new());
            let client = tokio::spawn(delivery::client(stream, store, receiver, permit));
            let queued = tokio::time::timeout(Duration::from_secs(1), async {
                while pending.strong_count() < 2 {
                    tokio::task::yield_now().await;
                }
            })
            .await;
            drop(peer);
            let stopped = tokio::time::timeout(Duration::from_secs(1), client).await;
            let retained = semaphore.available_permits();
            release.send(()).unwrap();
            blocker.await.unwrap();
            queued.unwrap();
            let _ = stopped.unwrap().unwrap();
            assert_eq!(retained, 0);
            let _permit = tokio::time::timeout(Duration::from_secs(1), semaphore.acquire_owned())
                .await
                .unwrap()
                .unwrap();
        });
}
