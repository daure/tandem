use std::collections::{BTreeMap, BTreeSet};

use super::Observation;
use crate::{
    environments::opencode::{
        RemoteQuestion, RemoteSession, RemoteStatus, transport, v2, valid_id,
    },
    store::opencode::observation::Failure,
};

pub(super) async fn observe(
    client: &reqwest::Client,
    server: &str,
    directories: &BTreeSet<String>,
    mut attached: BTreeSet<String>,
    observation: &mut Observation,
) {
    let statuses = v2::read(client, server, "/session/status")
        .await
        .and_then(|value| {
            serde_json::from_value::<BTreeMap<String, RemoteStatus>>(
                value.ok_or("OpenCode activity unavailable")?,
            )
            .map_err(|_| "Invalid OpenCode activity map".into())
        });
    match statuses {
        Ok(statuses) => {
            attached.extend(statuses.keys().cloned());
            observation.statuses = statuses;
            observation.failed_status.clear();
            observation.approval_directories = directories.clone();
        }
        Err(error) => observation.errors.push(Failure::server(server, error)),
    }

    // Session form and permission reads boot location services too. Only attached
    // clients and foreground work authorize those reads; saved history does not.
    attached.retain(|id| valid_id(id));
    observation.failed_questions = attached.clone();
    observation.failed_approvals = attached.clone();
    let mut pending = attached.into_iter();
    let mut tasks = tokio::task::JoinSet::new();
    loop {
        while tasks.len() < 16 {
            let Some(id) = pending.next() else { break };
            let client = client.clone();
            let server = server.to_owned();
            tasks.spawn(async move {
                let session = transport::get_optional::<RemoteSession>(
                    &client,
                    &server,
                    &format!("/session/{id}"),
                )
                .await;
                let requests = if matches!(session, Ok(Some(_))) {
                    Some(tokio::join!(
                        requests(&client, &server, &id, "form"),
                        requests(&client, &server, &id, "permission"),
                    ))
                } else {
                    None
                };
                (id, session, requests)
            });
        }
        let Some(result) = tasks.join_next().await else {
            break;
        };
        match result {
            Ok((id, Ok(Some(session)), Some((questions, approvals)))) => {
                let directory = &session.directory;
                match questions {
                    Ok(questions) => {
                        observation.failed_questions.remove(&id);
                        observation.questions.extend(questions);
                    }
                    Err(error) => {
                        observation
                            .errors
                            .push(Failure::directory(server, directory, error));
                    }
                }
                match approvals {
                    Ok(approvals) => {
                        observation.failed_approvals.remove(&id);
                        observation.approvals.extend(approvals);
                    }
                    Err(error) => {
                        observation
                            .errors
                            .push(Failure::directory(server, directory, error));
                    }
                }
                observation.sessions.insert(session.id.clone(), session);
            }
            Ok((id, Ok(None), _)) => {
                observation.deleted.insert(id);
            }
            Ok((_, Err(error), _)) => observation.errors.push(Failure::server(server, error)),
            Err(error) => observation
                .errors
                .push(Failure::server(server, error.to_string())),
            _ => unreachable!("verified sessions have request results"),
        }
    }
}

async fn requests(
    client: &reqwest::Client,
    server: &str,
    id: &str,
    kind: &str,
) -> Result<BTreeSet<String>, String> {
    let value = v2::envelope(client, server, &format!("/api/session/{id}/{kind}"))
        .await?
        .ok_or_else(|| format!("OpenCode {kind} observation unavailable"))?;
    let requests: Vec<RemoteQuestion> = serde_json::from_value(value)
        .map_err(|_| format!("Invalid OpenCode {kind} observation"))?;
    Ok(requests
        .into_iter()
        .map(|request| request.session_id)
        .collect())
}
