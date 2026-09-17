use super::*;

#[test]
fn action_keys_use_enter_for_details_and_control_semicolon_for_opening() {
    let directory = tempfile::tempdir().unwrap();
    let config = Config::at(directory.path().to_path_buf(), "keys-test".into(), 9876).unwrap();
    assert!(config.keys[0].matches(KeyEvent::from(Key::Enter)));
    assert!(config.keys[10].matches(KeyEvent {
        code: Key::Char(';'),
        modifiers: KeyModifiers::CONTROL
    }));
    assert!(
        !config
            .keys
            .iter()
            .any(|key| key.matches(KeyEvent::from(Key::Char('v'))))
    );
    for (name, value, expected) in [
        ("INFO", "Enter", config.keys[0]),
        ("OPEN_COMMAND", "ctrl+;", config.keys[10]),
        ("INFO", "i", KeySpec::plain('i')),
        ("OPEN_COMMAND", "O", KeySpec::shifted('o')),
        ("REFRESH", "R", KeySpec::shifted('r')),
    ] {
        assert_eq!(action_key(name, value).unwrap(), expected);
    }
    for (name, value) in [
        ("INFO", "ctrl+;"),
        ("OPEN_COMMAND", "Enter"),
        ("REFRESH", "Enter"),
        ("INFO", ""),
        ("OPEN_COMMAND", ";"),
    ] {
        assert!(action_key(name, value).is_err());
    }
}
