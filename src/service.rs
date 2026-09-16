use std::{error::Error, sync::Arc, time::Instant};

use schemars::JsonSchema;
use serde::Serialize;

mod environments;

#[derive(Clone)]
pub(crate) struct AppService {
    runtime: Arc<tokio::runtime::Runtime>,
    environments: Arc<crate::environments::Environments>,
    state: Arc<ServiceState>,
}

struct ServiceState {
    started_at: Instant,
    #[cfg(test)]
    _test_home: Option<tempfile::TempDir>,
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
        Ok(Self::from_config(
            crate::environments::config::Config::from_env()?,
        )?)
    }

    fn from_config(config: crate::environments::config::Config) -> Result<Self, std::io::Error> {
        Ok(Self {
            runtime: Arc::new(tokio::runtime::Runtime::new()?),
            environments: Arc::new(crate::environments::Environments::new(config)),
            state: Arc::new(ServiceState {
                started_at: Instant::now(),
                #[cfg(test)]
                _test_home: None,
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
