use super::*;

impl Observer {
    async fn live_zellij_sessions(&self) -> Result<Vec<String>, String> {
        // Short output hides EXITED status; reverse orders the newest session first.
        let sessions = zellij(
            &self.zellij,
            &[
                "list-sessions".into(),
                "--no-formatting".into(),
                "--reverse".into(),
            ],
        )
        .await?;
        let mut live = Vec::new();
        for line in sessions.lines().filter(|line| !line.trim().is_empty()) {
            let (name, status) = line
                .trim_end()
                .rsplit_once(" [Created ")
                .ok_or("Invalid Zellij session inventory")?;
            if name.is_empty() || name.len() > 200 || name.chars().any(char::is_control) {
                return Err("Invalid Zellij session name".into());
            }
            if status.ends_with("]") || status.ends_with("] (current)") {
                live.push(name.to_owned());
            } else if !status.ends_with("] (EXITED - attach to resurrect)") {
                return Err("Invalid Zellij session status".into());
            }
        }
        Ok(live)
    }

    pub(crate) async fn resolve_rule_destination(&self) -> Result<String, String> {
        self.live_zellij_sessions()
            .await?
            .into_iter()
            .next()
            .ok_or_else(|| {
                "Rule destination unavailable: no live Zellij session; start a session and retry the acceptance".into()
            })
    }

    pub(crate) async fn validate_rule_destination(&self, name: &str) -> Result<(), String> {
        if self
            .live_zellij_sessions()
            .await?
            .iter()
            .any(|session| session == name)
        {
            Ok(())
        } else {
            Err(format!(
                "Rule destination unavailable: Zellij session {name:?} is not live"
            ))
        }
    }

    pub(super) async fn resolve_launch_session(&self, current: &str) -> Result<String, String> {
        let sessions = self.live_zellij_sessions().await?;
        sessions
            .iter()
            .find(|session| session.as_str() == current)
            .or_else(|| sessions.first())
            .cloned()
            .ok_or_else(|| "OpenCode destination unavailable: no live Zellij session; start a session and retry".into())
    }
}
