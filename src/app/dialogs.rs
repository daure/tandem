use ratatui::widgets::Borders;
use std::{cell::RefCell, path::Path, rc::Rc};
use tuicore::{
    CrossSize, DataView, Dialog, DialogAction, DialogKeyBindings, Dropdown, DropdownCommitMode,
    Flex, FlexItem, FormField, KeySpec, Language, Padding, Paragraph, SyntaxHighlighter, Tab, Tabs,
    TabsVariant, TextInput, TextareaInput, Toggle, TreeAdapter,
};

use super::{Modal, Msg, properties::Properties, rows::Row};
use crate::store::environments::TemplateFile;

pub(super) const COMPACT_WIDTH: u16 = 60;

pub(super) fn dialog(title: &str) -> Dialog<Msg> {
    Dialog::new()
        .top_left(title)
        .keybindings(DialogKeyBindings {
            close: vec![KeySpec::plain('x')],
        })
        .on_close(|_| Msg::Close)
}

fn confirm() -> DialogAction<Msg> {
    DialogAction::new("Ok")
        .hotkey(KeySpec::plain('o'))
        .on_trigger(|| Msg::Submit)
}

fn cancel() -> DialogAction<Msg> {
    DialogAction::new("Cancel")
        .hotkey(KeySpec::plain('c'))
        .on_trigger(|| Msg::Close)
}

pub(super) fn details(row: &Row) -> Modal {
    Box::new(details_tabs(row))
}

fn details_tabs(row: &Row) -> Tabs<Msg> {
    let title = if row.is_template() {
        "Template details"
    } else {
        "Details"
    };
    let mut tabs = vec![Tab::new(title, Properties::new(row.details.clone()))];
    if row.is_template() {
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
        if row.files.iter().any(|file| !file.directory) {
            tabs.push(Tab::new("Files", files_tree(&row.files)));
        }
    }
    Tabs::dialog(tabs)
        .variant(TabsVariant::OneRow)
        .edge_borders(Borders::TOP)
        .on_close(|_| Msg::Close)
}

fn files_tree(files: &[TemplateFile]) -> DataView<TemplateFile, String> {
    DataView::list(
        files.to_vec(),
        |file: &TemplateFile| file.path.clone(),
        |file| {
            let name = Path::new(&file.path)
                .file_name()
                .unwrap_or_default()
                .to_string_lossy();
            if file.directory {
                format!("{name}/")
            } else {
                name.into_owned()
            }
        },
    )
    .headers(false)
    .action_bar(false)
    .filter_controls(false)
    .tree(TreeAdapter::parent_id(|file: &TemplateFile| {
        Path::new(&file.path)
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .map(|parent| parent.to_string_lossy().into_owned())
    }))
    .expanded(
        files
            .iter()
            .filter(|file| file.directory)
            .map(|file| file.path.clone()),
    )
}

pub(super) fn updated_details(selected_index: usize, previous: &Row, row: &Row) -> Modal {
    fn tab_names(row: &Row) -> Vec<&'static str> {
        let mut names = vec!["Details"];
        if !row.compose_file.is_empty() {
            names.push("Compose");
        }
        names.push("Manifest");
        if row.guidance_source.is_some() {
            names.push("Guidance");
        }
        if row.files.iter().any(|file| !file.directory) {
            names.push("Files");
        }
        names
    }
    let previous_tabs = tab_names(previous);
    let selected = previous_tabs.get(selected_index);
    let index = tab_names(row)
        .iter()
        .position(|name| Some(name) == selected)
        .unwrap_or(0);
    Box::new(details_tabs(row).selected(index))
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
        dialog(title)
            .actions([confirm(), cancel()])
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
    creation: &super::creation::Creation,
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
    let mut description_input = TextareaInput::new()
        .placeholder("Description")
        .panel("Description")
        .min_rows(2)
        .max_rows(8)
        .value(description)
        .on_change(Msg::DescriptionChanged);
    description_input.move_cursor_to_end();
    let mut prompt = TextareaInput::new()
        .placeholder("Optional; a prompt opens OpenCode automatically")
        .panel("Initial prompt")
        .min_rows(2)
        .max_rows(8)
        .value(&creation.prompt)
        .on_change(Msg::InitialPromptChanged);
    prompt.move_cursor_to_end();
    Box::new(
        dialog(title).actions([confirm(), cancel()]).host(
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
                )
                .child(
                    "initial-prompt",
                    prompt,
                    FlexItem::fit_content().cross_size(CrossSize::Fixed(64)),
                )
                .child(
                    "start-instance",
                    Toggle::new("Start instance")
                        .checked(creation.start_instance)
                        .disabled(!creation.can_start_instance)
                        .on_change(Msg::StartInstanceChanged),
                    FlexItem::fit_content(),
                ),
        ),
    )
}

