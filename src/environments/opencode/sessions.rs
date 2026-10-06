use std::collections::{BTreeMap, BTreeSet};

use crate::store::opencode::Session;

use super::{Observer, RemoteSession, RemoteStatus, history, transport, v2, valid_id};

async fn verified(client: &reqwest::Client, session: &Session) -> Result<RemoteSession, String> {
    if !valid_id(&session.id) {
        return Err("Invalid OpenCode conversation ID".into());
    }
    let path = history::target(&format!("/session/{}", session.id), &session.directory)?;
    let current: RemoteSession = transport::get_optional(client, &session.server, &path)
        .await?
        .ok_or("OpenCode conversation was deleted; refresh and try again")?;
    if current.id != session.id || current.directory != session.directory {
        return Err("OpenCode conversation ownership changed; refresh and try again".into());
    }
    Ok(current)
}

pub(crate) async fn rename(session: &Session, title: &str) -> Result<(), String> {
    let client = transport::client()?;
    verified(&client, session).await?;
    let path = if transport::is_v2(&client, &session.server).await? {
        format!("/api/session/{}", session.id)
    } else {
        history::target(&format!("/session/{}", session.id), &session.directory)?
    };
    transport::raw_body(
        &client,
        &session.server,
        &path,
        reqwest::Method::PATCH,
        Some(serde_json::json!({"title": title})),
    )
    .await?;
    if verified(&client, session).await?.title != title {
        return Err("OpenCode did not save the conversation title".into());
    }
    Ok(())
}

impl Observer {
    pub(crate) async fn clear_directory(
        &self,
        directory: &str,
        servers: Vec<String>,
    ) -> super::cleanup::Outcome {
        let mut removed = Vec::new();
        let result = tokio::time::timeout(std::time::Duration::from_secs(600), async {
            history::clear_known(self, directory, servers, &mut removed).await?;
            self.remove_directory_receipts(directory).await
        })
        .await
        .unwrap_or_else(|_| Err("OpenCode cleanup timed out; refresh before retrying".into()));
        super::cleanup::Outcome {
            cleared: result.is_ok(),
            removed,
            error: result.err(),
        }
    }

    async fn remove_directory_receipts(&self, directory: &str) -> Result<(), String> {
        let observer = self.clone();
        let directory = directory.to_owned();
        tokio::task::spawn_blocking(move || {
            if observer
                .presences()
                .iter()
                .any(|presence| presence.directory == directory)
            {
                return Err("An OpenCode client opened during cleanup; retry".to_owned());
            }
            for daemon in super::entries(&observer.daemons) {
                for path in super::entries(&daemon.join("dirs")) {
                    if path.extension().is_some_and(|extension| extension == "dir")
                        && super::read_small(&path)
                            .is_some_and(|value| value.trim_end_matches('\n') == directory)
                    {
                        std::fs::remove_file(path).map_err(|error| error.to_string())?;
                    }
                }
            }
            Ok(())
        })
        .await
        .map_err(|error| error.to_string())?
    }

    pub(crate) async fn delete_session(&self, session: &Session) -> Result<Vec<String>, String> {
        let client = transport::client()?;
        verified(&client, session).await?;
        let mut pending = BTreeMap::new();
        let mut queue = vec![session.id.clone()];
        while let Some(id) = queue.pop() {
            if pending.contains_key(&id) || pending.len() >= 10_000 {
                return Err("OpenCode conversation ancestry cannot be safely deleted".into());
            }
            let path = history::target(&format!("/session/{id}"), &session.directory)?;
            let current: history::SavedSession =
                transport::get(&client, &session.server, &path).await?;
            if current.id != id || current.directory != session.directory || !valid_id(&id) {
                return Err("Session deletion would include an unverified conversation".into());
            }
            let children: Vec<history::SavedSession> = transport::get(
                &client,
                &session.server,
                &history::target(&format!("/session/{id}/children"), &session.directory)?,
            )
            .await?;
            if children.iter().any(|child| {
                child.directory != session.directory
                    || !valid_id(&child.id)
                    || child.parent_id.as_deref() != Some(&id)
            }) {
                return Err(
                    "Session deletion would include an unverified child conversation".into(),
                );
            }
            queue.extend(children.into_iter().map(|child| child.id));
            pending.insert(id, current);
        }
        let ordered = history::deletion_order(pending)?;
        let ids: BTreeSet<_> = ordered.iter().map(|session| session.id.clone()).collect();
        self.close_session_clients(&session.server, &ids).await?;
        stop_sessions(&client, session, &ordered).await?;
        for current in &ordered {
            let path = history::target(&format!("/session/{}", current.id), &session.directory)?;
            let fresh =
                transport::get::<history::SavedSession>(&client, &session.server, &path).await?;
            if fresh.id != current.id
                || fresh.directory != session.directory
                || fresh.parent_id != current.parent_id
            {
                return Err("OpenCode conversation ownership changed during deletion".into());
            }
            let children: Vec<history::SavedSession> = transport::get(
                &client,
                &session.server,
                &history::target(
                    &format!("/session/{}/children", current.id),
                    &session.directory,
                )?,
            )
            .await?;
            let expected: BTreeSet<_> = ordered
                .iter()
                .filter(|child| child.parent_id.as_deref() == Some(&current.id))
                .map(|child| child.id.clone())
                .collect();
            let found: BTreeSet<_> = children.iter().map(|child| child.id.clone()).collect();
            if found != expected
                || children
                    .iter()
                    .any(|child| child.directory != session.directory)
            {
                return Err("OpenCode conversation children changed; refresh and try again".into());
            }
        }
        self.close_session_clients(&session.server, &ids).await?;
        ensure_stopped(&client, session, &ordered).await?;
        let path = history::target(&format!("/session/{}", session.id), &session.directory)?;
        transport::delete(&client, &session.server, &path).await?;
        for id in &ids {
            let path = history::target(&format!("/session/{id}"), &session.directory)?;
            if transport::get_optional::<history::SavedSession>(&client, &session.server, &path)
                .await?
                .is_some()
            {
                return Err("OpenCode did not delete the conversation".into());
            }
        }
        Ok(ids.into_iter().collect())
    }

