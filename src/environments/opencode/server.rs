use std::{
    collections::{BTreeMap, BTreeSet},
    time::Duration,
};

use super::{
    RemoteQuestion, RemoteSession, RemoteStatus, SESSION_DIRECTORY_WINDOW, transport, valid_id,
};
use crate::store::opencode::observation::Failure;

#[derive(Default)]
pub(super) struct Observation {
    pub statuses: BTreeMap<String, RemoteStatus>,
    pub questions: BTreeSet<String>,
    pub approvals: BTreeSet<String>,
    pub approval_directories: BTreeSet<String>,
    pub failed_status: BTreeSet<String>,
    pub failed_history: BTreeSet<String>,
    pub sessions: BTreeMap<String, RemoteSession>,
    pub deleted: BTreeSet<String>,
    pub errors: Vec<Failure>,
}

pub(super) async fn observe(
    client: reqwest::Client,
    server: &str,
    directories: &BTreeSet<String>,
    mut candidates: BTreeSet<String>,
) -> Observation {
    let mut observation = Observation {
        failed_status: directories.clone(),
        failed_history: directories.clone(),
        ..Default::default()
    };
    let result = tokio::time::timeout(Duration::from_secs(2), async {
        let mut status_tasks = tokio::task::JoinSet::new();
        for directory in directories {
            let mut path = reqwest::Url::parse("http://localhost/session/status")
                .map_err(|error| error.to_string())?;
            path.query_pairs_mut().append_pair("directory", directory);
            let target = format!("{}?{}", path.path(), path.query().unwrap_or_default());
            let questions = format!("/question?{}", path.query().unwrap_or_default());
            let permissions = format!("/permission?{}", path.query().unwrap_or_default());
            let client = client.clone();
            let server = server.to_owned();
            let directory = directory.clone();
            status_tasks.spawn(async move {
                let (result, permissions) = tokio::join!(
                    async {
                        tokio::try_join!(
                            transport::get::<BTreeMap<String, RemoteStatus>>(
                                &client, &server, &target,
                            ),
                            transport::get::<Vec<RemoteQuestion>>(&client, &server, &questions),
                        )
                    },
                    transport::get_optional::<Vec<RemoteQuestion>>(&client, &server, &permissions),
                );
                (directory, result, permissions)
            });
        }
        while let Some(result) = status_tasks.join_next().await {
            let (directory, result, permissions) = result.map_err(|error| error.to_string())?;
            if let Ok(Some(permissions)) = permissions {
                observation.approval_directories.insert(directory.clone());
                observation
                    .approvals
                    .extend(permissions.into_iter().map(|request| request.session_id));
            }
            match result {
                Ok((statuses, questions)) => {
                    observation.failed_status.remove(&directory);
                    observation.statuses.extend(statuses);
                    observation
                        .questions
                        .extend(questions.into_iter().map(|question| question.session_id));
                }
                Err(error) => {
                    observation.errors.push(Failure::directory(
                        server,
                        &directory,
                        format!("{directory}: OpenCode observation unavailable ({error})"),
                    ));
                }
            }
        }
        if observation.failed_status.len() == directories.len() {
            return Ok::<(), String>(());
        }

        let mut history_tasks = tokio::task::JoinSet::new();
        for directory in directories {
            let mut path = reqwest::Url::parse("http://localhost/experimental/session")
                .map_err(|error| error.to_string())?;
            path.query_pairs_mut()
                .append_pair("roots", "true")
                .append_pair("limit", &SESSION_DIRECTORY_WINDOW.to_string())
                .append_pair("directory", directory);
            let target = format!("{}?{}", path.path(), path.query().unwrap_or_default());
            let client = client.clone();
            let server = server.to_owned();
            let directory = directory.clone();
            history_tasks.spawn(async move {
                let result = transport::get::<Vec<RemoteSession>>(&client, &server, &target).await;
                (directory, result)
            });
        }
        while let Some(result) = history_tasks.join_next().await {
            match result {
                Ok((directory, Ok(sessions))) => {
                    observation.failed_history.remove(&directory);
                    for session in sessions {
                        observation.sessions.insert(session.id.clone(), session);
                    }
                }
                Ok((directory, Err(error))) => {
                    observation
                        .errors
                        .push(Failure::directory(server, &directory, error));
                    observation.failed_history.insert(directory);
                }
                Err(error) => {
                    observation
                        .failed_history
                        .extend(directories.iter().cloned());
                    observation
                        .errors
                        .push(Failure::server(server, error.to_string()));
                }
            }
        }
        candidates.extend(observation.statuses.keys().cloned());
        candidates.extend(observation.questions.iter().cloned());
        candidates.extend(observation.approvals.iter().cloned());
        let mut session_tasks = tokio::task::JoinSet::new();
        for id in candidates.into_iter().filter(|id| valid_id(id)) {
            if observation.sessions.contains_key(&id) {
                continue;
            }
            let client = client.clone();
            let server = server.to_owned();
            session_tasks.spawn(async move {
                let result = transport::get_optional::<RemoteSession>(
                    &client,
                    &server,
                    &format!("/session/{id}"),
                )
                .await;
                (id, result)
            });
        }
        while let Some(result) = session_tasks.join_next().await {
            match result {
                Ok((_, Ok(Some(session)))) => {
                    observation.sessions.insert(session.id.clone(), session);
                }
                Ok((id, Ok(None))) => {
                    observation.deleted.insert(id);
                }
                Ok((_, Err(error))) => {
                    observation.errors.push(Failure::server(server, error));
                }
                Err(error) => {
                    observation
                        .errors
                        .push(Failure::server(server, error.to_string()));
                }
            }
        }
        Ok(())
    })
    .await;
    match result {
        Ok(Ok(())) => {}
        Ok(Err(error)) => {
            observation.errors.push(Failure::server(server, error));
        }
        Err(_) => {
            observation.errors.push(Failure::server(
                server,
                format!("{server}: OpenCode observation timed out"),
            ));
        }
    }
    observation
}
