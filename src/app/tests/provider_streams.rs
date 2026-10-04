use super::*;

#[test]
fn streams_start_expanded_and_overview_expands_them_from_every_tab() {
    init_ui();
    for tab in 0..5 {
        for event in [
            TuiEvent::Key(KeyEvent {
                code: Key::Char('H'),
                modifiers: KeyModifiers::SHIFT,
            }),
            TuiEvent::Hotkey(HotkeyEvent::Commit("shift+h".into())),
        ] {
            let mut app = crate::app::root(AppService::for_tests());
            let providers = ["alpha", "beta"].map(|name| {
                let mut row = provider(name, Status::Running);
                row.streams = vec![crate::store::providers::Stream {
                    name: format!("{name}-stream"),
                    controllable: true,
                    enabled: true,
                    status: Status::Running,
                    operation: None,
                    error: None,
                    total: 10,
                    handovers: 2,
                }];
                row
            });
            let mut snapshot = Snapshot {
                providers: providers.into(),
                error: None,
            };
            app.pages_mut().update_providers(snapshot.clone());
            app.pages_mut()
                .tick(Duration::ZERO, AnimationSettings::default());
            let streams_tab = app.providers_tab_index();
            app.tabs_mut().select_index(streams_tab);
            app.after_event(&mut EventCtx::default());
            let (layout, text) = render(&mut app, 130);
            for name in ["alpha-stream", "beta-stream"] {
                assert!(text.contains(name), "{text}");
            }
            let target = layout
                .focus_targets()
                .iter()
                .find(|target| target.id.as_str() == crate::app::providers::FOCUS)
                .unwrap();
            app.dispatch_focus(target, true, &mut tuicore::FocusCtx::default());
            app.dispatch_event(
                &EventRoute::new(target.path.clone()),
                &TuiEvent::Key(Key::Char('z').into()),
                &mut EventCtx::default(),
            );
            snapshot.providers[0].streams[0].total += 1;
            app.pages_mut().update_providers(snapshot);
            app.pages_mut()
                .tick(Duration::ZERO, AnimationSettings::default());
            let text = render(&mut app, 130).1;
            assert!(
                !text.contains("alpha-stream") && !text.contains("beta-stream"),
                "{text}"
            );
            app.tabs_mut().select_index(tab);
            app.after_event(&mut EventCtx::default());
            let (layout, _) = render(&mut app, 130);
            let target = layout
                .focus_targets()
                .iter()
                .find(|target| target.id.as_str() == "tabs")
                .unwrap();
            app.dispatch_event(
                &EventRoute::new(target.path.clone()),
                &event,
                &mut EventCtx::default(),
            );
            app.tabs_mut().select_index(streams_tab);
            app.after_event(&mut EventCtx::default());
            let text = render(&mut app, 130).1;
            for name in ["alpha-stream", "beta-stream"] {
                assert!(text.contains(name), "tab {tab}: {text}");
            }
        }
    }
}

#[test]
fn provider_rows_show_stream_driven_collector_transitions_without_masking_active_siblings() {
    init_ui();
    for (runtime, action, sibling_enabled, sibling_status, expected) in [
        (
            Status::Stopped,
            Some(Action::Start),
            false,
            Status::Stopped,
            "Starting...",
        ),
        (
            Status::Running,
            Some(Action::Start),
            false,
            Status::Stopped,
            "Starting...",
        ),
        (Status::Running, None, false, Status::Stopped, "Starting..."),
        (
            Status::Running,
            Some(Action::Start),
            true,
            Status::Running,
            "Healthy",
        ),
        (
            Status::Running,
            Some(Action::Stop),
            false,
            Status::Stopped,
            "Stopping...",
        ),
        (
            Status::Running,
            Some(Action::Stop),
            true,
            Status::Running,
            "Healthy",
        ),
        (
            Status::Running,
            Some(Action::Stop),
            true,
            Status::Starting,
            "Healthy",
        ),
    ] {
        let mut app = root(AppService::for_tests());
        let mut row = provider("message", runtime);
        row.streams = [
            crate::store::providers::Stream {
                name: "messages".into(),
                controllable: true,
                enabled: true,
                status: if action == Some(Action::Stop) {
                    Status::Running
                } else {
                    Status::Starting
                },
                operation: action,
                error: None,
                total: 0,
                handovers: 0,
            },
            crate::store::providers::Stream {
                name: "reactions".into(),
                controllable: true,
                enabled: sibling_enabled,
                status: sibling_status,
                operation: None,
                error: None,
                total: 0,
                handovers: 0,
            },
        ]
        .into();
        app.pages_mut().update_providers(Snapshot {
            providers: vec![row],
            error: None,
        });
        app.pages_mut()
            .tick(Duration::ZERO, AnimationSettings::default());
        app.handle_message(
            Msg::ShowProvider("dev-message".into()),
            &mut EventCtx::default(),
        );
        let text = render(&mut app, 130).1;
        assert!(
            text.contains(&format!("dev-message ·  0/0 · {expected}")),
            "{runtime:?} {action:?}: {text}"
        );
    }
}

