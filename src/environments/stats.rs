use std::{env, io::Read, time::Instant};

use reqwest::blocking::Client;
use serde::de::DeserializeOwned;

use super::command::{docker, remaining, run};

pub(super) struct StatsClient {
    client: Client,
    base: String,
}

impl StatsClient {
    pub(super) fn connect(deadline: Instant) -> Result<Self, String> {
        let context = env::var("DOCKER_CONTEXT")
            .ok()
            .filter(|value| !value.is_empty());
        let host = env::var("DOCKER_HOST")
            .ok()
            .filter(|value| !value.is_empty());
        let endpoint = match (context, host) {
            (None, Some(host)) => host,
            (context, _) => {
                let mut command = docker();
                command.args([
                    "context",
                    "inspect",
                    "--format",
                    "{{.Endpoints.docker.Host}}",
                ]);
                if let Some(context) = context {
                    command.arg(context);
                }
                run(command, remaining(deadline)?, None)?.trim().to_owned()
            }
        };
        let socket = endpoint
            .strip_prefix("unix://")
            .filter(|path| !path.is_empty())
            .ok_or("resource sampling requires a local Docker Unix socket")?;
        Self::from_socket(socket, deadline)
    }

    fn from_socket(socket: &str, deadline: Instant) -> Result<Self, String> {
        let client = Client::builder()
            .unix_socket(socket)
            .no_proxy()
            .timeout(remaining(deadline)?)
            .build()
            .map_err(|error| error.to_string())?;
        let mut api = Self {
            client,
            base: "http://localhost".into(),
        };
        #[derive(serde::Deserialize)]
        struct Version {
            #[serde(rename = "ApiVersion")]
            api_version: String,
        }
        let version: Version = api.get("/version", deadline)?;
        let (major, minor) = version
            .api_version
            .split_once('.')
            .ok_or("invalid Docker API version")?;
        if major != "1"
            || minor
                .parse::<u32>()
                .map_err(|_| "invalid Docker API version")?
                < 41
        {
            return Err("resource sampling requires Docker API 1.41 or newer".into());
        }
        api.base = format!("http://localhost/v{}", version.api_version);
        Ok(api)
    }

    pub(super) fn sample<T: DeserializeOwned>(
        &self,
        id: &str,
        deadline: Instant,
    ) -> Result<T, String> {
        if id.is_empty() || !id.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err("invalid container ID for resource sampling".into());
        }
        self.get(
            &format!("/containers/{id}/stats?stream=false&one-shot=true"),
            deadline,
        )
    }

    fn get<T: DeserializeOwned>(&self, path: &str, deadline: Instant) -> Result<T, String> {
        let response = self
            .client
            .get(format!("{}{path}", self.base))
            .timeout(remaining(deadline)?)
            .send()
            .and_then(|response| response.error_for_status())
            .map_err(|error| format!("Docker stats request: {error}"))?;
        serde_json::from_reader(response.take(1024 * 1024))
            .map_err(|error| format!("invalid Docker stats response: {error}"))
    }
}

#[cfg(test)]
#[path = "tests/stats.rs"]
mod tests;
