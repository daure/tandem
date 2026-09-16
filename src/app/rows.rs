use crate::store::environments::EnvironmentSnapshot;

#[derive(Clone)]
pub(super) struct Row {
    pub id: String,
    pub parent: Option<String>,
    pub label: String,
    pub status: String,
    pub template: String,
    pub directory: String,
    pub compose_file: String,
    pub compose_source: String,
    pub instance: Option<String>,
    pub details: String,
}

pub(super) fn from_snapshot(snapshot: &EnvironmentSnapshot) -> Vec<Row> {
    let mut rows = Vec::new();
    for template in &snapshot.templates {
        let instance_count = snapshot
            .instances
            .iter()
            .filter(|instance| instance.template_directory == template.directory)
            .count();
        rows.push(Row {
            id: format!("template:{}", template.directory),
            parent: None,
            label: template.name.clone(),
            template: template.name.clone(),
            directory: template.directory.clone(),
            compose_file: template.compose_file.clone(),
            compose_source: template.compose_source.clone(),
            instance: None,
            status: if template.error.is_some() {
                "invalid template".into()
            } else if instance_count > 0 {
                format!(
                    "{instance_count} instance{}",
                    if instance_count == 1 { "" } else { "s" }
                )
            } else {
                "template".into()
            },
            details: format!(
                "{}\n\n{}\n\nDirectory\n{}\n\nCompose file\n{}\n\nRouting manifest\n{}{}",
                template.name,
                template.manifest.description,
                template.directory,
                template.compose_file,
                template.manifest_file,
                template
                    .error
                    .as_ref()
                    .map(|error| format!("\n\nError\n{error}"))
                    .unwrap_or_default()
            ),
        });
    }
    for instance in &snapshot.instances {
        let parent = format!("template:{}", instance.template_directory);
        if !rows.iter().any(|row| row.id == parent) {
            rows.push(Row { id: parent.clone(), parent: None, label: format!("{} [missing]", instance.template),
                status: "template path stale".into(), template: instance.template.clone(), directory: instance.template_directory.clone(),
                compose_file: String::new(), compose_source: String::new(), instance: None,
                details: format!("Template directory recorded by Docker\n{}\n\nThe directory is not in the current template listing. Instances remain visible.", instance.template_directory) });
        }
        let template_row = rows
            .iter()
            .find(|row| row.id == parent)
            .expect("template row was inserted");
        let services: Vec<String> = instance
            .services
            .iter()
            .map(|service| {
                format!(
                    "{} · {}{}",
                    service.name,
                    service.status,
                    service
                        .url
                        .as_ref()
                        .map(|url| format!("\n{url}"))
                        .unwrap_or_default()
                )
            })
            .collect();
        rows.push(Row {
            id: format!("instance:{}", instance.name),
            parent: Some(parent),
            label: instance.name.clone(),
            status: instance
                .services
                .iter()
                .map(|service| service.status.as_str())
                .collect::<Vec<_>>()
                .join(", "),
            template: instance.template.clone(),
            directory: template_row.directory.clone(),
            compose_file: template_row.compose_file.clone(),
            compose_source: template_row.compose_source.clone(),
            instance: Some(instance.name.clone()),
            details: format!(
                "{}\n\nProject\n{}\n\nWorkspace\n{}\n\n{}",
                instance.name,
                instance.project,
                instance.workspace,
                services.join("\n\n")
            ),
        });
    }
    rows
}
