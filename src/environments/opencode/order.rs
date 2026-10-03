use super::*;

#[derive(Deserialize)]
struct Tab {
    tab_id: u32,
    position: usize,
}

impl Observer {
    pub(crate) async fn refresh_tab_order(&self, snapshot: &mut Snapshot) {
        let names = snapshot
            .sessions
            .iter()
            .flat_map(|session| &session.panes)
            .chain(snapshot.clients.iter().map(|client| &client.pane))
            .map(|pane| pane.session.clone())
            .collect::<BTreeSet<_>>();
        snapshot.zellij_tabs.retain(|name, _| names.contains(name));
        let mut tasks = tokio::task::JoinSet::new();
        for name in names {
            let observer = self.clone();
            tasks.spawn(async move {
                let result = observer.list_tab_positions(&name).await;
                (name, result)
            });
        }
        while let Some(result) = tasks.join_next().await {
            match result {
                Ok((name, Ok(tabs))) => {
                    snapshot.zellij_tabs.insert(name, tabs);
                }
                Ok((_, Err(_))) => {
                    // Ordering is optional; keep the last trustworthy positions on failure.
                }
                Err(error) => crate::diagnostics::record_error("Zellij tab ordering", &error),
            }
        }
    }

    async fn list_tab_positions(&self, name: &str) -> Result<BTreeMap<u32, usize>, String> {
        let output = zellij(
            &self.zellij,
            &[
                "--session".into(),
                name.into(),
                "action".into(),
                "list-tabs".into(),
                "--json".into(),
            ],
        )
        .await?;
        let tabs: Vec<Tab> =
            serde_json::from_str(&output).map_err(|_| "Invalid Zellij tab inventory".to_owned())?;
        Ok(tabs
            .into_iter()
            .map(|tab| (tab.tab_id, tab.position))
            .collect())
    }
}
