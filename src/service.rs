use std::{error::Error, sync::Arc, time::Instant};

use schemars::JsonSchema;
use serde::Serialize;

mod environments;
mod refresh;
mod settings;

pub(crate) use environments::CreateInstanceOutcome;

#[derive(Clone)]
pub(crate) struct AppService {
    runtime: Arc<tokio::runtime::Runtime>,
    environments: Arc<crate::environments::Environments>,
    settings: Arc<settings::Settings>,
    refresh: Arc<refresh::RefreshWorker>,
    state: Arc<ServiceState>,
}

struct ServiceState {
    started_at: Instant,
    #[cfg(test)]
    _test_home: Option<tempfile::TempDir>,
    #[cfg(test)]
    opened_system_targets: std::sync::Mutex<Vec<String>>,
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
        Self::from_config(crate::environments::config::Config::from_env()?)
    }

    fn from_config(config: crate::environments::config::Config) -> Result<Self, Box<dyn Error>> {
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
            state: Arc::new(ServiceState {
                started_at: Instant::now(),
                #[cfg(test)]
                _test_home: None,
                #[cfg(test)]
                opened_system_targets: std::sync::Mutex::new(Vec::new()),
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
        service
    }

    pub(crate) fn status(&self) -> ServiceStatus {
        ServiceStatus {
            name: "Tandem",
            uptime_seconds: self.state.started_at.elapsed().as_secs(),
            templates_root: self.environments.config.templates.display().to_string(),
            gateway_origin: self.environments.config.origin(),
        }
    }
}

#[cfg(test)]
mod tests;
