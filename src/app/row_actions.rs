use super::Msg;
use crate::store::{
    events::Record,
    providers::{Action, Provider},
};

#[derive(Debug)]
pub(crate) enum Target {
    Provider(Box<Provider>),
    Event(Box<Record>),
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub(super) enum Command {
    Details,
    Start,
    Stop,
    Pause,
    Resume,
    Restart,
    Logs,
    Events,
    Replay,
    Provider,
}

impl Command {
    pub(super) fn label(self) -> &'static str {
        match self {
            Self::Details => "View details",
            Self::Start => "Start",
            Self::Stop => "Stop",
            Self::Pause => "Pause",
            Self::Resume => "Resume",
            Self::Restart => "Restart",
            Self::Logs => "Logs",
            Self::Events => "Events",
            Self::Replay => "Replay",
            Self::Provider => "Go to provider",
        }
    }

    pub(super) fn hotkey(self) -> &'static str {
        match self {
            Self::Details => "Enter",
            Self::Start | Self::Stop => "s",
            Self::Pause | Self::Resume => "a",
            Self::Provider => "p",
            Self::Logs => "l",
            Self::Events => "e",
            Self::Replay | Self::Restart => "r",
        }
    }
}

impl Target {
    pub(super) fn commands(&self) -> Vec<Command> {
        match self {
            Self::Provider(_) => vec![
                Command::Details,
                Command::Start,
                Command::Stop,
                Command::Pause,
                Command::Resume,
                Command::Restart,
                Command::Logs,
                Command::Events,
            ],
            Self::Event(_) => vec![Command::Details, Command::Replay, Command::Provider],
        }
    }

    pub(super) fn enabled(&self, command: Command) -> bool {
        match (self, command) {
            (Self::Provider(provider), Command::Events) => provider.manifest.is_some(),
            (Self::Provider(provider), command) => command
                .provider_action()
                .is_none_or(|action| provider.action_unavailable(action).is_none()),
            _ => true,
        }
    }

    pub(super) fn message(self, command: Command) -> Option<Msg> {
        match self {
            Self::Provider(provider) => {
                match command {
                    Command::Details => return Some(Msg::ProviderDetails(provider)),
                    Command::Events => {
                        return provider
                            .manifest
                            .as_ref()
                            .map(|manifest| Msg::ProviderEvents(manifest.name.clone()));
                    }
                    _ => {}
                }
                let action = command.provider_action()?;
                Some(Msg::ProviderAction(provider.name, action))
            }
            Self::Event(row) => match command {
                Command::Details => Some(Msg::OpenEvent(row)),
                Command::Replay => Some(Msg::ReplayEvent(row.sequence)),
                Command::Provider => Some(Msg::ShowProvider(row.provider.clone())),
                _ => None,
            },
        }
    }
}

impl Command {
    fn provider_action(self) -> Option<Action> {
        match self {
            Self::Start => Some(Action::Start),
            Self::Stop => Some(Action::Stop),
            Self::Pause => Some(Action::Pause),
            Self::Resume => Some(Action::Resume),
            Self::Restart => Some(Action::Restart),
            Self::Logs => Some(Action::Logs),
            _ => None,
        }
    }
}
