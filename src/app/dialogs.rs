use tuicore::{
    Dialog, DialogAction, Flex, FlexItem, Key, KeyModifiers, KeySpec, Paragraph, ScrollContainer,
    TextInput,
};

use super::{Modal, Msg, rows::Row};

fn close() -> DialogAction<Msg> {
    DialogAction::new("Close")
        .hotkey(KeySpec::key(Key::Esc))
        .on_trigger(|| Msg::Close)
}
fn submit(label: &str) -> DialogAction<Msg> {
    DialogAction::new(label)
        .hotkey(KeySpec::key_with_modifiers(
            Key::Char('s'),
            KeyModifiers::CONTROL,
        ))
        .on_trigger(|| Msg::Submit)
}

pub(super) fn information(row: &Row) -> Modal {
    let directory = row.directory.clone();
    let compose = row.compose_file.clone();
    let content = format!(
        "Directory\n{}\n\nCompose file\n{}\n\n{}",
        directory, compose, row.compose_source
    );
    Dialog::new()
        .top_left(format!("Template / {}", row.template))
        .on_close(|_| Msg::Close)
        .actions([
            DialogAction::new("Copy directory")
                .hotkey(KeySpec::plain('d'))
                .on_trigger(move || Msg::Copy(directory.clone())),
            DialogAction::new("Copy Compose path")
                .hotkey(KeySpec::plain('c'))
                .on_trigger(move || Msg::Copy(compose.clone())),
            close(),
        ])
        .host(Flex::column().child(
            "source",
            ScrollContainer::vertical(Paragraph::new(content)),
            FlexItem::fill(1),
        ))
}

pub(super) fn name_entry(title: &str, description: &str) -> Modal {
    let mut input = TextInput::new()
        .placeholder("lowercase-name")
        .max_len(40)
        .on_change(Msg::NameChanged);
    input.set_insert_mode(true);
    Dialog::new()
        .top_left(title)
        .on_close(|_| Msg::Close)
        .actions([submit("Confirm"), close()])
        .host(
            Flex::column()
                .gap(1)
                .child(
                    "description",
                    Paragraph::new(description),
                    FlexItem::fit_content(),
                )
                .child("name", input, FlexItem::fixed(3)),
        )
}

pub(super) fn confirm_stop(name: &str) -> Modal {
    Dialog::new().top_left(format!("Stop {name}?"))
        .on_close(|_| Msg::Close).actions([submit("Stop instance"), close()])
        .host(Flex::column().child("warning", Paragraph::new("Remove this instance's containers and private networks?\nWorkspace, volumes, and the shared gateway are kept."), FlexItem::fit_content()))
}
