use super::*;
use std::collections::BTreeSet;

impl TuiNode<Msg> for Rules {
    fn measure(&self, proposal: LayoutProposal) -> LayoutSizeHint {
        let mut hint = <RuleView as TuiNode<Msg>>::measure(&self.view, proposal);
        hint.preferred.height = hint
            .preferred
            .height
            .saturating_add(u16::from(self.control.is_some()));
        hint.normalized(proposal)
    }

    fn layout(&mut self, area: Rect, ctx: &mut LayoutCtx) -> LayoutResult {
        let alternate: BTreeSet<_> = self
            .view
            .base()
            .rows()
            .iter()
            .filter(|row| {
                tuicore::search_match(
                    &self.view.base().transform_state().search,
                    &row.search(),
                    tuicore::SearchMode::Fuzzy,
                )
                .is_some()
            })
            .enumerate()
            .filter(|(index, _)| index % 2 == 0)
            .map(|(_, row)| row.id())
            .collect();
        self.view.base_mut().set_row_style_by(move |row| {
            alternate
                .contains(&row.id())
                .then(|| Style::default().bg(tuicore::theme().surface_bg()))
        });
        let height = u16::from(self.control.is_some()).min(area.height);
        if let Some(control) = &mut self.control {
            let shared = self.shared.borrow();
            let enabled = bulk_enabled(&shared);
            control.child_mut().set_label(if enabled {
                "Activate all"
            } else {
                "Deactivate all"
            });
            control.child_mut().set_disabled(
                !shared
                    .rules
                    .iter()
                    .any(|rule| rule.definition.enabled != enabled),
            );
            let width = control
                .measure(LayoutProposal::at_most(area.width, height))
                .preferred
                .width
                .min(area.width);
            self.control_area =
                Rect::new(area.right().saturating_sub(width), area.y, width, height);
            control.layout(self.control_area, ctx);
        }
        self.view_area = Rect::new(
            area.x,
            area.y.saturating_add(height),
            area.width,
            area.height.saturating_sub(height),
        );
        ctx.push_slot(ChildKey::new(DATA_SLOT), self.view_area, |ctx| {
            <RuleView as TuiNode<Msg>>::layout(&mut self.view, self.view_area, ctx)
        });
        LayoutResult::new(area)
    }

