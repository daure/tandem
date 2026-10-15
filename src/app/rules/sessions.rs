use std::{cell::RefCell, rc::Rc};

use tuicore::{ButtonGroup, ButtonGroupItem, Flex, FlexItem, FormField, Paragraph, TextInput};

use super::Draft;
use crate::{app::Msg, store::rules::DeliveryMode};

pub(super) fn settings(draft: Rc<RefCell<Draft>>) -> Flex<Msg> {
    let name_draft = draft.clone();
    let definition = draft.borrow().rule.definition.clone();
    let name = TextInput::new()
        .value(definition.session_name.as_deref().unwrap_or_default())
        .placeholder("Blank opens a fresh conversation per match")
        .on_change(move |value| {
            name_draft.borrow_mut().rule.definition.session_name =
                (!value.is_empty()).then_some(value);
            Msg::RuleDraftChanged(name_draft.clone())
        });
    let modes = [
        DeliveryMode::Queue,
        DeliveryMode::Steer,
        DeliveryMode::Interrupt,
    ];
    let delivery = ButtonGroup::new(modes.map(|mode| ButtonGroupItem::new(mode, mode.label())))
        .selected(
            modes
                .iter()
                .position(|mode| *mode == definition.delivery_mode)
                .unwrap_or_default(),
        )
        .on_select(move |mode| {
            draft.borrow_mut().rule.definition.delivery_mode = *mode;
            Msg::RuleDraftChanged(draft.clone())
        });
    Flex::column()
        .child("session-name", FormField::new("Session name (requires target instance)", name), FlexItem::fit_content())
        .child("delivery-mode", FormField::new("Named-session delivery", delivery), FlexItem::fit_content())
        .child("delivery-help", Paragraph::new("Queue waits; Steer guides ongoing work; Interrupt stops execution before sending. Completed actions remain."), FlexItem::fit_content())
}
