use std::{
    io::{BufRead, BufReader, Write},
    os::unix::net::UnixListener,
    time::Duration,
};

use super::*;

fn with_server(replies: Vec<(&'static str, &'static str)>, check: impl FnOnce(&str)) {
    let root = tempfile::tempdir().unwrap();
    let socket = root.path().join("docker.sock");
    let listener = UnixListener::bind(&socket).unwrap();
    listener.set_nonblocking(true).unwrap();
    let server = std::thread::spawn(move || {
        let deadline = Instant::now() + Duration::from_secs(5);
        for (expected, reply) in replies {
            let (mut stream, _) = loop {
                match listener.accept() {
                    Ok(connection) => break connection,
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        assert!(Instant::now() < deadline, "missing request: {expected}");
                        std::thread::sleep(Duration::from_millis(1));
                    }
                    Err(error) => panic!("{error}"),
                }
            };
            stream
                .set_read_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut request = String::new();
            reader.read_line(&mut request).unwrap();
            assert_eq!(request.trim(), expected);
            loop {
                let mut header = String::new();
                assert!(reader.read_line(&mut header).unwrap() > 0);
                if header == "\r\n" {
                    break;
                }
            }
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{reply}",
                reply.len()
            )
            .unwrap();
        }
    });
    check(socket.to_str().unwrap());
    server.join().unwrap();
}

#[test]
fn one_shot_requests_negotiate_the_api_over_the_selected_socket() {
    with_server(
        vec![
            ("GET /version HTTP/1.1", r#"{"ApiVersion":"1.54"}"#),
            (
                "GET /v1.54/containers/abc123/stats?stream=false&one-shot=true HTTP/1.1",
                r#"{"memory_stats":{"usage":1234}}"#,
            ),
        ],
        |socket| {
            let deadline = Instant::now() + Duration::from_secs(3);
            let client = StatsClient::from_socket(socket, deadline).unwrap();
            let result: serde_json::Value = client.sample("abc123", deadline).unwrap();
            assert_eq!(result["memory_stats"]["usage"], 1234);
            assert!(
                client
                    .sample::<serde_json::Value>("../version", deadline)
                    .is_err()
            );
            assert!(
                client
                    .sample::<serde_json::Value>("abc123", Instant::now())
                    .is_err()
            );
        },
    );
}

#[test]
fn unsupported_api_versions_and_malformed_responses_are_reported() {
    with_server(
        vec![("GET /version HTTP/1.1", r#"{"ApiVersion":"1.40"}"#)],
        |socket| {
            assert!(
                StatsClient::from_socket(socket, Instant::now() + Duration::from_secs(3)).is_err()
            );
        },
    );
    with_server(
        vec![
            ("GET /version HTTP/1.1", r#"{"ApiVersion":"1.54"}"#),
            (
                "GET /v1.54/containers/abc123/stats?stream=false&one-shot=true HTTP/1.1",
                "invalid",
            ),
        ],
        |socket| {
            let deadline = Instant::now() + Duration::from_secs(3);
            let client = StatsClient::from_socket(socket, deadline).unwrap();
            assert!(
                client
                    .sample::<serde_json::Value>("abc123", deadline)
                    .is_err()
            );
        },
    );
}
