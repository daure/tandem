use super::AppService;

impl AppService {
    pub(crate) fn automation_enabled(&self) -> bool {
        self.environments.config.automation
    }

    pub(crate) fn require_automation(&self) -> Result<(), String> {
        if self.automation_enabled() {
            Ok(())
        } else {
            Err(crate::store::events::Error::Disabled.to_string())
        }
    }
}

#[cfg(test)]
#[path = "tests/automation.rs"]
mod tests;
