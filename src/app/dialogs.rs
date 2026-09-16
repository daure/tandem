use ratatui::widgets::Borders;
use tuicore::{
    CrossSize, Dialog, DialogAction, Flex, FlexItem, FormField, KeySpec, Language, Paragraph,
    SyntaxHighlighter, Tab, Tabs, TabsVariant, TextInput, Toggle,
};

use super::{Modal, Msg, properties::Properties, rows::Row};

fn create_new() -> DialogAction<Msg> {
    DialogAction::new("Create new")
        .hotkey(KeySpec::plain('n'))
        .on_trigger(|| Msg::Submit)
}

fn confirm(label: &str, hotkey: KeySpec) -> DialogAction<Msg> {
    DialogAction::new(label)
        .hotkey(hotkey)
        .on_trigger(|| Msg::Submit)
}

fn cancel() -> DialogAction<Msg> {
    DialogAction::new("Cancel")
        .hotkey(KeySpec::plain('c'))
        .on_trigger(|| Msg::Close)
}

pub(super) fn details(row: &Row) -> Modal {
    let title = if row.parent.is_none() {
        "Metadata"
    } else {
        "Details"
    };
    let mut tabs = vec![Tab::new(title, Properties::new(row.details.clone()))];
    if row.parent.is_none() {
        tabs.push(Tab::new(
            "Compose",
            SyntaxHighlighter::new(
                row.compose_source.clone(),
                Language::guess(Some(&row.compose_file), &row.compose_source),
            )
            .wrap(true),
        ));
        tabs.push(match &row.manifest_source {
            Some(source) => Tab::new(
                "Manifest",
                SyntaxHighlighter::new(
                    source.clone(),
                    Language::guess(Some("tandem.json"), source),
                )
                .wrap(true),
            ),
            None => Tab::new(
                "Manifest",
                Paragraph::new("tandem.json is unavailable. Check Metadata for template errors."),
            ),
        });
    }
    Box::new(
        Tabs::dialog(tabs)
            .variant(TabsVariant::OneRow)
            .edge_borders(Borders::TOP)
            .on_close(|_| Msg::Close),
    )
}

pub(super) fn name_entry(
    title: &str,
    value: &str,
    placeholder: &str,
    allowed_chars: Option<&str>,
) -> Modal {
    let mut input = TextInput::new()
        .placeholder(placeholder)
        .on_change(Msg::NameChanged);
    if let Some(allowed_chars) = allowed_chars {
        input = input.allowed_chars(allowed_chars).max_len(40);
    }
    input.set_value(value);
    input.set_insert_mode(true);
    input.move_cursor_to_end();
    Box::new(
        Dialog::new()
            .top_left(title)
            .on_close(|_| Msg::Close)
            .actions([create_new(), cancel()])
            .host(Flex::column().child(
                "name",
                input,
                FlexItem::fit_content().cross_size(CrossSize::Fixed(50)),
            )),
    )
}

pub(super) fn settings(branch_instances: bool, open_command: &str) -> Modal {
    let mut input = TextInput::new()
        .placeholder("Empty: open workspace folder")
        .on_change(Msg::OpenCommandChanged);
    input.set_value(open_command);
    input.set_insert_mode(true);
    input.move_cursor_to_end();
    Box::new(
        Dialog::new()
            .top_left("Settings")
            .on_close(|_| Msg::Close)
            .actions([DialogAction::new("Close")
                .hotkey(KeySpec::plain('c'))
                .on_trigger(|| Msg::Close)])
            .host(
                Flex::column()
                    .child(
                        "branch-instances",
                        Toggle::new("Branch instances")
                            .checked(branch_instances)
                            .focused(true)
                            .on_change(Msg::SetBranchInstances),
                        FlexItem::content(),
                    )
                    .child(
                        "open-command",
                        FormField::new("Open command", input),
                        FlexItem::fit_content().cross_size(CrossSize::Fixed(72)),
                    ),
            ),
    )
}

pub(super) fn confirm_stop(name: &str) -> Modal {
    confirmation(
        "Stop instance",
        "Stop",
        KeySpec::plain('s'),
        format!("Stop {name}? Data stays for restart."),
    )
}

pub(super) fn confirm_start(name: &str) -> Modal {
    confirmation(
        "Start instance",
        "Start",
        KeySpec::plain('s'),
        format!("Start {name}?"),
    )
}

pub(super) fn confirm_delete(name: &str) -> Modal {
    confirmation(
        "Delete instance",
        "Delete",
        KeySpec::plain('d'),
        format!("Delete {name}? This permanently removes its data."),
    )
}

pub(super) fn confirm_stop_template(name: &str) -> Modal {
    confirmation(
        "Stop all instances",
        "Stop all",
        KeySpec::plain('s'),
        format!("Stop every {name} instance? Data stays for restart."),
    )
}

pub(super) fn confirm_delete_template(name: &str) -> Modal {
    confirmation(
        "Purge all instances",
        "Purge all",
        KeySpec::plain('p'),
        format!("Purge every {name} instance? This permanently removes their data."),
    )
}

fn confirmation(title: &str, action: &str, hotkey: KeySpec, description: String) -> Modal {
    Box::new(
        Dialog::new()
            .top_left(title)
            .on_close(|_| Msg::Close)
            .actions([confirm(action, hotkey), cancel()])
            .host(Flex::column().child(
                "warning",
                Paragraph::new(description),
                FlexItem::fit_content(),
            )),
    )
}

pub(super) fn confirm_remove_template(name: &str, directory: &str) -> Modal {
    confirmation(
        "Delete template",
        "Delete",
        KeySpec::plain('d'),
        format!(
            "Permanently delete template {name}:\n{directory}\n\nStop and remove all its instances, delete their workspace folders,\nvolumes and networks, then delete the template and all its files.\nThis cannot be undone."
        ),
    )
}
