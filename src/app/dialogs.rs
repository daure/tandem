use ratatui::widgets::Borders;
use tuicore::{
    CrossSize, Dialog, DialogAction, Flex, FlexItem, FormField, KeySpec, Language, Padding,
    Paragraph, SyntaxHighlighter, Tab, Tabs, TabsVariant, TextInput, Toggle,
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
        if !row.compose_file.is_empty() {
            tabs.push(Tab::new(
                "Compose",
                SyntaxHighlighter::new(
                    row.compose_source.clone(),
                    Language::guess(Some(&row.compose_file), &row.compose_source),
                )
                .wrap(true),
            ));
        }
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
                Paragraph::new("No tandem.json specified; default template settings apply."),
            ),
        });
        if let Some(source) = &row.guidance_source {
            tabs.push(Tab::new(
                "Guidance",
                SyntaxHighlighter::new(
                    source.clone(),
                    Language::guess(Some("tandem-agents.md"), source),
                )
                .wrap(true),
            ));
        }
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

pub(super) fn instance_entry(
    title: &str,
    value: &str,
    description: &str,
    placeholder: &str,
    allowed_chars: Option<&str>,
) -> Modal {
    let mut name = TextInput::new()
        .placeholder(placeholder)
        .panel(if allowed_chars.is_some() {
            "Branch"
        } else {
            "Instance"
        })
        .on_change(Msg::NameChanged);
    if let Some(allowed_chars) = allowed_chars {
        name = name.allowed_chars(allowed_chars).max_len(40);
    }
    name.set_value(value);
    name.set_insert_mode(true);
    name.move_cursor_to_end();
    let mut description_input = TextInput::new()
        .placeholder("Description")
        .panel("Description")
        .on_change(Msg::DescriptionChanged);
    description_input.set_value(description);
    Box::new(
        Dialog::new()
            .top_left(title)
            .on_close(|_| Msg::Close)
            .actions([create_new(), cancel()])
            .host(
                Flex::column()
                    .child(
                        "name",
                        name,
                        FlexItem::fit_content().cross_size(CrossSize::Fixed(64)),
                    )
                    .child(
                        "description",
                        description_input,
                        FlexItem::fit_content().cross_size(CrossSize::Fixed(64)),
                    ),
            ),
    )
}

pub(super) fn description_entry(value: &str) -> Modal {
    let mut input = TextInput::new()
        .placeholder("Description")
        .on_change(Msg::DescriptionChanged);
    input.set_value(value);
    input.set_insert_mode(true);
    input.move_cursor_to_end();
    Box::new(
        Dialog::new()
            .top_left("Update description")
            .content_padding(Padding::default())
            .on_close(|_| Msg::Close)
            .actions([confirm("Save", KeySpec::plain('s')), cancel()])
            .host(Flex::column().child(
                "description",
                input,
                FlexItem::fit_content().cross_size(CrossSize::Fixed(64)),
            )),
    )
}

pub(super) fn settings(branch_instances: bool, open_command: &str, close_command: &str) -> Modal {
    let mut input = TextInput::new()
        .placeholder("Empty: open workspace folder")
        .on_change(Msg::OpenCommandChanged);
    input.set_value(open_command);
    input.set_insert_mode(false);
    input.move_cursor_to_end();
    let mut close_input = TextInput::new()
        .placeholder("Empty: no command before workspace deletion")
        .on_change(Msg::CloseCommandChanged);
    close_input.set_value(close_command);
    close_input.set_insert_mode(false);
    close_input.move_cursor_to_end();
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
                    )
                    .child(
                        "close-command",
                        FormField::new("Close command", close_input),
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

pub(super) fn confirm_service_state(
    name: &str,
    service: &str,
    running: bool,
    hotkey: KeySpec,
) -> Modal {
    let action = if running { "Start" } else { "Stop" };
    confirmation(
        &format!("{action} service"),
        action,
        hotkey,
        format!(
            "{action} {name}/{service}?\nData and container configuration stay; other services are untouched."
        ),
    )
}

pub(super) fn confirm_restart(name: &str, service: Option<&str>, hotkey: KeySpec) -> Modal {
    let (title, target) = match service {
        Some(service) => ("Restart service", format!("{name}/{service}")),
        None => ("Restart instance", name.to_owned()),
    };
    confirmation(
        title,
        "Restart",
        hotkey,
        format!(
            "Restart {target}? This briefly interrupts service.\nData and container configuration stay; one-shot setup jobs are skipped."
        ),
    )
}

pub(super) fn confirm_purge(name: &str) -> Modal {
    confirmation(
        "Purge instance",
        "Purge",
        KeySpec::plain('p'),
        format!("Purge {name}? This permanently removes its data."),
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

pub(super) fn confirm_stop_all(count: usize, hotkey: KeySpec) -> Modal {
    confirmation(
        "Stop all instances",
        "Stop all",
        hotkey,
        format!(
            "Stop {count} stoppable instances across all templates?\nWorkspaces and data stay for restart. Templates and the gateway stay."
        ),
    )
}

pub(super) fn confirm_purge_all(count: usize, hotkey: KeySpec) -> Modal {
    confirmation(
        "Purge all instances",
        "Purge all",
        hotkey,
        format!(
            "Permanently purge {count} instances across all templates?\nThis removes their containers, workspaces, volumes, and networks.\nTemplates and the gateway stay. This cannot be undone."
        ),
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
