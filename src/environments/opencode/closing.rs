use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use super::{Observer, Pane, Presence, RemotePane, transport::zellij};
use crate::store::opencode::closing::{CloseInput, Sessions};

#[derive(Clone, Deserialize, Serialize)]
pub(crate) struct Attachment {
    id: String,
    directory: String,
    server: String,
    pid: u32,
    pane: Pane,
}

#[derive(Deserialize, Serialize)]
pub(crate) struct Plan {
    caller: Attachment,
    attachments: Vec<Attachment>,
    tab: Option<Pane>,
}

async fn owned_directories(
    workspace: &str,
    directories: Vec<String>,
) -> Result<std::collections::BTreeSet<String>, String> {
    let workspace = workspace.to_owned();
    tokio::task::spawn_blocking(move || {
        let root = std::fs::canonicalize(&workspace).map_err(|error| error.to_string())?;
        Ok(directories
            .into_iter()
            .filter(|directory| {
                std::fs::canonicalize(directory).is_ok_and(|path| path.starts_with(&root))
            })
            .collect())
    })
    .await
    .map_err(|error| error.to_string())?
}

fn attached(presence: &Presence, pane: &Pane) -> bool {
    presence.zellij_session == pane.session && presence.pane_id == Some(pane.id)
}

fn verify_attachment(target: &Attachment, presences: &[Presence]) -> Result<(), String> {
    if !presences.iter().any(|presence| {
        presence.id == target.id
            && presence.server == target.server
            && presence.pid == target.pid
            && presence.directory == target.directory
            && attached(presence, &target.pane)
    }) || (target.id.is_empty()
        && presences
            .iter()
            .any(|presence| attached(presence, &target.pane) && !presence.id.is_empty()))
    {
        return Err(
            "selected OpenCode attachment closed or changed; inspect before retrying".into(),
        );
    }
    Ok(())
}

async fn fresh_presences(observer: &Observer) -> Result<Vec<Presence>, String> {
    let observer = observer.clone();
    tokio::task::spawn_blocking(move || observer.presences())
        .await
        .map_err(|error| error.to_string())
}

impl Observer {
    pub(crate) async fn plan_instance_close(
        &self,
        workspace: &str,
        caller: &str,
        input: &CloseInput,
    ) -> Result<Plan, String> {
        input.validate()?;
        if !super::valid_id(caller) {
            return Err("close requires verified OpenCode session metadata".into());
        }
        let presences = fresh_presences(self).await?;
        let owned = owned_directories(
            workspace,
            presences
                .iter()
                .map(|presence| presence.directory.clone())
                .collect(),
        )
        .await?;
        let callers: Vec<_> = presences
            .iter()
            .filter(|presence| presence.id == caller)
            .collect();
        let current = callers
            .first()
            .ok_or("calling OpenCode session is not attached; refresh before closing")?;
        if callers.iter().any(|presence| {
            presence.pid != current.pid
                || presence.server != current.server
                || presence.pane_id != current.pane_id
                || presence.zellij_session != current.zellij_session
        }) {
            return Err("calling OpenCode session has ambiguous attachments".into());
        }
        if !owned.contains(&current.directory) {
            return Err("calling session belongs to another workspace".into());
        }
        let mut attachments = Vec::new();
        let mut caller_attachment = None;
        let mut inventories = std::collections::BTreeMap::new();
        for presence in &presences {
            if presence.id.is_empty()
                && presences.iter().any(|other| {
                    !other.id.is_empty()
                        && other.zellij_session == presence.zellij_session
                        && other.pane_id == presence.pane_id
                })
            {
                continue;
            }
            let selected = match input.sessions {
                Sessions::None => false,
                Sessions::Current => presence.id == caller,
                Sessions::All => owned.contains(&presence.directory),
            };
            if !selected && presence.id != caller {
                continue;
            }
            if presence.zellij_session.is_empty() || presence.pane_id.is_none() {
                return Err("selected OpenCode client is outside Zellij; close it manually".into());
            }
            if !inventories.contains_key(&presence.zellij_session) {
                inventories.insert(
                    presence.zellij_session.clone(),
                    self.list_panes(&presence.zellij_session).await?,
                );
            }
            let actual = inventories[&presence.zellij_session]
                .iter()
                .find(|pane| Some(pane.id) == presence.pane_id && !pane.is_plugin && !pane.exited)
                .ok_or("selected OpenCode pane closed or changed")?;
            let target = Attachment {
                id: presence.id.clone(),
                directory: presence.directory.clone(),
                server: presence.server.clone(),
                pid: presence.pid,
                pane: Pane {
                    session: presence.zellij_session.clone(),
                    id: actual.id,
                    tab_id: actual.tab_id,
                    tab_name: actual.tab_name.clone(),
                },
            };
            if presence.id == caller {
                caller_attachment = Some(target.clone());
            }
            if selected
                && !attachments.iter().any(|existing: &Attachment| {
                    existing.id == target.id && existing.pane == target.pane
                })
            {
                attachments.push(target);
            }
        }
        let tab = if input.zellij_tab {
            let actual = inventories[&current.zellij_session]
                .iter()
                .find(|pane| Some(pane.id) == current.pane_id && !pane.is_plugin && !pane.exited)
                .ok_or("calling OpenCode pane is unavailable")?;
            let pane = Pane {
                session: current.zellij_session.clone(),
                id: actual.id,
                tab_id: actual.tab_id,
                tab_name: actual.tab_name.clone(),
            };
            self.verify_owned_tab(workspace, &pane, &presences, true)
                .await?;
            Some(pane)
        } else {
            None
        };
        Ok(Plan {
            caller: caller_attachment.ok_or("calling OpenCode attachment is unavailable")?,
            attachments,
            tab,
        })
    }

