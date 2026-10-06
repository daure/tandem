use super::*;
use std::{cell::RefCell, rc::Rc, time::Duration};

#[test]
fn rule_history_animates_starting_instances_when_animations_are_enabled() {
    init_ui();
    for enabled in [true, false] {
        let rule = super::rules::rule("selected");
        let mut acceptance = super::rules::acceptance(rule.clone(), 1, 7);
        acceptance.instance = "review".into();
        acceptance.status = crate::store::rules::DispatchStatus::Provisioning;
        let shared = Rc::new(RefCell::new(crate::store::rules::Snapshot {
            acceptances: vec![acceptance],
            ..Default::default()
        }));
        let mut inventory = snapshot();
        inventory.startup.insert(
            "review".into(),
            crate::store::environments::StartupTiming {
                elapsed_milliseconds: 1_000,
                estimate_milliseconds: Some(8_000),
                kind: crate::store::environments::StartupKind::Cold,
                ..Default::default()
            },
        );
        let context = Rc::new(RefCell::new(crate::app::acceptances::Context {
            inventory,
            ..Default::default()
        }));
        let mut view = crate::app::rules::Rules::for_rule(
            shared,
            rule.definition.name,
            AppService::for_tests().environment_keys(),
            context.clone(),
        );
        let area = Rect::new(0, 0, 130, 10);
        let mut terminal = Terminal::new(TestBackend::new(area.width, area.height)).unwrap();
        let mut render = |view: &mut crate::app::rules::Rules| {
            view.layout(area, &mut tuicore::LayoutCtx::new());
            terminal
                .draw(|frame| {
                    let mut ctx = RenderCtx::new();
                    view.render(frame, area, &mut ctx);
                    ctx.flush(frame);
                })
                .unwrap();
            rendered_lines(&terminal, area).join("\n")
        };
        assert!(render(&mut view).contains("⠋ review"));
        let settings = AnimationSettings {
            enabled,
            ..Default::default()
        };
        let result = view.tick(Duration::from_millis(80), settings);
        if enabled {
            assert!(result.changed && result.active);
        }
        let glyph = if enabled { "⠙" } else { "⠋" };
        let text = render(&mut view);
        assert!(text.contains(&format!("{glyph} review")), "{text}");
        context.borrow_mut().inventory.startup.clear();
        view.tick(Duration::from_millis(80), settings);
        let text = render(&mut view);
        assert!(text.contains(" review · Running"), "{text}");
    }
}
