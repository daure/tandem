use super::*;
use crate::store::environments::TemplateFile;

fn file(path: &str, directory: bool) -> TemplateFile {
    TemplateFile {
        path: path.into(),
        directory,
        size_bytes: 0,
        modified_at_unix_nanoseconds: None,
    }
}

#[test]
fn files_tab_requires_files_and_shows_an_expanded_tree() {
    init_ui();
    for files in [
        Vec::new(),
        vec![file("empty", true)],
        vec![file("config", true), file("config/settings.json", false)],
    ] {
        let mut inventory = snapshot();
        inventory.templates[0].files = files.clone();
        let row = rows::from_snapshot(&inventory).remove(0);
        let mut details = super::super::dialogs::details(&row);
        let area = Rect::new(0, 0, 130, 24);
        let mut terminal = Terminal::new(TestBackend::new(area.width, area.height)).unwrap();
        let mut render = |details: &mut super::super::Modal| {
            details.layout(area, &mut tuicore::LayoutCtx::new());
            terminal
                .draw(|frame| {
                    let mut ctx = RenderCtx::new();
                    details.render(frame, area, &mut ctx);
                    ctx.flush(frame);
                })
                .unwrap();
            rendered_lines(&terminal, area).join("\n")
        };
        let text = render(&mut details);
        let has_files = files.iter().any(|file| !file.directory);
        assert_eq!(
            text.lines().next().unwrap().contains("Files"),
            has_files,
            "{text}"
        );
        if has_files {
            for _ in 0..3 {
                details.event(
                    &TuiEvent::Key(KeyEvent::from(Key::Char(']'))),
                    &mut EventCtx::new(AnimationSettings::default()),
                );
            }
            let text = render(&mut details);
            assert!(text.contains("config/"), "{text}");
            assert!(text.contains("settings.json"), "{text}");
        }
    }
}

#[test]
fn open_files_tree_follows_snapshots_and_recovers_when_the_tab_is_removed() {
    init_ui();
    let mut app = root(AppService::for_tests());
    let mut inventory = snapshot();
    inventory.templates[0].files = vec![file("config", true), file("config/initial.txt", false)];
    app.update_snapshot(inventory.clone());
    let row = rows::from_snapshot(&inventory).remove(0);
    let mut events = EventCtx::new(AnimationSettings::default());
    app.open_details(&row, &mut events);
    let route = EventRoute::new(tuicore::TreePath::from_keys([tuicore::ChildKey::second()]));
    for _ in 0..3 {
        app.dispatch_event(
            &route,
            &TuiEvent::Key(KeyEvent::from(Key::Char(']'))),
            &mut events,
        );
    }
    let area = Rect::new(0, 0, 130, 40);
    let mut terminal = Terminal::new(TestBackend::new(area.width, area.height)).unwrap();
    let mut render = |app: &mut App| {
        app.layout(area, &mut tuicore::LayoutCtx::new());
        terminal
            .draw(|frame| {
                let mut ctx = RenderCtx::new();
                app.render(frame, area, &mut ctx);
                ctx.flush(frame);
            })
            .unwrap();
        rendered_lines(&terminal, area).join("\n")
    };
    assert!(render(&mut app).contains("initial.txt"));
    inventory.templates[0].guidance_source = Some("## New guidance".into());
    inventory.templates[0].compose_file.clear();
    inventory.templates[0].compose_source.clear();
    inventory.templates[0].files[1].path = "config/updated.txt".into();
    app.update_snapshot(inventory.clone());
    let text = render(&mut app);
    assert!(text.contains("updated.txt"), "{text}");
    assert!(!text.contains("initial.txt"), "{text}");
    inventory.templates[0].files.clear();
    app.update_snapshot(inventory);
    let text = render(&mut app);
    assert!(text.contains("Directory"), "{text}");
    assert!(!text.lines().any(|line| line.contains("· Files")), "{text}");
    assert!(app.view.is_active());
}
