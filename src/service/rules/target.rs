use super::{Acceptance, AppService, DispatchStatus, Startup};
use crate::environments::gateway::Lock;

impl AppService {
    pub(super) fn begin_rule_instance(
        &self,
        acceptance: &mut Acceptance,
        lock: Lock,
    ) -> Result<(), String> {
        let description = acceptance.instance_description();
        let operation = self.environments.begin_instance(
            &acceptance.instance,
            acceptance.template().to_owned(),
            Some(&description),
        )?;
        acceptance
            .operation_id
            .get_or_insert_with(|| operation.id.clone());
        acceptance.status = DispatchStatus::Provisioning;
        self.rules
            .store
            .update(acceptance, true)
            .map_err(|error| error.to_string())?;
        self.schedule_operation(
            operation,
            600,
            Startup {
                origin_operation_id: acceptance.operation_id.clone(),
                start_instance: acceptance.rule.definition.start_instance,
                instance_lock: Some(lock),
                description: Some(description),
                ..Default::default()
            },
        );
        Ok(())
    }

    pub(super) fn advance_target_dispatch(
        &self,
        acceptance: &mut Acceptance,
        available: &mut usize,
    ) -> Result<(), String> {
        if *available == 0 {
            return Ok(());
        }
        let Some((existing, lock)) = self.environments.admit_rule_target(&acceptance.instance)?
        else {
            return Ok(());
        };
        if !self
            .rules
            .store
            .admit_dispatch(acceptance.id)
            .map_err(|error| error.to_string())?
        {
            return Ok(());
        }
        *available -= 1;
        if !self.opencode_enabled() {
            return Err("OpenCode integration is disabled".into());
        }
        let destination = self
            .runtime
            .handle()
            .block_on(self.opencode.observer.resolve_rule_destination())?;
        if let Some(origin) = &acceptance.operation_id {
            self.rules
                .store
                .ensure_retry_instance_name(&acceptance.instance, acceptance.template(), origin)
                .map_err(|error| error.to_string())?;
            return self.begin_rule_instance(acceptance, lock);
        }
        let Some(instance) = existing else {
            self.rules
                .store
                .ensure_fresh_instance_name(&acceptance.instance)
                .map_err(|error| error.to_string())?;
            return self.begin_rule_instance(acceptance, lock);
        };
        acceptance.target_workspace_identity = Some(crate::environments::workspace_identity(
            &self.environments.config,
            &instance,
        )?);
        if acceptance.rule.definition.session_name.is_some() {
            return self.launch_named_rule_session(acceptance, &instance, &destination, true);
        }
        acceptance.status = DispatchStatus::Launching;
        acceptance.launch_started_at = Some(chrono::Utc::now().to_rfc3339());
        self.rules
            .store
            .update(acceptance, true)
            .map_err(|error| error.to_string())?;
        let pane = self.runtime.handle().block_on(
            self.opencode.observer.new_rule_session(
                &instance.workspace,
                &instance.name,
                &destination,
                &acceptance.launch_definition(),
                acceptance
                    .resolved_prompt
                    .as_deref()
                    .ok_or("resolved prompt unavailable")?,
                Some(instance.session_instructions()),
            ),
        )?;
        acceptance.pane = Some(pane);
        self.rules
            .store
            .update(acceptance, false)
            .map_err(|error| error.to_string())?;
        drop(lock);
        Ok(())
    }
}