pub(super) fn description_entry(value: &str) -> Modal {
    let mut input = TextareaInput::new()
        .placeholder("Description")
        .min_rows(2)
        .max_rows(8)
        .on_change(Msg::DescriptionChanged);
    input.set_value(value);
    input.set_insert_mode(true);
    input.move_cursor_to_end();
    Box::new(
        dialog("Edit description")
            .content_padding(Padding::default())
            .actions([confirm(), cancel()])
            .host(Flex::column().child(
                "description",
                input,
                FlexItem::fit_content().cross_size(CrossSize::Fixed(64)),
            )),
    )
}

pub(super) fn rename_opencode_session(value: &str) -> Modal {
    let mut input = TextInput::new()
        .style(tuicore::InputChrome::plain())
        .value(value)
        .max_len(1024)
        .on_change(Msg::NameChanged);
    input.set_insert_mode(true);
    input.move_cursor_to_end();
    Box::new(
        dialog("Rename OpenCode session")
            .actions([confirm(), cancel()])
            .host(Flex::column().child(
                "name",
                input,
                FlexItem::fit_content().cross_size(CrossSize::Fixed(52)),
            )),
    )
}

pub(super) fn delete_opencode_session(title: &str) -> Modal {
    Box::new(dialog("Delete OpenCode session")
        .actions([confirm(), cancel()])
        .host(Flex::column().child("warning", Paragraph::new(format!(
            "Delete {title} and its child sessions?\nActive sessions will close and stop. This cannot be undone."
        )), FlexItem::fit_content())))
}

pub(super) fn clear_opencode_folder(directory: &str) -> Modal {
    Box::new(dialog("Clear OpenCode data")
        .actions([confirm(), cancel()])
        .host(Flex::column().child("warning", Paragraph::new(format!(
            "Delete all sessions for {directory}?\nFolder files stay untouched. This cannot be undone."
        )), FlexItem::fit_content())))
}

pub(super) fn settings(
    branch_instances: bool,
    opencode: bool,
    clear_opencode_history: bool,
    fade_seconds: u64,
    sounds: Vec<crate::store::completion::SoundChoice>,
    selected_sounds: [&str; 2],
    sound_choice: Rc<RefCell<Option<Msg>>>,
) -> Modal {
    let mut input = TextInput::new()
        .numbers_only(true)
        .max_len(4)
        .on_change(Msg::CompletionFadeChanged);
    input.set_value(fade_seconds.to_string());
    input.set_insert_mode(false);
    input.move_cursor_to_end();
    let sound = sound_dropdown(
        "Completion sound",
        sounds.clone(),
        selected_sounds[0],
        sound_choice.clone(),
        Msg::CompletionSoundSelected,
    );
    let event_sound = sound_dropdown(
        "Event acceptance sound",
        sounds,
        selected_sounds[1],
        sound_choice,
        Msg::EventAcceptanceSoundSelected,
    );
    Box::new(
        dialog("Settings")
            .actions([DialogAction::new("Close")
                .hotkey(KeySpec::plain('x'))
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
                        "opencode-integration",
                        Toggle::new("Enable opencode integration")
                            .checked(opencode)
                            .on_change(Msg::SetOpencodeIntegration),
                        FlexItem::content(),
                    )
                    .child(
                        "clear-opencode-history",
                        Toggle::new("Clear OpenCode history on new instance")
                            .checked(clear_opencode_history)
                            .on_change(Msg::SetClearOpencodeHistory),
                        FlexItem::content(),
                    )
                    .child(
                        "completion-fade",
                        FormField::new("Completion fade duration", input),
                        FlexItem::fit_content().cross_size(CrossSize::Fixed(72)),
                    )
                    .child(
                        "completion-sound",
                        sound,
                        FlexItem::fit_content().cross_size(CrossSize::Fixed(72)),
                    )
                    .child(
                        "event-acceptance-sound",
                        event_sound,
                        FlexItem::fit_content().cross_size(CrossSize::Fixed(72)),
                    ),
            ),
    )
}