    async fn verify_owned_tab(
        &self,
        workspace: &str,
        tab: &Pane,
        presences: &[Presence],
        require_caller: bool,
    ) -> Result<Vec<RemotePane>, String> {
        let panes = self.list_panes(&tab.session).await?;
        let members: Vec<_> = panes
            .into_iter()
            .filter(|pane| pane.tab_id == tab.tab_id && !pane.exited)
            .collect();
        let owned = owned_directories(
            workspace,
            members
                .iter()
                .filter_map(|pane| pane.pane_cwd.clone())
                .chain(presences.iter().map(|presence| presence.directory.clone()))
                .collect(),
        )
        .await?;
        if require_caller
            && !members
                .iter()
                .any(|pane| pane.id == tab.id && !pane.is_plugin)
        {
            return Err("calling Zellij tab closed or changed".into());
        }
        for pane in &members {
            let target = Pane {
                id: pane.id,
                ..tab.clone()
            };
            let clients: Vec<_> = presences
                .iter()
                .filter(|presence| !pane.is_plugin && attached(presence, &target))
                .collect();
            if clients
                .iter()
                .any(|presence| !owned.contains(&presence.directory))
                || (clients.is_empty()
                    && !pane
                        .pane_cwd
                        .as_ref()
                        .is_some_and(|cwd| owned.contains(cwd)))
            {
                return Err(format!(
                    "Zellij tab contains unrelated or unverified pane {}; tab preserved",
                    pane.id
                ));
            }
        }
        Ok(members)
    }

    pub(crate) async fn execute_instance_close(
        &self,
        workspace: &str,
        plan: Plan,
    ) -> Result<(), String> {
        let deadline = Instant::now() + Duration::from_secs(60);
        tokio::time::timeout_at(deadline.into(), async {
            // Validate every requested effect before closing any attachment.
            let presences = fresh_presences(self).await?;
            let owned = owned_directories(
                workspace,
                presences
                    .iter()
                    .map(|presence| presence.directory.clone())
                    .collect(),
            )
            .await?;
            for target in std::iter::once(&plan.caller).chain(&plan.attachments) {
                if !owned.contains(&target.directory) {
                    return Err("selected OpenCode attachment belongs to another workspace".into());
                }
                verify_attachment(target, &presences)?;
            }
            if let Some(tab) = &plan.tab {
                self.verify_owned_tab(workspace, tab, &presences, true)
                    .await?;
            }
            for target in &plan.attachments {
                verify_attachment(target, &fresh_presences(self).await?)?;
                if target.id.is_empty() {
                    self.close_empty_client(&target.directory, &target.pane)
                        .await?;
                    self.wait_closed(&target.pane).await?;
                } else {
                    self.close(&target.id, &target.pane).await?;
                    if !fresh_presences(self)
                        .await?
                        .iter()
                        .any(|presence| attached(presence, &target.pane) && !presence.id.is_empty())
                    {
                        self.wait_closed(&target.pane).await?;
                    }
                }
            }
            if let Some(tab) = &plan.tab {
                let presences = fresh_presences(self).await?;
                if !plan
                    .attachments
                    .iter()
                    .any(|target| target.id == plan.caller.id && target.pane == plan.caller.pane)
                {
                    verify_attachment(&plan.caller, &presences)?;
                }
                if !self.tab_open(tab).await? {
                    return Ok(());
                }
                if self
                    .verify_owned_tab(workspace, tab, &presences, false)
                    .await?
                    .is_empty()
                {
                    return Ok(());
                }
                zellij(
                    &self.zellij,
                    &[
                        "--session".into(),
                        tab.session.clone(),
                        "action".into(),
                        "close-tab".into(),
                        "--tab-id".into(),
                        tab.tab_id.to_string(),
                    ],
                )
                .await?;
                self.wait_tab_closed(tab).await?;
            }
            Ok(())
        })
        .await
        .map_err(|_| "Instance closure timed out; inspect clients before retrying".to_owned())?
    }

    async fn tab_open(&self, tab: &Pane) -> Result<bool, String> {
        let names = zellij(
            &self.zellij,
            &[
                "list-sessions".into(),
                "--short".into(),
                "--no-formatting".into(),
            ],
        )
        .await?;
        Ok(names.lines().any(|name| name == tab.session)
            && self
                .list_panes(&tab.session)
                .await?
                .iter()
                .any(|pane| pane.tab_id == tab.tab_id))
    }

    async fn wait_tab_closed(&self, tab: &Pane) -> Result<(), String> {
        while self.tab_open(tab).await? {
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        Ok(())
    }
}
