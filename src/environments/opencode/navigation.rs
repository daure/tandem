use super::*;

pub(super) fn client_command(command: &[String]) -> Vec<String> {
    // Check inside the pane: Zellij's PATH can differ from Tandem's environment.
    let mut launch = vec![
        "/bin/sh".into(),
        "-c".into(),
        "if command -v direnv >/dev/null 2>&1; then exec direnv exec . \"$@\"; else exec \"$@\"; fi".into(),
        "tandem-opencode".into(),
    ];
    launch.extend_from_slice(command);
    launch
}

pub(super) fn new_client_command(directory: &str) -> Vec<String> {
    vec![
        "/bin/sh".into(),
        "-c".into(),
        "case \"$(opencode --version)\" in 'opencode v2.'*|2.*) if command -v opencode-station >/dev/null 2>&1; then exec opencode-station run; else exec opencode \"$1\" --standalone --auto; fi;; *) exec opencode \"$@\";; esac".into(),
        "tandem-opencode-client".into(),
        directory.into(),
    ]
}

#[cfg(test)]
#[path = "tests/direnv.rs"]
mod direnv_tests;

pub(super) fn rule_client_command(model: &str, variant: Option<&str>) -> Vec<String> {
    // Station model flags pre-create a session; the V2 companion owns creation and model selection.
    let mut command = vec![
        "/bin/sh".into(), "-c".into(),
        "case \"$(opencode --version)\" in 'opencode v2.'*|2.*) exec opencode-station run;; *) exec opencode \"$@\";; esac".into(),
        "tandem-rule-session".into(), "--model".into(), model.into(),
    ];
    if let Some(variant) = variant {
        command.extend(["--variant".into(), variant.into()]);
    }
    command
}

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
        let attachment = tokio::task::spawn_blocking(move || {
            observer
                .inventory()
                .0
                .into_iter()
                .find(|presence| {
                    id.as_ref().is_none_or(|id| presence.id == *id)
                        && presence.zellij_session == target.session
                        && presence.pane_id == Some(target.id)
                })
                .map(|presence| presence.tab_control)
        })
        .await
        .map_err(|error| error.to_string())?;
        let Some(control) = attachment else {
            return Err(if expected_id.is_some() {
                "The pane changed conversation or closed; refresh and try again".into()
            } else {
                "The OpenCode pane closed or changed; refresh and try again".into()
            });
        };
        let panes = self.list_panes(&pane.session).await?;
        let actual = panes
            .iter()
            .find(|actual| actual.id == pane.id && !actual.is_plugin && !actual.exited)
            .ok_or("OpenCode pane has closed; refresh and try again")?;
        if current.is_empty() {
            return Err("Run Tandem inside Zellij to jump to a pane".into());
        }
        if let (Some(id), Some(control)) = (expected_id.as_deref(), control) {
            let selected = control
                .request("/tabs/focus", serde_json::json!({"sessionID": id}))
                .await?;
            if selected != id {
                return Err(
                    "OpenCode selected a different conversation; refresh and try again".into(),
                );
            }
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

    pub(super) async fn focus_pane(
        &self,
        current: &str,
        pane: &Pane,
        floating: bool,
    ) -> Result<(), String> {
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
        if self.close_session_tab(session_id, pane).await? {
            return Ok(());
        }
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
            observer.presences().iter().any(|presence| {
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
        let client = transport::client()?;
        let _: serde_json::Value = get(&client, &server, "/global/health").await?;
        let command = if transport::is_v2(&client, &server).await? {
            vec![
                "opencode-station".into(),
                "connect".into(),
                server,
                session.directory.clone(),
                "--session".into(),
                session.id.clone(),
            ]
        } else {
            vec![
                "opencode".into(),
                "attach".into(),
                server,
                "--dir".into(),
                session.directory.clone(),
                "--session".into(),
                session.id.clone(),
            ]
        };
        self.launch_panel(
            &session.directory,
            instance_name,
            current,
            destination,
            &command,
            true,
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
        instructions: Option<&str>,
    ) -> Result<Pane, String> {
        self.new_session_selected(
            directory,
            name,
            current,
            destination,
            &crate::store::opencode::Launch {
                prompt: initial_prompt.map(str::to_owned),
                ..Default::default()
            },
            instructions,
        )
        .await
    }

    pub(crate) async fn new_session_selected(
        &self,
        directory: &str,
        name: &str,
        current: &str,
        destination: Option<&Pane>,
        launch: &crate::store::opencode::Launch,
        instructions: Option<&str>,
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
                launch.prompt.as_deref().unwrap_or_default()
            ),
        ];
        command.push(format!(
            "TANDEM_SESSION_INSTRUCTIONS={}",
            instructions.unwrap_or_default()
        ));
        command.push(format!(
            "TANDEM_SESSION_MODEL={}",
            launch.selector().unwrap_or_default()
        ));
        command.push(format!(
            "TANDEM_SESSION_VARIANT={}",
            launch.variant.as_deref().unwrap_or_default()
        ));
        let selector = launch.selector();
        let (model, variant) =
            selector
                .as_deref()
                .map_or((None, launch.variant.as_deref()), |selector| {
                    let (model, variant) = selector
                        .split_once('#')
                        .map_or((selector, None), |(model, variant)| (model, Some(variant)));
                    (Some(model), variant)
                });
        let mut selection = Vec::<String>::new();
        if let Some(model) = model {
            selection.extend(["--model".into(), model.into()]);
        }
        if let Some(variant) = variant {
            selection.extend(["--variant".into(), variant.into()]);
        }
        command.extend(if let Some(server) = server {
            let client = transport::client()?;
            let _: serde_json::Value = get(&client, &server, "/global/health").await?;
            if transport::is_v2(&client, &server).await? {
                vec![
                    "opencode-station".into(),
                    "connect".into(),
                    server,
                    directory.into(),
                ]
            } else {
                let mut client = vec![
                    "opencode".into(),
                    "attach".into(),
                    server,
                    "--dir".into(),
                    directory.into(),
                ];
                client.extend(selection);
                client
            }
        } else {
            // Select the adapter inside the pane, after direnv resolves its station and PATH.
            let mut client = new_client_command(directory);
            client.extend(selection);
            client
        });
        self.launch_panel(directory, name, current, destination, &command, true)
            .await
    }

    pub(crate) async fn new_rule_session(
        &self,
        directory: &str,
        name: &str,
        current: &str,
        rule: &crate::store::rules::Definition,
        prompt: &str,
        instructions: Option<&str>,
    ) -> Result<Pane, String> {
        if current.is_empty()
            || !Path::new(directory).is_absolute()
            || !Path::new(directory).is_dir()
        {
            return Err(
                "rule session requires a prepared workspace and a target Zellij session".into(),
            );
        }
        self.validate_rule_destination(current).await?;
        let selector = rule
            .session_launch()
            .selector()
            .ok_or("rule model unavailable")?;
        let model = selector.as_str();
        let (model, variant) = model
            .split_once('#')
            .map_or((model, None), |(model, variant)| (model, Some(variant)));
        let mut command = vec![
            "env".into(),
            format!("TANDEM_INITIAL_PROMPT={prompt}"),
            format!(
                "TANDEM_SESSION_INSTRUCTIONS={}",
                instructions.unwrap_or_default()
            ),
            format!(
                "TANDEM_SESSION_MODEL={model}{}",
                variant
                    .map(|variant| format!("#{variant}"))
                    .unwrap_or_default()
            ),
            "TANDEM_SESSION_VARIANT=".into(),
        ];
        command.extend(rule_client_command(model, variant));
        self.launch_panel(directory, name, current, None, &command, rule.focus_pane)
            .await
    }

    pub(crate) async fn validate_rule_destination(&self, name: &str) -> Result<(), String> {
        let sessions = zellij(
            &self.zellij,
            &[
                "list-sessions".into(),
                "--short".into(),
                "--no-formatting".into(),
            ],
        )
        .await?;
        if sessions.lines().any(|session| session == name) {
            Ok(())
        } else {
            Err(format!(
                "Zellij session {name:?} is unavailable; deactivate and reactivate the rule in the current Tandem TUI"
            ))
        }
    }

    async fn live_destination(&self, destination: &Pane) -> Result<Option<Pane>, String> {
        let panes = match self.list_panes(&destination.session).await {
            Ok(panes) => panes,
            Err(error) => {
                let sessions = zellij(
                    &self.zellij,
                    &[
                        "list-sessions".into(),
                        "--short".into(),
                        "--no-formatting".into(),
                    ],
                )
                .await?;
                if sessions.lines().any(|name| name == destination.session) {
                    return Err(error);
                }
                return Ok(None);
            }
        };
        Ok(panes
            .into_iter()
            .find(|pane| pane.id == destination.id && !pane.is_plugin && !pane.exited)
            .map(|pane| Pane {
                session: destination.session.clone(),
                id: pane.id,
                tab_id: pane.tab_id,
                tab_name: pane.tab_name,
            }))
    }

    async fn launch_panel(
        &self,
        directory: &str,
        name: &str,
        current: &str,
        destination: Option<&Pane>,
        command: &[String],
        focus: bool,
    ) -> Result<Pane, String> {
        let destination = match destination {
            Some(pane) => self.live_destination(pane).await?,
            None => None,
        };
        let target_session = destination
            .as_ref()
            .map_or(current, |pane| pane.session.as_str());
        let command = client_command(command);
        // An empty name lets OpenCode own the title; omitting it pins the command in Zellij.
        let mut args = vec!["--session".into(), target_session.into(), "action".into()];
        let tab_id = if let Some(pane) = &destination {
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
        if !focus {
            args.push("--no-focus".into());
        }
        args.extend(["--cwd".into(), directory.into(), "--".into()]);
        args.extend_from_slice(&command);
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
            if focus {
                self.focus_pane(current, &pane, false).await?;
            }
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
            if focus {
                self.focus_pane(current, &pane, actual.is_floating).await?;
            }
            Ok(pane)
        }
    }
}
