use std::{error::Error, fmt::Write, future::Future};

use serde::Serialize;
use serde_json::json;

use crate::{
    service::AppService,
    store::environments::{InstanceInspection, OperationState, RuntimeInventory, Template},
};

pub(super) fn list_instances(service: &AppService, json: bool) -> Result<(), Box<dyn Error>> {
    let inventory = query(service.list_instances())?;
    print_output(&inventory, json, || instances_text(&inventory))?;
    if let Some(error) = inventory.runtime_error {
        return Err(format!("runtime inventory incomplete: {error}").into());
    }
    Ok(())
}

pub(super) fn inspect_instance(
    service: &AppService,
    name: String,
    json: bool,
) -> Result<(), Box<dyn Error>> {
    let inspection = query(service.inspect_instance(name))?;
    print_output(&inspection, json, || instance_text(&inspection))?;
    if let Some(error) = inspection.runtime_error {
        return Err(format!("runtime inventory incomplete: {error}").into());
    }
    Ok(())
}

pub(super) fn list_templates(service: &AppService, json: bool) -> Result<(), Box<dyn Error>> {
    let templates = query(service.list_templates())?;
    print_output(&json!({"templates": templates}), json, || {
        templates_text(&templates)
    })
}

pub(super) fn inspect_template(
    service: &AppService,
    name: String,
    json: bool,
) -> Result<(), Box<dyn Error>> {
    let template = query(service.get_template(name))?;
    let status = service.status();
    let inspection = json!({
        "template": template,
        "templates_root": status.templates_root,
        "gateway_origin": status.gateway_origin,
    });
    print_output(&inspection, json, || {
        template_text(&template, &status.templates_root, &status.gateway_origin)
    })
}

fn query<T>(future: impl Future<Output = Result<T, String>>) -> Result<T, Box<dyn Error>> {
    tokio::runtime::Runtime::new()?
        .block_on(future)
        .map_err(Into::into)
}

fn print_output(
    value: &impl Serialize,
    json: bool,
    text: impl FnOnce() -> String,
) -> Result<(), Box<dyn Error>> {
    if json {
        println!("{}", serde_json::to_string_pretty(value)?);
    } else {
        print!("{}", text());
    }
    Ok(())
}

fn instances_text(inventory: &RuntimeInventory) -> String {
    let mut output = if inventory.instances.is_empty() {
        "No instances found.\n".into()
    } else {
        table(
            &["NAME", "TEMPLATE", "STATUS", "SERVICES", "WORKSPACE"],
            inventory.instances.iter().map(|instance| {
                let summary = &instance.summary;
                let status = match &summary.detail {
                    Some(detail) => format!("{}: {detail}", summary.label),
                    None => summary.label.clone(),
                };
                vec![
                    instance.name.clone(),
                    instance.template.clone(),
                    status,
                    if instance.workspace_only {
                        "—".into()
                    } else {
                        format!("{}/{}", summary.running, summary.expected)
                    },
                    instance.workspace.clone(),
                ]
            }),
        )
    };
    if !inventory.activities.is_empty() {
        output.push_str("\nActivities:\n");
        for activity in &inventory.activities {
            let state = activity.error.as_deref().unwrap_or_else(|| {
                if activity.active() {
                    "running"
                } else {
                    "finished"
                }
            });
            writeln!(
                output,
                "{}  {}  {}  {}",
                activity.id,
                activity.name,
                activity.action,
                single_line(state)
            )
            .unwrap();
        }
    }
    output
}

fn instance_text(inspection: &InstanceInspection) -> String {
    let mut output = format!(
        "Instance: {}\nObserved at (Unix seconds): {}\n",
        inspection.name, inspection.observed_at_unix_seconds
    );
    if let Some(instance) = &inspection.instance {
        writeln!(
            output,
            "Template: {}\nType: {}\nStatus: {}\nDescription: {}\nWorkspace: {}\nProject: {}\nTemplate directory: {}",
            instance.template,
            if instance.workspace_only { "Workspace" } else { "Compose" },
            instance.summary.label,
            instance.description,
            instance.workspace,
            instance.project,
            instance.template_directory,
        )
        .unwrap();
        if let Some(detail) = &instance.summary.detail {
            writeln!(output, "Status detail: {detail}").unwrap();
        }
        if let Some(issue) = &instance.runtime.issue {
            writeln!(output, "Runtime issue: {issue}").unwrap();
        }
        for repository in &instance.repositories {
            writeln!(
                output,
                "Repository {}: {}",
                repository.target, repository.path
            )
            .unwrap();
        }
        if instance.services.is_empty() {
            output.push_str("\nServices: None observed\n");
        }
        for service in &instance.services {
            writeln!(
                output,
                "\nService: {}\nStatus: {}\nDocker status: {}\nContainer: {}\nHealthcheck: {}\nSetup job: {}",
                service.name,
                service.summary.label,
                service.status,
                service.container_id,
                service.health.as_deref().unwrap_or("Not reported"),
                service.one_shot,
            )
            .unwrap();
            if let Some(detail) = &service.summary.detail {
                writeln!(output, "Status detail: {detail}").unwrap();
            }
            if let Some(image) = &service.image {
                writeln!(output, "Image: {image}").unwrap();
            }
            if let Some(url) = &service.url {
                writeln!(output, "URL: {url}").unwrap();
            }
            if let Some(checked_at) = service.runtime.readiness_checked_at {
                writeln!(
                    output,
                    "Readiness last passed at (Unix seconds): {checked_at}"
                )
                .unwrap();
            } else {
                output.push_str("Readiness: Not recorded\n");
            }
        }
    } else {
        output.push_str("Runtime instance: None observed; retained operation evidence follows\n");
    }
    if let Some(startup) = &inspection.startup {
        let state = match startup.state {
            OperationState::Running => "running",
            OperationState::Succeeded => "succeeded",
            OperationState::Failed => "failed",
        };
        writeln!(
            output,
            "\nLatest startup: {}\nOutcome: {state}\nElapsed: {} seconds",
            startup.id, startup.elapsed_seconds
        )
        .unwrap();
        for line in &startup.progress {
            writeln!(output, "  {line}").unwrap();
        }
        for warning in &startup.warnings {
            writeln!(output, "Warning: {warning}").unwrap();
        }
        if let Some(error) = &startup.error {
            writeln!(output, "Startup error: {error}").unwrap();
        }
    }
    for activity in &inspection.activities {
        writeln!(output, "\nActivity: {} ({})", activity.action, activity.id).unwrap();
        if let Some(error) = &activity.error {
            writeln!(output, "Activity error: {error}").unwrap();
        }
    }
    output
}

