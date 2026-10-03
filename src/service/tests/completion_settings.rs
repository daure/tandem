use super::AppService;
use crate::store::completion::SoundChoice;

#[test]
fn completion_settings_persist_and_invalid_values_preserve_saved_values() {
    let mut service = AppService::for_tests();
    let choice = SoundChoice {
        id: "/sounds/bell.oga".into(),
        label: "Bell".into(),
    };
    service.set_sound_choices_for_tests(vec![choice.clone()]);
    assert_eq!(service.completion_fade_seconds(), 20);
    assert_eq!(service.completion_sound_choice(), "");
    service
        .runtime
        .block_on(service.set_completion_fade_seconds("45".into()).unwrap())
        .unwrap()
        .unwrap();
    service
        .runtime
        .block_on(
            service
                .set_completion_sound_choice(choice.id.clone())
                .unwrap(),
        )
        .unwrap()
        .unwrap();
    assert_eq!(service.completion_sound_count_for_tests(), 1);
    service.play_completion_sound();
    assert_eq!(
        *service.state.completion_sounds.lock().unwrap(),
        [choice.id.clone(), choice.id.clone()]
    );
    let restarted = AppService::from_config(service.environments.config.clone()).unwrap();
    assert_eq!(restarted.completion_fade_seconds(), 45);
    assert_eq!(restarted.completion_sound_choice(), choice.id);
    restarted.play_completion_sound();
    assert_eq!(*restarted.state.completion_sounds.lock().unwrap(), [""]);
    for value in ["", "0", "3601", "-1", "abc", "999999999999999999999999"] {
        assert!(service.set_completion_fade_seconds(value.into()).is_err());
    }
    assert!(
        service
            .set_completion_sound_choice("/missing.oga".into())
            .is_err()
    );
    assert_eq!(service.completion_fade_seconds(), 45);
    assert_eq!(service.completion_sound_choice(), choice.id);
    assert_eq!(service.completion_sound_count_for_tests(), 2);
}

#[test]
fn event_acceptance_sound_persists_independently_and_falls_back_for_unavailable_choices() {
    let mut service = AppService::for_tests();
    let choice = SoundChoice {
        id: "/sounds/event.oga".into(),
        label: "Event".into(),
    };
    service.set_sound_choices_for_tests(vec![choice.clone()]);
    assert_eq!(service.event_acceptance_sound_choice(), "");
    service
        .set_event_acceptance_sound_choice(choice.id.clone())
        .unwrap()
        .blocking_recv()
        .unwrap()
        .unwrap();
    assert_eq!(service.completion_sound_choice(), "");
    service.play_event_acceptance_sound();
    assert_eq!(
        *service.state.completion_sounds.lock().unwrap(),
        [choice.id.clone(), choice.id.clone()]
    );
    assert!(
        service
            .set_event_acceptance_sound_choice("/missing.oga".into())
            .is_err()
    );
    assert_eq!(service.event_acceptance_sound_choice(), choice.id);
    let restarted = AppService::from_config(service.environments.config.clone()).unwrap();
    assert_eq!(restarted.event_acceptance_sound_choice(), choice.id);
    restarted.play_event_acceptance_sound();
    assert_eq!(*restarted.state.completion_sounds.lock().unwrap(), [""]);
    let connection =
        rusqlite::Connection::open(service.environments.config.home.join("settings.sqlite3"))
            .unwrap();
    connection.execute_batch("CREATE TRIGGER reject_event_sound BEFORE INSERT ON app_settings BEGIN SELECT RAISE(FAIL, 'read only settings'); END;").unwrap();
    assert!(
        service
            .set_event_acceptance_sound_choice(choice.id.clone())
            .unwrap()
            .blocking_recv()
            .unwrap()
            .is_err()
    );
    assert_eq!(service.event_acceptance_sound_choice(), choice.id);
}

#[test]
fn failed_completion_setting_writes_retain_the_last_persisted_values() {
    let service = AppService::for_tests();
    service
        .runtime
        .block_on(service.set_completion_fade_seconds("45".into()).unwrap())
        .unwrap()
        .unwrap();
    let connection =
        rusqlite::Connection::open(service.environments.config.home.join("settings.sqlite3"))
            .unwrap();
    connection.execute_batch("CREATE TRIGGER reject_settings BEFORE INSERT ON app_settings BEGIN SELECT RAISE(FAIL, 'read only settings'); END;").unwrap();
    let error = service
        .runtime
        .block_on(service.set_completion_fade_seconds("60".into()).unwrap())
        .unwrap()
        .unwrap_err();
    assert!(error.contains("read only settings"));
    assert_eq!(service.completion_fade_seconds(), 45);
}