#[test]
fn provider_tree_targets_stream_details_menus_and_confirmations_independently() {
    init_ui();
    let mut app = root(AppService::for_tests());
    let mut row = provider("message", Status::Running);
    row.streams = ["messages", "reactions"]
        .into_iter()
        .enumerate()
        .map(|(index, name)| crate::store::providers::Stream {
            name: name.into(),
            controllable: true,
            enabled: index == 0,
            status: if index == 0 {
                Status::Running
            } else {
                Status::Stopped
            },
            operation: None,
            error: None,
            total: 10 + index as u64,
            handovers: 2,
        })
        .collect();
    let snapshot = Snapshot {
        providers: vec![row.clone()],
        error: None,
    };
    app.pages_mut().update_providers(snapshot.clone());
    app.pages_mut()
        .tick(Duration::ZERO, AnimationSettings::default());
    app.handle_message(
        Msg::ShowProvider("dev-message".into()),
        &mut EventCtx::default(),
    );
    app.pages_mut()
        .tick(Duration::ZERO, AnimationSettings::default());
    let (layout, text) = render(&mut app, 130);
    assert!(text.contains("1/2 collecting"), "{text}");
    assert!(text.contains("messages ·  2/10 · Collecting"), "{text}");
    assert!(text.contains("reactions ·  2/11 · Stopped"), "{text}");
    let mut starting = row.clone();
    starting.streams[0].status = Status::Starting;
    app.pages_mut().update_providers(Snapshot {
        providers: vec![starting],
        error: None,
    });
    app.pages_mut()
        .tick(Duration::ZERO, AnimationSettings::default());
    let text = render(&mut app, 130).1;
    assert!(text.contains("messages ·  2/10 · Starting..."), "{text}");
    app.pages_mut().update_providers(snapshot.clone());
    app.pages_mut()
        .tick(Duration::ZERO, AnimationSettings::default());
    let target = layout
        .focus_targets()
        .iter()
        .find(|target| target.id.as_str() == crate::app::providers::FOCUS)
        .unwrap();
    let route = EventRoute::new(target.path.clone());
    app.dispatch_focus(target, true, &mut tuicore::FocusCtx::default());
    app.dispatch_event(
        &route,
        &TuiEvent::Key(KeyEvent::from(Key::Down)),
        &mut EventCtx::default(),
    );
    let mut ctx = EventCtx::default();
    app.dispatch_event(
        &route,
        &TuiEvent::Key(KeyEvent::from(Key::Char('d'))),
        &mut ctx,
    );
    assert!(
        matches!(ctx.messages(), [Msg::ProviderStreamDetails(name, stream)] if name == "message" && stream.name == "messages")
    );
    for message in ctx.drain_messages() {
        app.handle_message(message, &mut EventCtx::default());
    }
    let text = render(&mut app, 130).1;
    assert!(
        text.contains("Stream details") && text.contains("live_only"),
        "{text}"
    );
    app.handle_message(Msg::Close, &mut EventCtx::default());
    let mut ctx = EventCtx::default();
    app.dispatch_event(
        &route,
        &TuiEvent::Key(KeyEvent::from(Key::Char('.'))),
        &mut ctx,
    );
    for message in ctx.drain_messages() {
        app.handle_message(message, &mut EventCtx::default());
    }
    let text = render(&mut app, 130).1;
    for label in [
        "Stream details",
        "Start stream",
        "Stop stream",
        "Stream events",
    ] {
        assert!(text.contains(label), "{text}");
    }
    assert!(
        text.lines()
            .any(|line| line.contains("Stream events") && line.contains("Enter / e")),
        "{text}"
    );
    app.event(
        &TuiEvent::Key(KeyEvent::from(Key::Esc)),
        &mut EventCtx::default(),
    );
    app.dispatch_focus(target, true, &mut tuicore::FocusCtx::default());
    let mut activation = EventCtx::default();
    app.dispatch_event(&route, &TuiEvent::Key(Key::Enter.into()), &mut activation);
    assert!(
        matches!(activation.messages(), [Msg::ProviderStreamEvents(name, stream)] if name == "dev-message" && stream == "messages")
    );
    app.handle_message(
        Msg::ProviderStreamAction("message".into(), "messages".into(), Action::Stop),
        &mut EventCtx::default(),
    );
    assert_eq!(
        app.provider_stream_confirmation,
        Some(("message".into(), "messages".into(), Action::Stop))
    );
    assert!(app.provider_confirmation.is_none());
    app.handle_message(Msg::Close, &mut EventCtx::default());
    let mut stopped = row.clone();
    stopped.status = Status::Stopped;
    for stream in &mut stopped.streams {
        stream.status = Status::Stopped;
    }
    let target = crate::app::row_actions::Target::Stream(Box::new((
        stopped.clone(),
        stopped.streams[0].clone(),
    )));
    assert!(target.enabled(crate::app::row_actions::Command::StartStream));
    assert!(!target.enabled(crate::app::row_actions::Command::StopStream));
    app.pages_mut().update_providers(Snapshot {
        providers: vec![stopped],
        error: None,
    });
    app.pages_mut()
        .tick(Duration::ZERO, AnimationSettings::default());
    let mut ctx = EventCtx::default();
    app.dispatch_event(
        &route,
        &TuiEvent::Key(KeyEvent::from(Key::Char('s'))),
        &mut ctx,
    );
    assert!(
        matches!(ctx.messages(), [Msg::ProviderStreamAction(name, stream, Action::Start)] if name == "message" && stream == "messages")
    );
    app.pages_mut().update_providers(snapshot.clone());
    app.pages_mut()
        .tick(Duration::ZERO, AnimationSettings::default());
    app.dispatch_event(
        &route,
        &TuiEvent::Key(KeyEvent::from(Key::Down)),
        &mut EventCtx::default(),
    );
    app.pages_mut().update_providers(Snapshot {
        providers: vec![row],
        error: Some("observation warning".into()),
    });
    app.pages_mut()
        .tick(Duration::ZERO, AnimationSettings::default());
    let mut ctx = EventCtx::default();
    app.dispatch_event(
        &route,
        &TuiEvent::Key(KeyEvent::from(Key::Char('s'))),
        &mut ctx,
    );
    assert!(
        matches!(ctx.messages(), [Msg::ProviderStreamAction(name, stream, Action::Start)] if name == "message" && stream == "reactions")
    );
    app.dispatch_event(
        &route,
        &TuiEvent::Key(KeyEvent::from(Key::Left)),
        &mut EventCtx::default(),
    );
    app.dispatch_event(
        &route,
        &TuiEvent::Key(KeyEvent::from(Key::Left)),
        &mut EventCtx::default(),
    );
    app.pages_mut().update_providers(snapshot);
    app.pages_mut()
        .tick(Duration::ZERO, AnimationSettings::default());
    assert!(!render(&mut app, 130).1.contains("reactions · "));
}

