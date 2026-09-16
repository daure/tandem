use std::time::Duration;

use ratatui::{
    Frame,
    layout::{Constraint, Rect},
};
use tuicore::{
    AnimationSettings, Column, DataView, Dialog, DialogHost, DialogLayer, EventCtx, EventOutcome,
    EventRoute, Flex, FocusCtx, FocusId, FocusTarget, KeySpec, LayoutCtx, LayoutProposal,
    LayoutResult, LayoutSizeHint, LifecycleCtx, Panel, PanelHost, Paragraph, RenderCtx,
    ScrollContainer, SelectionMode, Split, StatusBar, TickResult, TreeAdapter, TuiEvent, TuiNode,
};

use crate::{
    service::AppService,
    store::environments::{EnvironmentSnapshot, OperationState},
};

mod dialogs;
mod rows;
use rows::Row;

const TREE_FOCUS: &str = "environments";

pub(crate) fn initial_focus() -> tuicore::FocusRequest {
    tuicore::FocusRequest::Target(FocusId::new(TREE_FOCUS))
}

#[derive(Debug)]
pub(crate) enum Msg {
    Close,
    NameChanged(String),
    Submit,
    Copy(String),
}

enum Intent {
    Start(String),
    NewTemplate,
    Stop(String),
}

type Tree = PanelHost<DataView<Row, String>, Msg>;
type Content = Split<Tree, PanelHost<ScrollContainer<Paragraph, Msg>, Msg>>;
type Modal = DialogHost<Flex<Msg>, Msg>;
type MainView = DialogLayer<Content, Modal>;
type View = Split<MainView, Flex<Msg>>;

pub(crate) struct App {
    service: AppService,
    snapshot: EnvironmentSnapshot,
    view: View,
    keys: [KeySpec; 5],
    poll_elapsed: Duration,
    detail: String,
    intent: Option<Intent>,
    name: String,
    notice: String,
}

pub(crate) fn root(service: AppService) -> App {
    let keys = service.environment_keys().map(KeySpec::plain);
    let snapshot = service.environment_snapshot();
    let tree = DataView::new(rows::from_snapshot(&snapshot), |row: &Row| row.id.clone())
        .focus_id(TREE_FOCUS)
        .columns(vec![
            Column::text(
                "name",
                "Templates / instances",
                Constraint::Percentage(60),
                |row: &Row| row.label.clone(),
            ),
            Column::text(
                "status",
                "Status",
                Constraint::Percentage(40),
                |row: &Row| row.status.clone(),
            ),
        ])
        .headers(true)
        .action_bar(true)
        .selection_mode(SelectionMode::Single)
        .tree(TreeAdapter::parent_id(|row: &Row| row.parent.clone()));
    let hints = format!(
        "{} info · {} start · {} template · {} stop · {} refresh",
        keys[0].label(),
        keys[1].label(),
        keys[2].label(),
        keys[3].label(),
        keys[4].label()
    );
    let content = Split::horizontal(
        Panel::new()
            .top_left("Tandem")
            .bottom_left(format!(
                "{} expand / collapse",
                tuicore::keybindings().data_view().toggle_expansion_label()
            ))
            .host(tree),
        Panel::new()
            .top_left("Details / operations")
            .host(ScrollContainer::vertical(Paragraph::new(
                "Discovering templates and instances…",
            ))),
    )
    .ratio(3, 5);
    let modal = Dialog::<Msg>::new()
        .on_close(|_| Msg::Close)
        .host(Flex::column());
    let main = DialogLayer::new(content, modal)
        .active(false)
        .fit_content()
        .fit_content_max(110, 34);
    let footer = Flex::column()
        .child(
            "actions",
            Paragraph::new(hints),
            tuicore::FlexItem::fixed(1),
        )
        .child(
            "status",
            StatusBar::new().ai_enabled(false),
            tuicore::FlexItem::fixed(1),
        );
    let view =
        Split::vertical(main, footer).constraints(Constraint::Fill(1), Constraint::Length(2));
    service.poll_environments();
    App {
        service,
        snapshot,
        view,
        keys,
        poll_elapsed: Duration::ZERO,
        detail: String::new(),
        intent: None,
        name: String::new(),
        notice: String::new(),
    }
}

impl App {
    fn tree(&self) -> &DataView<Row, String> {
        self.view.first().base().first().child()
    }
    fn tree_mut(&mut self) -> &mut DataView<Row, String> {
        self.view.first_mut().base_mut().first_mut().child_mut()
    }
    fn selected(&self) -> Option<Row> {
        let id = self.tree().highlighted_id()?;
        self.tree().rows().iter().find(|row| row.id == id).cloned()
    }

