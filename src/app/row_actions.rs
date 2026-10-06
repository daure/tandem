use super::Msg;
use crate::store::{
    events::{Deletion, Record},
    providers::{Action, Provider, Stream},
    rules::Rule,
};

#[derive(Debug)]
pub(crate) enum Target {
    Provider(Box<Provider>),
    Stream(Box<(Provider, Stream)>),
    Event(Box<Record>),
    Rule(Box<Rule>),
    AcceptanceContext(Box<super::acceptances::Target>),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum Command {
    Details,
    OpenLink,
    Start,
    Stop,
    Logs,
    Replay,
    Delete,
    Provider,
    Activate,
    Deactivate,
    ProviderDetails,
    StreamDetails,
    StartStream,
    StopStream,
    StreamEvents,
    Instance,
    Session,
    CreateInstance,
    NewSession,
    PurgeInstance,
    Rule,
    Routes,
    Report,
    RenameSession,
    DeleteSession,
    ClosePanel,
}

impl Command {
    pub(super) fn label(self) -> &'static str {
        match self {
            Self::Details => "View details",
            Self::OpenLink => "Open link",
            Self::Start => "Start provider",
            Self::Stop => "Stop provider",
            Self::StartStream => "Start stream",
            Self::StopStream => "Stop stream",
            Self::ProviderDetails => "Provider details",
            Self::StreamDetails => "Stream details",
            Self::StreamEvents => "Stream events",
            Self::Logs => "Logs",
            Self::Replay => "Replay",
            Self::Delete => "Delete",
            Self::Provider => "Go to provider",
            Self::Activate => "Activate",
            Self::Deactivate => "Deactivate",
            Self::Instance => "Go to instance",
            Self::Session => "Go to OpenCode session",
            Self::CreateInstance => "Recreate instance",
            Self::NewSession => "New OpenCode session",
            Self::PurgeInstance => "Purge instance",
            Self::Rule => "Go to rule",
            Self::Routes => "Open routes",
            Self::Report => "View report",
            Self::RenameSession => "Rename OpenCode session",
            Self::DeleteSession => "Delete OpenCode session",
            Self::ClosePanel => "Close OpenCode panel",
        }
    }

    pub(super) fn hotkey(self) -> &'static str {
        match self {
            Self::Details | Self::ProviderDetails | Self::StreamDetails => "d",
            Self::Start | Self::Stop | Self::StartStream | Self::StopStream => "s",
            Self::Provider => "p",
            Self::Logs | Self::OpenLink => "Enter",
            Self::StreamEvents => "e",
            Self::Replay => "r",
            Self::Delete => "x",
            Self::Activate | Self::Deactivate => "a",
            Self::Instance => "v",
            Self::Session => "Enter",
            Self::CreateInstance => "i",
            Self::NewSession => "n",
            Self::PurgeInstance => "p",
            Self::Rule => "r",
            Self::RenameSession => "r",
            Self::DeleteSession => "x",
            Self::ClosePanel => "c",
            Self::Routes => "Enter",
            Self::Report => "f",
        }
    }
}

impl Target {
    pub(super) fn enter_command(&self) -> Command {
        match self {
            Self::Provider(_) => Command::Logs,
            Self::Stream(_) => Command::StreamEvents,
            Self::Event(_) => Command::OpenLink,
            Self::Rule(_) => Command::Details,
            Self::AcceptanceContext(target) => {
                if target.selected.is_some() {
                    Command::Session
                } else {
                    Command::Routes
                }
            }
        }
    }

    pub(super) fn enter_message(self) -> Option<Msg> {
        let command = self.enter_command();
        self.message(command)
    }

