use super::*;

impl Observer {
    pub(super) async fn list_panes(&self, name: &str) -> Result<Vec<RemotePane>, String> {
        let output = zellij(
            &self.zellij,
            &[
                "--session".into(),
                name.into(),
                "action".into(),
                "list-panes".into(),
                "--all".into(),
                "--json".into(),
            ],
        )
        .await?;
        serde_json::from_str(&output).map_err(|_| "Invalid Zellij pane inventory".into())
    }

    pub(crate) async fn jump(
        &self,
        session_id: &str,
        pane: &Pane,
        current: &str,
    ) -> Result<(), String> {
        self.jump_matching_pane(Some(session_id), pane, current)
            .await
    }

    pub(crate) async fn jump_pane(&self, pane: &Pane, current: &str) -> Result<(), String> {
        self.jump_matching_pane(None, pane, current).await
    }

    async fn jump_matching_pane(
        &self,
        session_id: Option<&str>,
        pane: &Pane,
        current: &str,
    ) -> Result<(), String> {
        let observer = self.clone();
        let expected_id = session_id.map(str::to_owned);
        let id = expected_id.clone();
        let target = pane.clone();
        let still_attached = tokio::task::spawn_blocking(move || {
            observer.inventory().0.iter().any(|presence| {
                id.as_ref().is_none_or(|id| presence.id == *id)
                    && presence.zellij_session == target.session
                    && presence.pane_id == Some(target.id)
            })
        })
        .await
        .map_err(|error| error.to_string())?;
        if !still_attached {
            return Err(if expected_id.is_some() {
                "The pane changed conversation or closed; refresh and try again".into()
            } else {
                "The OpenCode pane closed or changed; refresh and try again".into()
            });
        }
        let panes = self.list_panes(&pane.session).await?;
        let actual = panes
            .iter()
            .find(|actual| actual.id == pane.id && !actual.is_plugin && !actual.exited)
            .ok_or("OpenCode pane has closed; refresh and try again")?;
        if current.is_empty() {
            return Err("Run Tandem inside Zellij to jump to a pane".into());
        }
        if current != pane.session {
            zellij(
                &self.zellij,
                &[
                    "--session".into(),
                    current.into(),
                    "action".into(),
                    "switch-session".into(),
                    pane.session.clone(),
                    "--pane-id".into(),
                    format!("terminal_{}", pane.id),
                ],
            )
            .await?;
        } else {
            zellij(
                &self.zellij,
                &[
                    "--session".into(),
                    current.into(),
                    "action".into(),
                    "go-to-tab-by-id".into(),
                    actual.tab_id.to_string(),
                ],
            )
            .await?;
            zellij(
                &self.zellij,
                &[
                    "--session".into(),
                    current.into(),
                    "action".into(),
                    "focus-pane-id".into(),
                    format!("terminal_{}", pane.id),
                ],
            )
            .await?;
        }
        Ok(())
    }

    pub(crate) async fn close(&self, session_id: &str, pane: &Pane) -> Result<(), String> {
        self.close_matching_pane(Some(session_id), pane).await
    }

    pub(crate) async fn close_pane(&self, pane: &Pane) -> Result<(), String> {
        self.close_matching_pane(None, pane).await
    }

    async fn close_matching_pane(
        &self,
        session_id: Option<&str>,
        pane: &Pane,
    ) -> Result<(), String> {
        let observer = self.clone();
        let id = session_id.map(str::to_owned);
        let target = pane.clone();
        let still_attached = tokio::task::spawn_blocking(move || {
            observer.inventory().0.iter().any(|presence| {
                id.as_ref().is_none_or(|id| presence.id == *id)
                    && presence.zellij_session == target.session
                    && presence.pane_id == Some(target.id)
            })
        })
        .await
        .map_err(|error| error.to_string())?;
        if !still_attached {
            return Err("The OpenCode pane closed or changed; refresh and try again".into());
        }
        let panes = self.list_panes(&pane.session).await?;
        panes
            .iter()
            .find(|actual| actual.id == pane.id && !actual.is_plugin && !actual.exited)
            .ok_or("OpenCode pane has closed; refresh and try again")?;
        zellij(
            &self.zellij,
            &[
                "--session".into(),
                pane.session.clone(),
                "action".into(),
                "close-pane".into(),
                "--pane-id".into(),
                format!("terminal_{}", pane.id),
            ],
        )
        .await?;
        Ok(())
    }

    pub(crate) async fn attach(
        &self,
        session: &Session,
        instance_name: &str,
        current: &str,
        destination: Option<&Pane>,
    ) -> Result<(), String> {
        if current.is_empty() {
            return Err("Run Tandem inside Zellij to attach a conversation".into());
        }
        if !valid_id(&session.id) || !Path::new(&session.directory).is_absolute() {
            return Err("Invalid OpenCode session target".into());
        }
        let server = local_server(&session.server).ok_or(
            "The OpenCode server is unavailable; reopen the workspace with your open command",
        )?;
        let _: serde_json::Value = get(&transport::client()?, &server, "/global/health").await?;
        let target_session = destination.map_or(current, |pane| pane.session.as_str());
        let mut args = vec!["--session".into(), target_session.into(), "action".into()];
        if let Some(destination) = destination {
            let panes = self.list_panes(target_session).await?;
            let pane = panes
                .iter()
                .find(|pane| pane.id == destination.id && !pane.is_plugin && !pane.exited)
                .ok_or("Destination pane closed; refresh and try again")?;
            args.extend([
                "new-pane".into(),
                "--stacked".into(),
                "--tab-id".into(),
                pane.tab_id.to_string(),
            ]);
        } else {
            args.extend(["new-tab".into(), "--name".into(), instance_name.into()]);
        }
        args.extend([
            "--cwd".into(),
            session.directory.clone(),
            "--".into(),
            "opencode".into(),
            "attach".into(),
            server,
            "--dir".into(),
            session.directory.clone(),
            "--session".into(),
            session.id.clone(),
        ]);
        let created = zellij(&self.zellij, &args).await?;
        if current != target_session {
            let pane = created.trim();
            if pane
                .strip_prefix("terminal_")
                .is_none_or(|id| id.parse::<u32>().is_err())
            {
                return Err("Zellij did not return the new OpenCode pane ID".into());
            }
            zellij(
                &self.zellij,
                &[
                    "--session".into(),
                    current.into(),
                    "action".into(),
                    "switch-session".into(),
                    target_session.into(),
                    "--pane-id".into(),
                    pane.into(),
                ],
            )
            .await?;
        }
        Ok(())
    }
}