    pub(crate) fn handle_message(&mut self, message: Msg, ctx: &mut EventCtx<Msg>) {
        match message {
            Msg::Close => {
                self.view.first_mut().set_active_with_context(false, ctx);
                ctx.focus(initial_focus());
                self.intent = None;
            }
            Msg::NameChanged(name) => self.name = name,
            Msg::Copy(text) => ctx.copy_to_clipboard(text),
            Msg::Submit => {
                let result = match &self.intent {
                    Some(Intent::Start(template)) => self.service.submit_operation(
                        "create_instance",
                        &self.name,
                        Some(template.clone()),
                        600,
                        true,
                    ),
                    Some(Intent::NewTemplate) => {
                        self.service
                            .submit_operation("create_template", &self.name, None, 60, true)
                    }
                    Some(Intent::Stop(name)) => {
                        self.service
                            .submit_operation("stop_instance", name, None, 60, true)
                    }
                    None => return,
                };
                match result {
                    Ok(operation) => {
                        self.notice = format!("{} queued: {}", operation.action, operation.name);
                        self.view.first_mut().set_active_with_context(false, ctx);
                        ctx.focus(initial_focus());
                        self.intent = None;
                    }
                    Err(error) => {
                        self.notice = error.clone();
                        self.view
                            .first_mut()
                            .layer_mut()
                            .dialog_mut()
                            .set_bottom_left(error);
                    }
                }
            }
        }
        self.refresh_detail();
        ctx.request_redraw();
    }

    fn open(&mut self, modal: Modal, ctx: &mut EventCtx<Msg>) {
        self.view.first_mut().replace_layer(modal, ctx);
        self.view.first_mut().set_active_with_context(true, ctx);
        if matches!(self.intent, Some(Intent::Start(_) | Intent::NewTemplate)) {
            ctx.focus(tuicore::FocusRequest::Path(tuicore::TreePath::from_keys([
                tuicore::ChildKey::first(),
                tuicore::ChildKey::second(),
                tuicore::ChildKey::body(),
                tuicore::ChildKey::new("name"),
            ])));
        }
    }

    fn action(&mut self, index: usize, ctx: &mut EventCtx<Msg>) {
        let row = self.selected();
        self.name.clear();
        match index {
            0 => {
                if let Some(row) = row {
                    self.intent = None;
                    self.open(dialogs::information(&row), ctx);
                }
            }
            1 => {
                if let Some(row) = row {
                    self.intent = Some(Intent::Start(row.template.clone()));
                    self.open(dialogs::name_entry(&format!("Start instance / {}", row.template), "Run this trusted template with local Docker privileges.\nThe gateway starts automatically; readiness progress appears in Details."), ctx);
                }
            }
            2 => {
                self.intent = Some(Intent::NewTemplate);
                self.open(
                    dialogs::name_entry(
                        "New template",
                        "Create a dedicated editable Compose folder with a working web starter.",
                    ),
                    ctx,
                );
            }
            3 => {
                if let Some(name) = row.and_then(|row| row.instance) {
                    self.intent = Some(Intent::Stop(name.clone()));
                    self.open(dialogs::confirm_stop(&name), ctx);
                }
            }
            4 => self.service.poll_environments(),
            _ => {}
        }
        ctx.request_redraw();
    }

    fn refresh_detail(&mut self) -> bool {
        let mut detail = self.selected().map(|row| row.details).unwrap_or_else(|| format!(
            "Templates root\n{}\n\nGateway\n{}\n\n{} new template · MCP create_template / get_instructions\n\nExpand template rows to browse instances.", self.snapshot.templates_root, self.snapshot.gateway_origin, self.keys[2].label()));
        if let Some(error) = &self.snapshot.error {
            detail.push_str(&format!("\n\nRefresh warning\n{error}"));
        }
        if !self.notice.is_empty() {
            detail.push_str(&format!("\n\n{}", self.notice));
        }
        for operation in self.service.operations().iter().rev().take(3) {
            let state = match operation.state {
                OperationState::Running => "running",
                OperationState::Succeeded => "succeeded",
                OperationState::Failed => "failed",
            };
            detail.push_str(&format!(
                "\n\n{} / {} · {state} · {}s",
                operation.action, operation.name, operation.elapsed_seconds
            ));
            for line in operation.progress.iter().rev().take(4).rev() {
                detail.push_str(&format!("\n{line}"));
            }
            if let Some(error) = &operation.error {
                detail.push_str(&format!("\n{error}"));
            }
        }
        if detail == self.detail {
            return false;
        }
        self.detail = detail.clone();
        self.view
            .first_mut()
            .base_mut()
            .second_mut()
            .child_mut()
            .child_mut()
            .set_text(detail);
        true
    }

