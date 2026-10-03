use super::Msg;
use crate::store::{
    events::Record,
    providers::{Action, Provider, Stream},
    rules::{Acceptance, Rule},
};

#[derive(Debug)]
pub(crate) enum Target {
    Provider(Box<Provider>),
    Stream(Box<(Provider, Stream)>),
    Event(Box<Record>),
    Rule(Box<Rule>),
    Acceptance(Box<Acceptance>),
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub(super) enum Command {
    Details,
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
}

impl Command {
    pub(super) fn label(self) -> &'static str {
        match self {
            Self::Details => "View details",
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
        }
    }

    pub(super) fn hotkey(self) -> &'static str {
        match self {
            Self::Details | Self::ProviderDetails | Self::StreamDetails => "Enter",
            Self::Start | Self::Stop | Self::StartStream | Self::StopStream => "s",
            Self::Provider => "p",
            Self::Logs => "o",
            Self::StreamEvents => "e",
            Self::Replay => "r",
            Self::Delete => "x",
            Self::Activate | Self::Deactivate => "a",
            Self::Instance => "i",
            Self::Session => "o",
        }
    }
}

impl Target {
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
            Self::Acceptance(_) => vec![Command::Instance, Command::Session],
        }
    }

    pub(super) fn enabled(&self, command: Command) -> bool {
        match (self, command) {
            (Self::Stream(target), Command::StreamEvents) => target.0.manifest.is_some(),
            (Self::Stream(target), command) => command
                .provider_action()
                .is_none_or(|action| target.1.action_unavailable(&target.0, action).is_none()),
            (Self::Provider(provider), command) => command
                .provider_action()
                .is_none_or(|action| provider.action_unavailable(action).is_none()),
            (Self::Rule(rule), Command::Activate) => !rule.definition.enabled,
            (Self::Rule(rule), Command::Deactivate) => rule.definition.enabled,
            (Self::Acceptance(row), Command::Session) => {
                row.session_id.is_some() || row.pane.is_some()
            }
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
                Command::Replay => Some(Msg::ReplayEvent(row.sequence)),
                Command::Delete => Some(Msg::DeleteEvents(Some(row.sequence))),
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
            Self::Acceptance(row) => match command {
                Command::Instance => Some(Msg::AcceptanceInstance(row.instance.clone())),
                Command::Session => Some(Msg::AcceptanceSession(row)),
                _ => None,
            },
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