    async fn close_session_clients(
        &self,
        server: &str,
        ids: &BTreeSet<String>,
    ) -> Result<(), String> {
        let server = transport::local_server(server).ok_or("OpenCode server must be local HTTP")?;
        let targets = self
            .presences()
            .into_iter()
            .filter(|presence| {
                transport::local_server(&presence.server).as_ref() == Some(&server)
                    && ids.contains(&presence.id)
            })
            .collect::<Vec<_>>();
        let mut closed = BTreeSet::new();
        let names = if targets.is_empty() {
            String::new()
        } else {
            transport::zellij(
                &self.zellij,
                &[
                    "list-sessions".into(),
                    "--short".into(),
                    "--no-formatting".into(),
                ],
            )
            .await?
        };
        for presence in targets {
            let Some(id) = presence
                .pane_id
                .filter(|_| !presence.zellij_session.is_empty())
            else {
                return Err("Close the OpenCode client outside Zellij and retry".into());
            };
            if !closed.insert((presence.zellij_session.clone(), id)) {
                continue;
            }
            if !names.lines().any(|name| name == presence.zellij_session)
                || !self
                    .list_panes(&presence.zellij_session)
                    .await?
                    .iter()
                    .any(|pane| pane.id == id && !pane.is_plugin && !pane.exited)
            {
                continue;
            }
            let pane = crate::store::opencode::Pane {
                session: presence.zellij_session,
                id,
                tab_id: 0,
                tab_name: String::new(),
            };
            self.close(&presence.id, &pane).await?;
            self.wait_closed(&pane).await?;
        }
        Ok(())
    }
}

async fn stop_sessions(
    client: &reqwest::Client,
    session: &Session,
    ordered: &[history::SavedSession],
) -> Result<(), String> {
    let native = transport::is_v2(client, &session.server).await?;
    for current in ordered {
        let path = if native {
            format!("/api/session/{}/interrupt?resume=false", current.id)
        } else {
            history::target(
                &format!("/session/{}/abort", current.id),
                &session.directory,
            )?
        };
        transport::raw_body(client, &session.server, &path, reqwest::Method::POST, None).await?;
    }
    ensure_stopped(client, session, ordered).await
}

async fn ensure_stopped(
    client: &reqwest::Client,
    session: &Session,
    ordered: &[history::SavedSession],
) -> Result<(), String> {
    if transport::is_v2(client, &session.server).await? {
        let active = v2::envelope(client, &session.server, "/api/session/active")
            .await?
            .ok_or("OpenCode activity unavailable")?;
        let active = active.as_object().ok_or("Invalid OpenCode activity map")?;
        for current in ordered {
            if active.contains_key(&current.id) {
                return Err("OpenCode conversations are active".into());
            }
        }
    } else {
        let statuses: BTreeMap<String, RemoteStatus> = transport::get(
            client,
            &session.server,
            &history::target("/session/status", &session.directory)?,
        )
        .await?;
        if ordered.iter().any(|current| {
            statuses
                .get(&current.id)
                .is_some_and(|status| status.kind != "idle")
        }) {
            return Err("OpenCode conversations are active".into());
        }
    }
    Ok(())
}
