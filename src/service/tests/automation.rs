use super::*;
use crate::store::{
    events::{Batch, Deletion, Error},
    providers::Action,
};

fn rejected<T, E: std::fmt::Display>(result: Result<T, E>) {
    match result {
        Err(error) => assert_eq!(error.to_string(), Error::Disabled.to_string()),
        Ok(_) => panic!("disabled automation accepted a request"),
    }
}

#[test]
fn disabled_automation_blocks_commands_and_preserves_saved_events() {
    let enabled = AppService::for_tests();
    let token = enabled.register_provider_for_tests("sample");
    let batch = |id| Batch {
        events: vec![crate::environments::events::tests::event(id)],
    };
    enabled
        .runtime
        .block_on(enabled.ingest_events(token.clone(), batch("retained")))
        .unwrap();
    let config = crate::environments::config::Config::at(
        enabled.config_for_tests().home,
        "tandem-test".into(),
        9876,
    )
    .unwrap();
    let disabled = AppService::from_config(config).unwrap();
    assert!(!disabled.automation_enabled());
    disabled.runtime.block_on(async {
        rejected(disabled.list_events().await);
        rejected(disabled.list_rules().await);
        rejected(disabled.list_providers().await);
        rejected(disabled.authenticate_provider(token.clone()).await);
        rejected(disabled.ingest_events(token, batch("blocked")).await);
        rejected(disabled.retained_event(1).await.unwrap());
        rejected(disabled.event_diagnostics(1).await.unwrap());
        rejected(disabled.replay_event(1).await.unwrap());
        rejected(disabled.delete_events(Deletion::All).await.unwrap());
        rejected(disabled.retry_acceptance(1, true).await);
        let provider = disabled.provider_action("sample".into(), Action::Start, true);
        rejected(provider.await.unwrap());
        let stream =
            disabled.provider_stream_action("sample".into(), "main".into(), Action::Stop, true);
        rejected(stream.await.unwrap());
        let deletion = disabled.delete_provider("sample".into(), true);
        rejected(deletion.await.unwrap());
        rejected(disabled.search_event_reports(vec!["sample".into()]).await);
        rejected(disabled.get_event_report(1).await);
        let conclusions = disabled.search_conclusions(vec!["sample".into()]).await;
        assert!(conclusions.unwrap().is_empty());
    });
    rejected(disabled.setup_developer_providers());
    rejected(disabled.setup_developer_rules("openai/test", false));
    disabled.poll_events();
    disabled.poll_rules();
    disabled.poll_providers();
    let retained = enabled.runtime.block_on(enabled.list_events()).unwrap();
    assert_eq!(retained.total, 1);
    assert_eq!(retained.records[0].event.event_id, "retained");
    assert!(disabled.event_snapshot().records.is_empty());
    assert!(disabled.rule_snapshot().rules.is_empty());
    assert!(disabled.provider_snapshot().providers.is_empty());
    assert!(disabled.operations().is_empty());

    let home = disabled.config_for_tests().home;
    std::fs::write(home.join("config.json"), r#"{"ui":{"automation":true}}"#).unwrap();
    assert!(!disabled.automation_enabled());
    let config = crate::environments::config::Config::at(home, "tandem-test".into(), 9876).unwrap();
    let restarted = AppService::from_config(config).unwrap();
    assert!(restarted.automation_enabled());
    let restored = restarted.runtime.block_on(restarted.list_events()).unwrap();
    assert_eq!(restored, retained);
}
