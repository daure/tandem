use tuicore::Notification;

use crate::store::environments::{EnvironmentSnapshot, Operation, OperationState};

use super::{instances, rows};

pub(super) struct Deletion {
    operation: Operation,
    succeeded: bool,
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
        })
    }

    pub(super) fn project(
        &mut self,
        snapshot: &mut EnvironmentSnapshot,
        lookup: impl FnOnce(&str) -> Result<Operation, String>,
        notifications: &mut Vec<Notification>,
    ) -> bool {
        if !self.succeeded {
            match lookup(&self.operation.id) {
                Ok(operation) if operation.state == OperationState::Running => {}
                Ok(operation) if operation.state == OperationState::Succeeded => {
                    self.succeeded = true;
                    let title = match operation.action.as_str() {
                        "delete_instance" => "Instance deleted",
                        "remove_template" => "Template deleted",
                        _ => "Instances purged",
                    };
                    notifications.push(Notification::success(title, &operation.name));
                }
                result => {
                    let error = match result {
                        Ok(operation) => {
                            operation.error.unwrap_or_else(|| "Operation failed".into())
                        }
                        Err(error) => error,
                    };
                    notifications.push(Notification::error(
                        "Delete failed",
                        format!("{}: {error}", self.operation.name),
                    ));
                    return false;
                }
            }
        }
        let before = (snapshot.instances.len(), snapshot.templates.len());
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
        !self.succeeded || before != (snapshot.instances.len(), snapshot.templates.len())
    }
}

impl super::App {
    pub(super) fn operation_accepted(&mut self, operation: Operation) {
        match operation.action.as_str() {
            "restart_instance" | "restart_service" => {
                self.restarts.push(operation.clone());
            }
            "create_instance" | "create_template" => {
                instances::select_created(&self.instances, &operation);
            }
            _ => {}
        }
        if let Some(deletion) = Deletion::new(operation) {
            self.deletions.push(deletion);
        }
        self.sync_environment();
    }

    pub(super) fn sync_environment(&mut self) -> bool {
        let mut snapshot = self.service.environment_snapshot();
        let mut notifications = Vec::new();
        self.restarts.retain(|pending| {
            let result = self.service.get_operation(&pending.id);
            let notification = match result {
                Ok(operation) if operation.state == OperationState::Running => return true,
                Ok(operation) if operation.state == OperationState::Succeeded => {
                    Notification::success("Restart completed", restart_target(&operation))
                }
                result => {
                    let error = match result {
                        Ok(operation) => operation.error.unwrap_or_else(|| "Restart failed".into()),
                        Err(error) => error,
                    };
                    Notification::error(
                        "Restart failed",
                        format!("{}: {error}", restart_target(pending)),
                    )
                }
            };
            notifications.push(notification);
            false
        });
        self.deletions.retain_mut(|deletion| {
            deletion.project(
                &mut snapshot,
                |id| self.service.get_operation(id),
                &mut notifications,
            )
        });
        for notification in notifications {
            self.notify(notification);
        }
        if snapshot == self.snapshot {
            return false;
        }
        instances::replace_rows(&self.instances, rows::from_snapshot(&snapshot));
        self.snapshot = snapshot;
        true
    }
}

fn restart_target(operation: &Operation) -> String {
    match &operation.service {
        Some(service) => format!("{}/{service}", operation.name),
        None => operation.name.clone(),
    }
}
