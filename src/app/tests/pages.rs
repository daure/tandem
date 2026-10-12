use super::*;
use std::cell::RefCell;

struct TimedPage {
    elapsed: Rc<Cell<Duration>>,
    tick: TickResult,
}

impl TuiNode<Msg> for TimedPage {
    fn layout(&mut self, area: Rect, _: &mut LayoutCtx) -> LayoutResult {
        LayoutResult::new(area)
    }

    fn render<'a>(&'a self, _: &mut Frame, _: Rect, _: &mut RenderCtx<'a>) {}

    fn tick(&mut self, dt: Duration, _: AnimationSettings) -> TickResult {
        self.elapsed.set(self.elapsed.get() + dt);
        self.tick
    }
}

#[test]
fn hidden_pages_keep_their_timers_without_invalidating_the_visible_page() {
    crate::app::tests::init_ui();
    let mut pages = Pages::new(
        std::array::from_fn(|_| KeySpec::plain('a')),
        Rc::new(RefCell::new(toolbar::State::default())),
        std::array::from_fn(|_| Rc::new(RefCell::new(super::super::instances::State::default()))),
        false,
    );
    let elapsed = Rc::new(Cell::new(Duration::ZERO));
    let animated = TickResult {
        changed: true,
        layout: true,
        active: false,
        next_tick: Some(Duration::from_millis(80)),
    };
    pages.pages = std::array::from_fn(|index| {
        ChildSlot::new(
            format!("page-{index}"),
            Box::new(TimedPage {
                elapsed: if index == 4 {
                    elapsed.clone()
                } else {
                    Rc::new(Cell::new(Duration::ZERO))
                },
                tick: if index == 4 {
                    animated
                } else {
                    TickResult::IDLE
                },
            }) as Box<dyn TuiNode<Msg>>,
        )
    });

    assert_eq!(
        pages.tick(Duration::from_millis(80), AnimationSettings::default()),
        TickResult::scheduled_after(Duration::from_millis(80))
    );
    assert_eq!(elapsed.get(), Duration::from_millis(80));
    pages.select_rules();
    assert_eq!(
        pages.tick(Duration::ZERO, AnimationSettings::default()),
        animated
    );
}