#[test]
fn stream_event_links_select_one_exact_stream_and_shift_enter_clears_the_selection() {
    init_ui();
    let mut app = root(AppService::for_tests());
    let records = ["messages", "reactions", "messages-other"]
        .into_iter()
        .enumerate()
        .map(|(index, stream)| {
            let mut event = crate::environments::events::tests::event(&format!("event-{index}"));
            event.stream = stream.into();
            if let crate::store::events::Payload::Message(message) = &mut event.payload {
                message.text = format!("Body for {stream}");
            }
            crate::store::events::Record {
                sequence: index as i64,
                provider: "dev-message".into(),
                received_at: "now".into(),
                event,
                attempts: vec![],
                acceptances: vec![],
            }
        })
        .collect();
    app.pages_mut()
        .update_events(crate::store::events::Snapshot {
            records,
            total: 3,
            ..Default::default()
        });
    app.handle_message(
        Msg::ProviderStreamEvents("dev-message".into(), "messages".into()),
        &mut EventCtx::default(),
    );
    super::super::events::show_all_events(&mut app);
    app.pages_mut()
        .tick(Duration::ZERO, AnimationSettings::default());
    let text = render(&mut app, 130).1;
    assert!(
        text.contains("dev-message · messages") && text.contains("1 of 3 events"),
        "{text}"
    );
    assert!(text.contains("Body for messages"), "{text}");
    assert!(
        !text.contains("Body for reactions") && !text.contains("Body for messages-other"),
        "{text}"
    );
    app.pages_mut().focus_overview(
        &TuiEvent::Key(KeyEvent {
            code: Key::Enter,
            modifiers: KeyModifiers::SHIFT,
        }),
        &mut EventCtx::default(),
    );
    app.pages_mut()
        .tick(Duration::ZERO, AnimationSettings::default());
    let text = render(&mut app, 130).1;
    assert!(text.contains("3 of 3 events"), "{text}");
}
