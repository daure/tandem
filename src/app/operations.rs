use tuicore::Notification;

use crate::store::environments::{EnvironmentSnapshot, Operation, OperationState};

use super::instances;

pub(super) struct Deletion {
    operation: Operation,
    succeeded: bool,
    workspaces: Vec<String>,
}

impl Deletion {
    pub(super) fn new(operation: Operation) -> Option<Self> {
        matches!(
            operation.action.as_str(),
            "delete_instance" | "delete_template" | "remove_template"
        )
        .then_some(Self {
            operation,
            succeeded: false,
            workspaces: Vec::new(),
        })
    }

    fn remember_workspaces(&mut self, snapshot: &EnvironmentSnapshot) {
        for instance in &snapshot.instances {
            let targeted = if self.operation.action == "delete_instance" {
                instance.name == self.operation.name
            } else {
                instance.template == self.operation.name
            };
            if targeted && !self.workspaces.contains(&instance.workspace) {
                self.workspaces.push(instance.workspace.clone());
            }
        }
    }

    pub(super) fn project(
        &mut self,
        snapshot: &mut EnvironmentSnapshot,
        lookup: impl FnOnce(&str) -> Result<Operation, String>,
        notifications: &mut Vec<Notification>,
    ) -> bool {
        self.remember_workspaces(snapshot);
        if !self.succeeded {
            match lookup(&self.operation.id) {
                Ok(operation) if operation.state == OperationState::Running => {}
                Ok(operation) if operation.state == OperationState::Succeeded => {
                    self.succeeded = true;
                    let title = match operation.action.as_str() {
                        "delete_instance" => "Instance purged",
                        "remove_template" => "Template deleted",
                        _ => "Instances purged",
                    };
                    notifications.push(if operation.warnings.is_empty() {
                        Notification::success(title, &operation.name)
                    } else {
                        Notification::warning(title, operation.warnings.join("\n"))
                    });
                }
                result => {
                    let error = match result {
                        Ok(operation) => {
                            let mut error =
                                operation.error.unwrap_or_else(|| "Operation failed".into());
                            for warning in operation.warnings {
                                error.push_str(&format!("\n{warning}"));
                            }
                            error
                        }
                        Err(error) => error,
                    };
                    notifications.push(Notification::error(
                        "Purge failed",
                        format!("{}: {error}", self.operation.name),
                    ));
                    return false;
                }
            }
        }
        let before = (
            snapshot.instances.len(),
            snapshot.templates.len(),
            snapshot.activities.len(),
        );
        snapshot.activities.retain(|activity| {
            if self.operation.action == "delete_instance" {
                activity.name != self.operation.name
            } else {
                activity.template.as_deref() != Some(&self.operation.name)
            }
        });
        snapshot.instances.retain(|instance| {
            if self.operation.action == "delete_instance" {
                instance.name != self.operation.name
            } else {
                instance.template != self.operation.name
            }
        });
        if self.operation.action == "remove_template" {
            snapshot
                .templates
                .retain(|template| template.name != self.operation.name);
        }
        // Keep successful deletes hidden until the inventory catches up.
        !self.succeeded
            || before
                != (
                    snapshot.instances.len(),
                    snapshot.templates.len(),
                    snapshot.activities.len(),
                )
    }

    fn contains_directory(&self, directory: &str) -> bool {
        crate::store::opencode::workspace_owner(
            directory,
            self.workspaces
                .iter()
                .map(|path| (path.as_str(), path.as_str())),
        )
        .is_some()
    }

    fn awaiting_clients(&self, snapshot: &crate::store::opencode::Snapshot) -> bool {
        self.succeeded
            && (snapshot
                .sessions
                .iter()
                .any(|session| session.attached() && self.contains_directory(&session.directory))
                || snapshot
                    .clients
                    .iter()
                    .any(|client| self.contains_directory(&client.directory)))
    }

    pub(super) fn hide_opencode(&self, snapshot: &mut crate::store::opencode::Snapshot) {
        snapshot
            .directories
            .retain(|directory| !self.contains_directory(directory));
        snapshot
            .sessions
            .retain(|session| !self.contains_directory(&session.directory));
        snapshot
            .clients
            .retain(|client| !self.contains_directory(&client.directory));
        snapshot
            .resources
            .retain(|process| !self.contains_directory(&process.directory));
    }
}

impl super::App {
    pub(super) fn operation_accepted(&mut self, operation: Operation) {
        match operation.action.as_str() {
            "restart_instance" | "restart_service" | "start_service" | "stop_service"
            | "stop_instance" | "stop_template" => {
                self.container_operations.push(operation.clone());
            }
            "create_instance" => {
                self.container_operations.push(operation.clone());
                instances::select_created(&self.instances, &operation);
            }
            "create_template" => {
                instances::select_created(&self.instances, &operation);
            }
            _ => {}
        }
        if let Some(mut deletion) = Deletion::new(operation) {
            deletion.remember_workspaces(&self.snapshot);
            self.deletions.push(deletion);
        }
        self.sync_environment();
    }

    pub(super) fn sync_environment(&mut self) -> bool {
        let mut snapshot = self.service.environment_snapshot();
        let mut notifications = Vec::new();
        self.container_operations.retain(|pending| {
            let action = match pending.action.as_str() {
                "start_service" | "create_instance" => "Start",
                "stop_service" | "stop_instance" | "stop_template" => "Stop",
                _ => "Restart",
            };
            let result = self.service.get_operation(&pending.id);
            let notification = match result {
                Ok(operation) if operation.state == OperationState::Running => return true,
                Ok(operation) if operation.state == OperationState::Succeeded => {
                    Notification::success(
                        format!("{action} completed"),
                        operation_target(&operation),
                    )
                }
                result => {
                    let error = match result {
                        Ok(operation) => operation
                            .error
                            .unwrap_or_else(|| format!("{action} failed")),
                        Err(error) => error,
                    };
                    Notification::error(
                        format!("{action} failed"),
                        format!("{}: {error}", operation_target(pending)),
                    )
                }
            };
            notifications.push(notification);
            false
        });
        let opencode = self.service.opencode_snapshot();
        self.deletions.retain_mut(|deletion| {
            deletion.project(
                &mut snapshot,
                |id| self.service.get_operation(id),
                &mut notifications,
            ) || deletion.awaiting_clients(&opencode)
        });
        for notification in notifications {
            self.notify(notification);
        }
        self.update_snapshot(snapshot)
    }
}

fn operation_target(operation: &Operation) -> String {
    match &operation.service {
        Some(service) => format!("{}/{service}", operation.name),
        None => operation.name.clone(),
    }
}
