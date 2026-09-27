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
        self.focus_pane(
            current,
            &Pane {
                tab_id: actual.tab_id,
                ..pane.clone()
            },
            actual.is_floating,
        )
        .await
    }

    async fn focus_pane(&self, current: &str, pane: &Pane, floating: bool) -> Result<(), String> {
        if current != pane.session {
            self.set_floating_visibility(&pane.session, pane.tab_id, floating)
                .await?;
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
            self.select_tab(current, pane.tab_id, floating).await?;
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

    async fn select_tab(&self, session: &str, tab_id: u32, floating: bool) -> Result<(), String> {
        zellij(
            &self.zellij,
            &[
                "--session".into(),
                session.into(),
                "action".into(),
                "go-to-tab-by-id".into(),
                tab_id.to_string(),
            ],
        )
        .await?;
        self.set_floating_visibility(session, tab_id, floating)
            .await
    }

    async fn set_floating_visibility(
        &self,
        session: &str,
        tab_id: u32,
        visible: bool,
    ) -> Result<(), String> {
        zellij(
            &self.zellij,
            &[
                "--session".into(),
                session.into(),
                "action".into(),
                if visible {
                    "show-floating-panes"
                } else {
                    "hide-floating-panes"
                }
                .into(),
                "--tab-id".into(),
                tab_id.to_string(),
            ],
        )
        .await?;
        Ok(())
    }

    pub(crate) async fn close(&self, session_id: &str, pane: &Pane) -> Result<(), String> {
        self.close_matching_pane(Some(session_id), None, pane).await
    }

    pub(crate) async fn close_pane(&self, pane: &Pane) -> Result<(), String> {
        self.close_matching_pane(None, None, pane).await
    }

    pub(crate) async fn close_in_directory(
        &self,
        directory: &str,
        pane: &Pane,
    ) -> Result<(), String> {
        self.close_matching_pane(None, Some(directory), pane).await
    }

    async fn close_matching_pane(
        &self,
        session_id: Option<&str>,
        directory: Option<&str>,
        pane: &Pane,
    ) -> Result<(), String> {
        let observer = self.clone();
        let id = session_id.map(str::to_owned);
        let directory = directory.map(str::to_owned);
        let target = pane.clone();
        let still_attached = tokio::task::spawn_blocking(move || {
            observer.inventory().0.iter().any(|presence| {
                id.as_ref().is_none_or(|id| presence.id == *id)
                    && directory
                        .as_ref()
                        .is_none_or(|directory| presence.directory == *directory)
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
        let server = local_server(&session.server)
            .ok_or("The OpenCode server is unavailable; start its server and retry")?;
        let _: serde_json::Value = get(&transport::client()?, &server, "/global/health").await?;
        self.launch_panel(
            &session.directory,
            instance_name,
            current,
            destination,
            &[
                "opencode".into(),
                "attach".into(),
                server,
                "--dir".into(),
                session.directory.clone(),
                "--session".into(),
                session.id.clone(),
            ],
        )
        .await
        .map(|_| ())
    }

    pub(crate) async fn new_session(
        &self,
        directory: &str,
        name: &str,
        current: &str,
        destination: Option<&Pane>,
        initial_prompt: Option<&str>,
    ) -> Result<Pane, String> {
        if current.is_empty() {
            return Err("Run Tandem inside Zellij to create an OpenCode session".into());
        }
        let observer = self.clone();
        let path = directory.to_owned();
        let servers = tokio::task::spawn_blocking(move || {
            if !Path::new(&path).is_absolute() || !Path::new(&path).is_dir() {
                return Err("OpenCode workspace directory is unavailable".to_owned());
            }
            Ok(observer.inventory().1)
        })
        .await
        .map_err(|error| error.to_string())??;
        let server = servers
            .into_iter()
            .find_map(|(server, directories)| directories.contains(directory).then_some(server));
        // Set this in the pane's command, not in Zellij's long-lived server environment.
        let mut command = vec![
            "env".into(),
            format!(
                "TANDEM_INITIAL_PROMPT={}",
                initial_prompt.unwrap_or_default()
            ),
        ];
        command.extend(if let Some(server) = server {
            let _: serde_json::Value =
                get(&transport::client()?, &server, "/global/health").await?;
            vec![
                "opencode".into(),
                "attach".into(),
                server,
                "--dir".into(),
                directory.into(),
            ]
        } else {
            vec!["opencode".into(), directory.into()]
        });
        self.launch_panel(directory, name, current, destination, &command)
            .await
    }

    async fn launch_panel(
        &self,
        directory: &str,
        name: &str,
        current: &str,
        destination: Option<&Pane>,
        command: &[String],
    ) -> Result<Pane, String> {
        let target_session = destination.map_or(current, |pane| pane.session.as_str());
        // An empty name lets OpenCode own the title; omitting it pins the command in Zellij.
        let mut args = vec!["--session".into(), target_session.into(), "action".into()];
        let tab_id = if let Some(destination) = destination {
            let panes = self.list_panes(target_session).await?;
            let pane = panes
                .iter()
                .find(|pane| pane.id == destination.id && !pane.is_plugin && !pane.exited)
                .ok_or("Destination pane closed; refresh and try again")?;
            args.extend([
                "new-pane".into(),
                "--stacked".into(),
                "--name".into(),
                String::new(),
                "--tab-id".into(),
                pane.tab_id.to_string(),
            ]);
            Some(pane.tab_id)
        } else {
            args.extend(["new-tab".into(), "--name".into(), name.into()]);
            None
        };
        args.extend(["--cwd".into(), directory.into(), "--".into()]);
        args.extend_from_slice(command);
        let created = zellij(&self.zellij, &args).await?;
        if let Some(tab_id) = tab_id {
            let id = created
                .trim()
                .strip_prefix("terminal_")
                .and_then(|id| id.parse::<u32>().ok())
                .ok_or("Zellij did not return the new OpenCode pane ID")?;
            let pane = Pane {
                session: target_session.into(),
                id,
                tab_id,
                tab_name: name.into(),
            };
            self.focus_pane(current, &pane, false).await?;
            Ok(pane)
        } else {
            let tab_id = created
                .trim()
                .parse::<u32>()
                .map_err(|_| "Zellij did not return the new OpenCode tab ID")?;
            let panes = self.list_panes(target_session).await?;
            let candidates = panes
                .iter()
                .filter(|pane| pane.tab_id == tab_id && !pane.is_plugin && !pane.exited)
                .collect::<Vec<_>>();
            let actual = if let [pane] = candidates.as_slice() {
                *pane
            } else {
                let command = command.join(" ");
                let mut matching = candidates
                    .iter()
                    .copied()
                    .filter(|pane| pane.terminal_command.as_deref() == Some(command.as_str()));
                let pane = matching
                    .next()
                    .ok_or("Cannot identify the new OpenCode pane")?;
                if matching.next().is_some() {
                    return Err("Multiple panes match the new OpenCode command".into());
                }
                pane
            };
            let pane = Pane {
                session: target_session.into(),
                id: actual.id,
                tab_id,
                tab_name: actual.tab_name.clone(),
            };
            zellij(
                &self.zellij,
                &[
                    "--session".into(),
                    target_session.into(),
                    "action".into(),
                    "rename-pane".into(),
                    "--pane-id".into(),
                    format!("terminal_{}", pane.id),
                    String::new(),
                ],
            )
            .await?;
            self.focus_pane(current, &pane, actual.is_floating).await?;
            Ok(pane)
        }
    }
}
