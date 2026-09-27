use super::*;

#[test]
fn desktop_sound_catalog_lists_playable_files_and_deduplicates_roots() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().to_path_buf();
    fs::create_dir_all(root.join("theme/stereo")).unwrap();
    fs::write(root.join("theme/stereo/complete.oga"), "audio").unwrap();
    fs::write(root.join("theme/stereo/bell.wav"), "audio").unwrap();
    fs::write(root.join("theme/index.theme"), "metadata").unwrap();
    #[cfg(unix)]
    std::os::unix::fs::symlink(&root, root.join("theme/loop")).unwrap();
    let sounds = discover(&[root.clone(), root.clone(), root.join("missing")]);
    assert_eq!(
        sounds
            .iter()
            .map(|sound| sound.label.as_str())
            .collect::<Vec<_>>(),
        [
            "System default (complete)",
            "theme/stereo/bell",
            "theme/stereo/complete",
        ]
    );
    assert_eq!(
        sounds[2].id,
        root.join("theme/stereo/complete.oga").to_str().unwrap()
    );
}

#[test]
fn superseded_sound_requests_skip_playback_and_stop_active_players() {
    play_completion("", || true).unwrap();
    let started = Instant::now();
    play("sleep", &["30"], &|| true).unwrap();
    assert!(started.elapsed() < Duration::from_secs(2));
}
