use std::{error::Error, sync::Arc, time::Instant};

use schemars::JsonSchema;
use serde::Serialize;

mod creation;
#[cfg(debug_assertions)]
mod dev_server;
mod environments;
mod events;
mod instance_mcp;
mod lifecycle;
mod opencode;
mod providers;
mod refresh;
mod rules;
mod settings;
mod startup;

pub(crate) use creation::NewInstanceOutcome;
pub(crate) use environments::CreateInstanceOutcome;

#[derive(Clone)]
pub(crate) struct AppService {
    runtime: Arc<tokio::runtime::Runtime>,
    environments: Arc<crate::environments::Environments>,
    settings: Arc<settings::Settings>,
    refresh: Arc<refresh::RefreshWorker>,
    state: Arc<ServiceState>,
    opencode: Arc<opencode::Integration>,
    sound_choices: Arc<Vec<crate::store::completion::SoundChoice>>,
    events: Arc<events::Integration>,
    providers: Arc<providers::Integration>,
    rules: Arc<rules::Integration>,
    #[cfg(not(test))]
    sound_playback: Arc<SoundPlayback>,
}

#[cfg(not(test))]
#[derive(Default)]
struct SoundPlayback {
    generation: std::sync::atomic::AtomicU64,
    gate: std::sync::Mutex<()>,
}

struct ServiceState {
    started_at: Instant,
    #[cfg(test)]
    _test_home: Option<tempfile::TempDir>,
    #[cfg(test)]
    opened_system_targets: std::sync::Mutex<Vec<String>>,
    #[cfg(test)]
    completion_sounds: std::sync::Mutex<Vec<String>>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub(crate) struct ServiceStatus {
    pub name: &'static str,
    pub uptime_seconds: u64,
    pub templates_root: String,
    pub gateway_origin: String,
}

impl AppService {
    pub(crate) fn initialize() -> Result<Self, Box<dyn Error>> {
        let config = crate::environments::config::Config::from_env()?;
        crate::environments::runtime_db::prepare(&config)?;
        Self::from_config(config)
    }

    fn from_config(config: crate::environments::config::Config) -> Result<Self, Box<dyn Error>> {
        let events = Arc::new(events::Integration::new(&config)?);
        let providers = Arc::new(providers::Integration::new(&config)?);
        let rules = Arc::new(rules::Integration::new(&config)?);
        let settings = Arc::new(settings::Settings::open(
            config.home.join("settings.sqlite3"),
        )?);
        let environments = Arc::new(crate::environments::Environments::new(config));
        let refresh = Arc::new(refresh::RefreshWorker::start(
            Arc::clone(&environments),
            Arc::clone(&settings),
        )?);
        Ok(Self {
            runtime: Arc::new(tokio::runtime::Runtime::new()?),
            settings,
            environments,
            refresh,
            opencode: Arc::new(opencode::Integration::new()),
            sound_choices: Arc::new(crate::environments::sound::available()),
            events,
            providers,
            rules,
            #[cfg(not(test))]
            sound_playback: Arc::new(SoundPlayback::default()),
            state: Arc::new(ServiceState {
                started_at: Instant::now(),
                #[cfg(test)]
                _test_home: None,
                #[cfg(test)]
                opened_system_targets: std::sync::Mutex::new(Vec::new()),
                #[cfg(test)]
                completion_sounds: std::sync::Mutex::new(Vec::new()),
            }),
        })
    }

    #[cfg(test)]
    pub(crate) fn for_tests() -> Self {
        let directory = tempfile::tempdir().unwrap();
        let config = crate::environments::config::Config::at(
            directory.path().to_path_buf(),
            "tandem-test".into(),
            9876,
        )
        .unwrap();
        let mut service = Self::from_config(config).unwrap();
        Arc::get_mut(&mut service.state).unwrap()._test_home = Some(directory);
        service.set_opencode_snapshot_for_tests(Default::default());
        service
    }

    #[cfg(test)]
    pub(crate) fn config_for_tests(&self) -> crate::environments::config::Config {
        self.environments.config.clone()
    }

    pub(crate) fn status(&self) -> ServiceStatus {
        ServiceStatus {
            name: "Tandem",
            uptime_seconds: self.state.started_at.elapsed().as_secs(),
            templates_root: self.environments.config.templates.display().to_string(),
            gateway_origin: self.environments.config.origin(),
        }
    }

    #[cfg(debug_assertions)]
    pub(crate) fn replace_dev_server(
        &self,
        port: u16,
    ) -> Result<dev_server::DevServerLease, String> {
        dev_server::DevServerLease::replace(&self.environments.config.home, port)
    }

    pub(crate) fn play_completion_sound(&self) {
        self.play_selected_sound(self.completion_sound_choice());
    }

    pub(crate) fn play_event_acceptance_sound(&self) {
        self.play_selected_sound(self.event_acceptance_sound_choice());
    }

    fn play_selected_sound(&self, choice: String) {
        let choice = if self.sound_choices.iter().any(|sound| sound.id == choice) {
            choice
        } else {
            String::new()
        };
        self.play_sound(choice);
    }

    pub(crate) fn completion_sound_choices(&self) -> Vec<crate::store::completion::SoundChoice> {
        self.sound_choices.as_ref().clone()
    }

    fn play_sound(&self, selection: String) {
        #[cfg(test)]
        {
            self.state.completion_sounds.lock().unwrap().push(selection);
        }
        #[cfg(not(test))]
        {
            use std::sync::atomic::Ordering;
            let playback = Arc::clone(&self.sound_playback);
            let generation = playback.generation.fetch_add(1, Ordering::AcqRel) + 1;
            self.runtime.spawn_blocking(move || {
                let _guard = playback
                    .gate
                    .lock()
                    .unwrap_or_else(|error| error.into_inner());
                if let Err(error) = crate::environments::sound::play_completion(&selection, || {
                    playback.generation.load(Ordering::Acquire) != generation
                }) {
                    crate::diagnostics::record_error(
                        "could not play notification sound",
                        &std::io::Error::other(error),
                    );
                }
            });
        }
    }

    #[cfg(test)]
    pub(crate) fn set_sound_choices_for_tests(
        &mut self,
        choices: Vec<crate::store::completion::SoundChoice>,
    ) {
        self.sound_choices = Arc::new(choices);
    }

    #[cfg(test)]
    pub(crate) fn completion_sound_count_for_tests(&self) -> usize {
        self.state.completion_sounds.lock().unwrap().len()
    }
}

#[cfg(test)]
mod tests;