fn templates_text(templates: &[Template]) -> String {
    if templates.is_empty() {
        return "No templates found.\n".into();
    }
    table(
        &["NAME", "TYPE", "STATUS", "DESCRIPTION", "DIRECTORY"],
        templates.iter().map(|template| {
            vec![
                template.name.clone(),
                if template.error.is_some() {
                    "Unknown".into()
                } else {
                    template_kind(template).into()
                },
                template
                    .error
                    .as_ref()
                    .map(|error| format!("Invalid: {error}"))
                    .unwrap_or_else(|| "Valid".into()),
                template.manifest.description.clone(),
                template.directory.clone(),
            ]
        }),
    )
}

fn template_text(template: &Template, templates_root: &str, gateway_origin: &str) -> String {
    let mut output = format!(
        "Template: {}\nType: {}\nDirectory: {}\nTemplates root: {templates_root}\nConfigured gateway URL: {gateway_origin}\n",
        template.name,
        template_kind(template),
        template.directory,
    );
    writeln!(
        output,
        "Manifest file: {}\nCompose file: {}\nGuidance file: {}\nFiles directory: {}",
        if template.manifest_source.is_some() {
            &template.manifest_file
        } else {
            "None"
        },
        if template.workspace_only() {
            "None"
        } else {
            &template.compose_file
        },
        if template.guidance_source.is_some() {
            &template.guidance_file
        } else {
            "None"
        },
        template.files_directory.as_deref().unwrap_or("None"),
    )
    .unwrap();
    writeln!(
        output,
        "\nManifest:\n{}",
        serde_json::to_string_pretty(&template.manifest).unwrap()
    )
    .unwrap();
    if !template.workspace_only() {
        writeln!(output, "\nCompose:\n{}", template.compose_source).unwrap();
    }
    if let Some(guidance) = &template.guidance_source {
        writeln!(output, "\nGuidance:\n{guidance}").unwrap();
    }
    if template.files_directory.is_some() {
        writeln!(output, "\nFiles (copied into workspace root):").unwrap();
        for file in &template.files {
            let path = std::path::Path::new(&file.path);
            let depth = path.components().count().saturating_sub(1);
            writeln!(
                output,
                "{}{}{}",
                "  ".repeat(depth),
                path.file_name().unwrap_or_default().to_string_lossy(),
                if file.directory { "/" } else { "" }
            )
            .unwrap();
        }
    }
    output
}

fn template_kind(template: &Template) -> &'static str {
    if template.workspace_only() {
        "Workspace"
    } else {
        "Compose"
    }
}

fn table(headers: &[&str], rows: impl Iterator<Item = Vec<String>>) -> String {
    let rows: Vec<Vec<String>> = rows
        .map(|row| row.iter().map(|cell| single_line(cell)).collect())
        .collect();
    let widths: Vec<usize> = headers
        .iter()
        .enumerate()
        .map(|(column, header)| {
            rows.iter()
                .map(|row| row[column].chars().count())
                .chain([header.len()])
                .max()
                .unwrap()
        })
        .collect();
    let mut output = String::new();
    let header: Vec<String> = headers.iter().map(|header| (*header).into()).collect();
    for row in std::iter::once(&header).chain(rows.iter()) {
        for (column, cell) in row.iter().enumerate() {
            if column > 0 {
                output.push_str("  ");
            }
            if column + 1 == row.len() {
                output.push_str(cell);
            } else {
                write!(output, "{cell:<width$}", width = widths[column]).unwrap();
            }
        }
        output.push('\n');
    }
    output
}

fn single_line(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}