    fn handle_key(&mut self, event: &TuiEvent, ctx: &mut EventCtx<Msg>) -> bool {
        if self.view.first().is_active()
            && matches!(self.intent, Some(Intent::Start(_) | Intent::NewTemplate))
            && let TuiEvent::Key(key) = event
            && KeySpec::key(tuicore::Key::Enter).matches(*key)
        {
            self.handle_message(Msg::Submit, ctx);
            ctx.stop_propagation();
            return true;
        }
        if self.view.first().is_active() || self.tree().is_searching() {
            return false;
        }
        if let TuiEvent::Key(key) = event
            && let Some(index) = self.keys.iter().position(|spec| spec.matches(*key))
        {
            self.action(index, ctx);
            ctx.stop_propagation();
            return true;
        }
        false
    }

    fn after_event(&mut self, ctx: &mut EventCtx<Msg>) {
        let _ = self.tree_mut().take_events();
        if self.refresh_detail() {
            ctx.request_redraw();
        }
    }
}

impl TuiNode<Msg> for App {
    fn measure(&self, proposal: LayoutProposal) -> LayoutSizeHint {
        self.view.measure(proposal)
    }
    fn layout(&mut self, area: Rect, ctx: &mut LayoutCtx) -> LayoutResult {
        self.view.layout(area, ctx)
    }
    fn render<'a>(&'a self, frame: &mut Frame, area: Rect, ctx: &mut RenderCtx<'a>) {
        self.view.render(frame, area, ctx);
    }
    fn event(&mut self, event: &TuiEvent, ctx: &mut EventCtx<Msg>) -> EventOutcome {
        if self.handle_key(event, ctx) {
            return EventOutcome::Handled;
        }
        let outcome = self.view.event(event, ctx);
        self.after_event(ctx);
        outcome
    }
    fn dispatch_event(
        &mut self,
        route: &EventRoute,
        event: &TuiEvent,
        ctx: &mut EventCtx<Msg>,
    ) -> EventOutcome {
        if self.handle_key(event, ctx) {
            return EventOutcome::Handled;
        }
        let outcome = self.view.dispatch_event(route, event, ctx);
        self.after_event(ctx);
        outcome
    }
    fn tick(&mut self, dt: Duration, settings: AnimationSettings) -> TickResult {
        self.poll_elapsed = self.poll_elapsed.saturating_add(dt);
        if self.poll_elapsed >= Duration::from_secs(2) {
            self.poll_elapsed = Duration::ZERO;
            self.service.poll_environments();
        }
        let snapshot = self.service.environment_snapshot();
        let mut changed = false;
        if snapshot != self.snapshot {
            self.tree_mut().set_rows(rows::from_snapshot(&snapshot));
            self.snapshot = snapshot;
            changed = true;
        }
        changed |= self.refresh_detail();
        let mut result = self.view.tick(dt, settings);
        if changed {
            result = result.merge(TickResult::CHANGED);
        }
        result.merge(TickResult::scheduled_after(Duration::from_millis(250)))
    }
    fn take_pending_focus_request(&mut self) -> Option<tuicore::FocusRequest> {
        self.view.take_pending_focus_request()
    }
    fn take_pending_clipboard_request(&mut self) -> Option<String> {
        self.view.take_pending_clipboard_request()
    }
    fn focus(&mut self, target: Option<&FocusId>, focused: bool, ctx: &mut FocusCtx<Msg>) {
        self.view.focus(target, focused, ctx);
    }
    fn dispatch_focus(&mut self, target: &FocusTarget, focused: bool, ctx: &mut FocusCtx<Msg>) {
        self.view.dispatch_focus(target, focused, ctx);
    }
    fn focus_reveal_area(&self, target: &FocusTarget) -> Option<Rect> {
        self.view.focus_reveal_area(target)
    }
    fn focus_reveal_centered(&self, target: &FocusTarget) -> bool {
        self.view.focus_reveal_centered(target)
    }
    fn init(&mut self, ctx: &mut LifecycleCtx<Msg>) {
        self.view.init(ctx);
    }
    fn mount(&mut self, ctx: &mut LifecycleCtx<Msg>) {
        self.view.mount(ctx);
    }
    fn unmount(&mut self, ctx: &mut LifecycleCtx<Msg>) {
        self.view.unmount(ctx);
    }
    fn destroy(&mut self, ctx: &mut LifecycleCtx<Msg>) {
        self.view.destroy(ctx);
    }
}

#[cfg(test)]
mod tests;