    fn render<'a>(&'a self, frame: &mut Frame, _area: Rect, ctx: &mut RenderCtx<'a>) {
        if let Some(control) = &self.control {
            control.render(frame, self.control_area, ctx);
        }
        <RuleView as TuiNode<Msg>>::render(&self.view, frame, self.view_area, ctx);
    }

    fn event(&mut self, event: &TuiEvent, ctx: &mut EventCtx<Msg>) -> EventOutcome {
        if !self.view.is_active()
            && !self.view.base().is_searching()
            && let Some(control) = &mut self.control
            && control.child_mut().event(event, ctx) == EventOutcome::Handled
        {
            ctx.stop_propagation();
            return EventOutcome::Handled;
        }
        if self.action(event, ctx) {
            return EventOutcome::Handled;
        }
        let outcome = if self.view.is_active() {
            self.view.event(event, ctx)
        } else {
            self.view.base_mut().event(event, ctx)
        };
        self.drain(ctx);
        outcome
    }

    fn dispatch_event(
        &mut self,
        route: &EventRoute,
        event: &TuiEvent,
        ctx: &mut EventCtx<Msg>,
    ) -> EventOutcome {
        if !self.view.is_active()
            && !self.view.base().is_searching()
            && let Some(control) = &mut self.control
            && control.dispatch_event(route, event, ctx) == EventOutcome::Handled
        {
            ctx.stop_propagation();
            return EventOutcome::Handled;
        }
        if self.action(event, ctx) {
            return EventOutcome::Handled;
        }
        let Some(path) = route.path.without_first_if(&ChildKey::new(DATA_SLOT)) else {
            return EventOutcome::Ignored;
        };
        let outcome = self.view.dispatch_event(&EventRoute::new(path), event, ctx);
        self.drain(ctx);
        outcome
    }

    fn tick(&mut self, dt: Duration, settings: AnimationSettings) -> TickResult {
        let changed = self.sync();
        let mut result = <RuleView as TuiNode<Msg>>::tick(&mut self.view, dt, settings);
        if self.rows.iter().any(|row| {
            matches!(row, Entry::Acceptance(target) if target.instance.as_ref().is_some_and(|instance| instance.loading || instance.secondary_loading))
        }) {
            result = result.merge(Animated::tick(
                &mut *self.spinner.borrow_mut(),
                dt,
                settings,
            ));
        }
        if let Some(control) = &mut self.control {
            result = result.merge(control.tick(dt, settings));
        }
        result.merge(if changed {
            TickResult {
                layout: true,
                ..TickResult::CHANGED
            }
        } else {
            TickResult::IDLE
        })
    }

    fn focus(&mut self, target: Option<&FocusId>, focused: bool, ctx: &mut FocusCtx<Msg>) {
        if self.view.is_active() {
            self.view.layer_mut().focus(target, focused, ctx);
        } else {
            self.view.base_mut().focus(target, focused, ctx);
        }
        if let Some(control) = &mut self.control {
            control.child_mut().focus(target, focused, ctx);
        }
    }

    fn dispatch_focus(&mut self, target: &FocusTarget, focused: bool, ctx: &mut FocusCtx<Msg>) {
        if let Some(target) = target.for_child(&ChildKey::new(DATA_SLOT)) {
            self.view.dispatch_focus(&target, focused, ctx);
        }
        if let Some(control) = &mut self.control {
            control.dispatch_focus(target, focused, ctx);
        }
    }

    fn focus_reveal_area(&self, target: &FocusTarget) -> Option<Rect> {
        if let Some(area) = self
            .control
            .as_ref()
            .and_then(|control| control.focus_reveal_area(target))
        {
            return Some(area);
        }
        let target = target.for_child(&ChildKey::new(DATA_SLOT))?;
        let target = target.for_child(&ChildKey::first())?;
        <DataView<Entry, String> as TuiNode<Msg>>::focus_reveal_area(self.view.base(), &target)
    }

    fn focus_reveal_centered(&self, target: &FocusTarget) -> bool {
        target
            .for_child(&ChildKey::new(DATA_SLOT))
            .and_then(|target| target.for_child(&ChildKey::first()))
            .is_some_and(|target| {
                <DataView<Entry, String> as TuiNode<Msg>>::focus_reveal_centered(
                    self.view.base(),
                    &target,
                )
            })
    }

    fn take_pending_focus_request(&mut self) -> Option<tuicore::FocusRequest> {
        <RuleView as TuiNode<Msg>>::take_pending_focus_request(&mut self.view)
    }

    fn take_pending_clipboard_request(&mut self) -> Option<String> {
        <DataView<Entry, String> as TuiNode<Msg>>::take_pending_clipboard_request(
            self.view.base_mut(),
        )
    }

    fn init(&mut self, ctx: &mut LifecycleCtx<Msg>) {
        self.view.init(ctx);
        if let Some(control) = &mut self.control {
            control.init(ctx);
        }
    }
    fn mount(&mut self, ctx: &mut LifecycleCtx<Msg>) {
        self.view.mount(ctx);
        if let Some(control) = &mut self.control {
            control.mount(ctx);
        }
    }
    fn unmount(&mut self, ctx: &mut LifecycleCtx<Msg>) {
        self.view.unmount(ctx);
        if let Some(control) = &mut self.control {
            control.unmount(ctx);
        }
    }
    fn destroy(&mut self, ctx: &mut LifecycleCtx<Msg>) {
        self.view.destroy(ctx);
        if let Some(control) = &mut self.control {
            control.destroy(ctx);
        }
    }
}