    pub(super) fn commands(&self) -> Vec<Command> {
        match self {
            Self::Provider(_) => vec![
                Command::ProviderDetails,
                Command::Start,
                Command::Stop,
                Command::Logs,
            ],
            Self::Stream(_) => vec![
                Command::StreamDetails,
                Command::StartStream,
                Command::StopStream,
                Command::StreamEvents,
            ],
            Self::Event(_) => vec![
                Command::OpenLink,
                Command::Details,
                Command::Replay,
                Command::Provider,
                Command::Delete,
            ],
            Self::Rule(rule) => vec![
                Command::Details,
                if rule.definition.enabled {
                    Command::Deactivate
                } else {
                    Command::Activate
                },
            ],
            Self::AcceptanceContext(target) => {
                let mut commands = vec![
                    Command::Rule,
                    Command::Report,
                    Command::CreateInstance,
                    Command::Instance,
                    Command::Routes,
                    Command::NewSession,
                    Command::PurgeInstance,
                ];
                if target.selected.is_some() {
                    commands.insert(0, Command::Details);
                    commands.push(Command::Session);
                    commands.push(Command::ClosePanel);
                } else {
                    commands.push(Command::Delete);
                }
                if target.selected.as_ref().is_some_and(|row| {
                    matches!(row.opencode, Some(super::opencode::Target::Session { .. }))
                }) {
                    commands.retain(|command| *command != Command::Rule);
                    commands.extend([Command::RenameSession, Command::DeleteSession]);
                }
                commands
            }
        }
    }

    pub(super) fn enabled(&self, command: Command) -> bool {
        match (self, command) {
            (Self::AcceptanceContext(target), command) => target.enabled(command),
            (Self::Stream(target), Command::StreamEvents) => target.0.manifest.is_some(),
            (Self::Stream(target), command) => command
                .provider_action()
                .is_none_or(|action| target.1.action_unavailable(&target.0, action).is_none()),
            (Self::Provider(provider), command) => command
                .provider_action()
                .is_none_or(|action| provider.action_unavailable(action).is_none()),
            (Self::Rule(rule), Command::Activate) => !rule.definition.enabled,
            (Self::Rule(rule), Command::Deactivate) => rule.definition.enabled,
            _ => true,
        }
    }

    pub(super) fn message(self, command: Command) -> Option<Msg> {
        match self {
            Self::Provider(provider) => {
                if matches!(command, Command::Details | Command::ProviderDetails) {
                    return Some(Msg::ProviderDetails(provider));
                }
                let action = command.provider_action()?;
                Some(Msg::ProviderAction(provider.name, action))
            }
            Self::Stream(target) => {
                let (provider, stream) = *target;
                match command {
                    Command::StreamDetails => {
                        Some(Msg::ProviderStreamDetails(provider.name, Box::new(stream)))
                    }
                    Command::StreamEvents => provider
                        .manifest
                        .map(|manifest| Msg::ProviderStreamEvents(manifest.name, stream.name)),
                    _ => Some(Msg::ProviderStreamAction(
                        provider.name,
                        stream.name,
                        command.provider_action()?,
                    )),
                }
            }
            Self::Event(row) => match command {
                Command::Details => Some(Msg::OpenEvent(row)),
                Command::OpenLink => Some(Msg::OpenEventLink(row)),
                Command::Replay => Some(Msg::ReplayEvent(row.sequence)),
                Command::Delete => Some(Msg::DeleteEvents(Deletion::Event(row.sequence))),
                Command::Provider => Some(Msg::ShowProvider(row.provider.clone())),
                _ => None,
            },
            Self::Rule(mut rule) => match command {
                Command::Details => Some(Msg::OpenRule(rule)),
                Command::Activate | Command::Deactivate => {
                    rule.definition.enabled = command == Command::Activate;
                    Some(Msg::SaveRule(rule))
                }
                _ => None,
            },
            Self::AcceptanceContext(target) => {
                if matches!(command, Command::RenameSession | Command::DeleteSession) {
                    let Some(super::opencode::Target::Session { id, .. }) = target
                        .selected
                        .as_ref()
                        .and_then(|row| row.opencode.as_ref())
                    else {
                        return None;
                    };
                    let action = if command == Command::RenameSession {
                        super::opencode::SessionAction::Rename
                    } else {
                        super::opencode::SessionAction::Delete
                    };
                    Some(Msg::OpencodeSessionAction(id.clone(), action))
                } else {
                    Some(Msg::AcceptanceAction(target, command))
                }
            }
        }
    }
}

impl Command {
    fn provider_action(self) -> Option<Action> {
        match self {
            Self::Start | Self::StartStream => Some(Action::Start),
            Self::Stop | Self::StopStream => Some(Action::Stop),
            Self::Logs => Some(Action::Logs),
            _ => None,
        }
    }
}
