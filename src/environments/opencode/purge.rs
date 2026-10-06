use super::*;
use std::time::{Duration, Instant};

impl Observer {
    pub(crate) async fn close_workspace(
        &self,
        workspace: &str,
        deadline: Instant,
    ) -> Result<(), String> {
        tokio::time::timeout_at(deadline.into(), async {
            loop {
                let targets = self.workspace_panes(workspace).await?;
                if targets.is_empty() {
                    return Ok::<(), String>(());
                }
                self.close_panes(targets, deadline).await?;
            }
        })
        .await
        .map_err(|_| "Timed out closing OpenCode clients; instance data preserved".to_owned())?
        .map_err(|error| format!("Cannot close OpenCode clients before purge: {error}"))
    }

    pub(crate) async fn close_panes(
        &self,
        targets: Vec<(String, Pane)>,
        deadline: Instant,
    ) -> Result<(), String> {
        tokio::time::timeout_at(deadline.into(), async {
            let mut closing = tokio::task::JoinSet::new();
            for (directory, pane) in targets {
                let observer = self.clone();
                closing.spawn(async move {
                    let result = async {
                        observer.close_in_directory(&directory, &pane).await?;
                        observer.wait_closed(&pane).await
                    }
                    .await;
                    result.map_err(|error| format!("{} / pane {}: {error}", pane.session, pane.id))
                });
            }
            let mut errors = Vec::new();
            while let Some(result) = closing.join_next().await {
                match result {
                    Ok(Ok(())) => {}
                    Ok(Err(error)) => errors.push(error),
                    Err(error) => errors.push(error.to_string()),
                }
            }
            if errors.is_empty() {
                Ok(())
            } else {
                Err(errors.join("\n"))
            }
        })
        .await
        .map_err(|_| "Timed out closing OpenCode clients".to_owned())?
    }

    pub(super) async fn wait_closed(&self, pane: &Pane) -> Result<(), String> {
        loop {
            let names = zellij(
                &self.zellij,
                &[
                    "list-sessions".into(),
                    "--short".into(),
                    "--no-formatting".into(),
                ],
            )
            .await?;
            if !names.lines().any(|name| name == pane.session)
                || !self
                    .list_panes(&pane.session)
                    .await?
                    .iter()
                    .any(|actual| actual.id == pane.id && !actual.is_plugin && !actual.exited)
            {
                return Ok(());
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }

    async fn workspace_panes(&self, workspace: &str) -> Result<Vec<(String, Pane)>, String> {
        let observer = self.clone();
        let presences = tokio::task::spawn_blocking(move || observer.presences())
            .await
            .map_err(|error| error.to_string())?;
        let presences = presences
            .into_iter()
            .filter(|presence| {
                crate::store::opencode::workspace_owner(
                    &presence.directory,
                    std::iter::once((workspace, workspace)),
                )
                .is_some()
            })
            .collect::<Vec<_>>();
        if presences.is_empty() {
            return Ok(Vec::new());
        }
        if presences
            .iter()
            .any(|presence| presence.zellij_session.is_empty() || presence.pane_id.is_none())
        {
            return Err(
                "An associated OpenCode client is outside Zellij; close it manually and retry"
                    .into(),
            );
        }
        let names = zellij(
            &self.zellij,
            &[
                "list-sessions".into(),
                "--short".into(),
                "--no-formatting".into(),
            ],
        )
        .await?;
        let mut panes = BTreeMap::new();
        let mut targets = BTreeMap::new();
        for presence in presences {
            if !names.lines().any(|name| name == presence.zellij_session) {
                continue;
            }
            if !panes.contains_key(&presence.zellij_session) {
                panes.insert(
                    presence.zellij_session.clone(),
                    self.list_panes(&presence.zellij_session).await?,
                );
            }
            if let Some(actual) = panes[&presence.zellij_session]
                .iter()
                .find(|pane| Some(pane.id) == presence.pane_id && !pane.is_plugin && !pane.exited)
            {
                targets.insert(
                    (presence.zellij_session.clone(), actual.id),
                    (
                        presence.directory,
                        Pane {
                            session: presence.zellij_session,
                            id: actual.id,
                            tab_id: actual.tab_id,
                            tab_name: actual.tab_name.clone(),
                        },
                    ),
                );
            }
        }
        Ok(targets.into_values().collect())
    }
}