fn sound_dropdown(
    label: &str,
    mut sounds: Vec<crate::store::completion::SoundChoice>,
    selected_sound: &str,
    sound_choice: Rc<RefCell<Option<Msg>>>,
    selected_message: fn(String) -> Msg,
) -> Dropdown<crate::store::completion::SoundChoice, String> {
    if !sounds.iter().any(|sound| sound.id == selected_sound) {
        sounds.push(crate::store::completion::SoundChoice {
            id: selected_sound.into(),
            label: format!("Unavailable: {selected_sound} (using system default)"),
        });
    }
    Dropdown::single(
        sounds,
        |sound| sound.id.clone(),
        |sound| sound.label.clone(),
    )
    .variant(tuicore::DropdownVariant::Bordered)
    .label(label)
    .selected_one(selected_sound.to_owned())
    .commit_mode(DropdownCommitMode::Explicit)
    .max_popup_height(12)
    .on_select(move |selected| {
        *sound_choice.borrow_mut() = selected.into_iter().next().map(selected_message);
    })
}

pub(super) fn confirm_stop(name: &str) -> Modal {
    confirmation("Stop instance", format!("Stop {name}?"))
}

pub(super) fn confirm_start(name: &str) -> Modal {
    confirmation("Start instance", format!("Start {name}?"))
}

pub(super) fn confirm_close_opencode_sessions(name: &str) -> Modal {
    confirmation(
        "Close OpenCode sessions",
        format!("Close all panes for {name}?"),
    )
}

pub(super) fn confirm_service_state(name: &str, service: &str, running: bool) -> Modal {
    let action = if running { "Start" } else { "Stop" };
    confirmation(
        &format!("{action} service"),
        format!("{action} {name}/{service}?"),
    )
}

pub(super) fn confirm_restart(name: &str, service: Option<&str>) -> Modal {
    let (title, target) = match service {
        Some(service) => ("Restart service", format!("{name}/{service}")),
        None => ("Restart instance", name.to_owned()),
    };
    confirmation(title, format!("Restart {target}?"))
}

pub(super) fn confirm_purge(name: &str) -> Modal {
    confirmation(
        "Purge instance",
        format!("Permanently delete {name} and its data?"),
    )
}

pub(super) fn confirm_stop_template(name: &str) -> Modal {
    confirmation(
        "Stop all instances",
        format!("Stop all instances of {name}?"),
    )
}

pub(super) fn confirm_delete_template(name: &str) -> Modal {
    confirmation(
        "Purge all instances",
        format!("Permanently delete all instances of {name} and their data?"),
    )
}

pub(super) fn confirmation(title: &str, description: String) -> Modal {
    Box::new(
        dialog(title)
            .actions([confirm(), cancel()])
            .host(Flex::column().child(
                "warning",
                Paragraph::new(description),
                FlexItem::fit_content(),
            )),
    )
}

pub(super) fn confirm_stop_all(count: usize) -> Modal {
    confirmation(
        "Stop all instances",
        format!("Stop {count} stoppable instances across all templates?"),
    )
}

pub(super) fn confirm_purge_all(count: usize) -> Modal {
    confirmation(
        "Purge all instances",
        format!("Permanently delete {count} instances across all templates and their data?"),
    )
}

pub(super) fn confirm_remove_template(name: &str, directory: &str) -> Modal {
    confirmation(
        "Delete template",
        format!("Permanently delete {name}, its files, instances, and data?\n{directory}"),
    )
}
