use super::*;

#[test]
#[ignore = "entry point for detached test workers"]
fn worker_entry() {
    let Ok(descriptor) = std::env::var("TANDEM_TEST_CLOSE_FD") else {
        return;
    };
    let request: serde_json::Value =
        serde_json::from_str(&std::env::var("TANDEM_INSTANCE_CLOSE").unwrap()).unwrap();
    AppService::run_instance_close_worker(
        request["name"].as_str().unwrap(),
        descriptor.parse().unwrap(),
    )
    .unwrap();
}
