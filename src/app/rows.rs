use std::{collections::HashSet, path::Path};

use crate::store::environments::{EnvironmentSnapshot, InstanceService, ResourceUsage};
use ratatui::{
    style::{Color, Style},
    text::{Line, Span, Text},
};

use super::{details, properties::Property};

const TEMPLATE_ICON: &str = "󰠲";
const GATEWAY_ICON: &str = "󰖟";
const SERVICE_ICON: &str = "󰒋";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Tone {
    Normal,
    Muted,
    Success,
    Info,
    Error,
    Warning,
}

impl Tone {
    pub(super) fn color(self) -> Color {
        let theme = tuicore::theme();
        match self {
            Self::Normal => theme.text_fg(),
            Self::Muted => theme.muted_fg(),
            Self::Success => theme.success_fg(),
            Self::Info => theme.info_fg(),
            Self::Error => theme.error_fg(),
            Self::Warning => theme.warning_fg(),
        }
    }
}

fn service_tone(status: &str) -> Tone {
    match status {
        "healthy" => Tone::Success,
        "up" => Tone::Info,
        "boot" | "created" | "starting" | "restarting" | "removing" | "paused" | "exited 0" => {
            Tone::Muted
        }
        "unhealthy" => Tone::Error,
        value if value.starts_with("down (exit ") => Tone::Muted,
        _ => Tone::Warning,
    }
}

#[derive(Clone)]
pub(super) struct Row {
    pub id: String,
    pub parent: Option<String>,
    pub label: String,
    pub icon: &'static str,
    pub tone: Tone,
    pub loading: bool,
    pub usage: Option<ResourceUsage>,
    pub memory_limit_bytes: Option<u64>,
    pub template: String,
    pub directory: String,
    pub compose_file: String,
    pub compose_source: String,
    pub manifest_source: Option<String>,
    pub instance: Option<String>,
    pub workspace: Option<String>,
    pub running: bool,
    pub alternate_background: bool,
    pub gateway_url: Option<String>,
    pub details: Vec<Property>,
}

impl Row {
    pub(super) fn text(&self, spinner: &str) -> Text<'static> {
        let mut lines = self.label.lines();
        let icon = if self.loading { spinner } else { self.icon };
        let mut text = vec![Line::from(vec![
            Span::styled(format!("{icon} "), Style::default().fg(self.tone.color())),
            Span::raw(lines.next().unwrap_or_default().to_owned()),
        ])];
        text.extend(lines.map(|line| {
            Line::from(Span::styled(
                line.to_owned(),
                Style::default().fg(tuicore::theme().muted_fg()),
            ))
        }));
        Text::from(text)
    }

    pub(super) fn resource_text(&self) -> Text<'static> {
        let memory = self
            .usage
            .map_or_else(|| "—".into(), |usage| details::memory(usage.memory_bytes));
        let cpu = self.usage.and_then(|usage| usage.cpu_basis_points);
        Text::from(vec![
            Line::from(Span::styled(
                format!("󰑹 {memory}"),
                Style::default()
                    .fg(details::memory_tone(self.usage, self.memory_limit_bytes).color()),
            )),
            Line::from(Span::styled(
                format!(" {}", cpu.map_or_else(|| "—".into(), details::cpu)),
                Style::default().fg(if cpu.is_some() {
                    Tone::Normal
                } else {
                    Tone::Muted
                }
                .color()),
            )),
        ])
    }
}

fn workspace_label(workspace: &str, home: Option<&str>) -> String {
    match home.and_then(|home| Path::new(workspace).strip_prefix(home).ok()) {
        Some(relative) if relative.as_os_str().is_empty() => "~".into(),
        Some(relative) => format!("~/{}", relative.display()),
        None => workspace.to_owned(),
    }
}

fn tree_row_ids(
    rows: &[Row],
    parent: Option<&str>,
    visible_ids: Option<&HashSet<&str>>,
    ids: &mut Vec<String>,
) {
    for row in rows.iter().filter(|row| {
        row.parent.as_deref() == parent
            && visible_ids.is_none_or(|ids| ids.contains(row.id.as_str()))
    }) {
        ids.push(row.id.clone());
        tree_row_ids(rows, Some(&row.id), visible_ids, ids);
    }
}

fn matches_search(row: &Row, query: &str) -> bool {
    query.is_empty()
        || [row.label.as_str(), &row.resource_text().to_string()]
            .into_iter()
            .any(|value| tuicore::search_match(query, value, tuicore::SearchMode::Fuzzy).is_some())
}

fn searched_row_ids<'a>(rows: &'a [Row], query: &str) -> HashSet<&'a str> {
    let mut ids = HashSet::new();
    for row in rows.iter().filter(|row| matches_search(row, query)) {
        let mut current = Some(row);
        while let Some(row) = current {
            if !ids.insert(row.id.as_str()) {
                break;
            }
            current = row
                .parent
                .as_deref()
                .and_then(|parent| rows.iter().find(|row| row.id == parent));
        }
    }
    ids
}

pub(super) fn assign_alternating_backgrounds(rows: &mut [Row], query: &str) {
    let query = query.trim();
    let mut ids = Vec::with_capacity(rows.len());
    let visible_ids = (!query.is_empty()).then(|| searched_row_ids(rows, query));
    tree_row_ids(rows, None, visible_ids.as_ref(), &mut ids);
    for (index, id) in ids.into_iter().enumerate() {
        rows.iter_mut()
            .find(|row| row.id == id)
            .expect("tree row ID was collected from the same rows")
            .alternate_background = index % 2 == 0;
    }
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
            label: format!(
                "{}{}\n{instance_count} instance{}",
                template.name,
                if template.error.is_some() {
                    " · invalid template"
                } else {
                    ""
                },
                if instance_count == 1 { "" } else { "s" },
            ),
            icon: TEMPLATE_ICON,
            tone: if template.error.is_some() {
                Tone::Error
            } else {
                Tone::Normal
            },
            loading: false,
            usage: ResourceUsage::total(
                snapshot
                    .instances
                    .iter()
                    .filter(|instance| instance.template_directory == template.directory)
                    .flat_map(|instance| &instance.services),
            ),
            memory_limit_bytes: InstanceService::total_memory_limit(
                snapshot
                    .instances
                    .iter()
                    .filter(|instance| instance.template_directory == template.directory)
                    .flat_map(|instance| &instance.services),
            ),
            template: template.name.clone(),
            directory: template.directory.clone(),
            compose_file: template.compose_file.clone(),
            compose_source: template.compose_source.clone(),
            manifest_source: template.manifest_source.clone(),
            instance: None,
            workspace: None,
            running: false,
            alternate_background: false,
            gateway_url: None,
            details: details::template(template, instance_count),
        });
    }
    for instance in &snapshot.instances {
        let parent = format!("template:{}", instance.template_directory);
        if !rows.iter().any(|row| row.id == parent) {
            let instance_count = snapshot
                .instances
                .iter()
                .filter(|other| other.template_directory == instance.template_directory)
                .count();
            rows.push(Row {
                id: parent.clone(),
                parent: None,
                label: format!(
                    "{} [missing]\n{instance_count} instance{}",
                    instance.template,
                    if instance_count == 1 { "" } else { "s" }
                ),
                icon: TEMPLATE_ICON,
                tone: Tone::Warning,
                loading: false,
                usage: ResourceUsage::total(
                    snapshot
                        .instances
                        .iter()
                        .filter(|other| other.template_directory == instance.template_directory)
                        .flat_map(|other| &other.services),
                ),
                memory_limit_bytes: InstanceService::total_memory_limit(
                    snapshot
                        .instances
                        .iter()
                        .filter(|other| other.template_directory == instance.template_directory)
                        .flat_map(|other| &other.services),
                ),
                template: instance.template.clone(),
                directory: instance.template_directory.clone(),
                compose_file: String::new(),
                compose_source: String::new(),
                instance: None,
                running: false,
                alternate_background: false,
                manifest_source: None,
                workspace: None,
                gateway_url: None,
                details: vec![
                    Property::new("Template directory", &instance.template_directory),
                    Property::new(
                        "Status",
                        "Missing from template listing; instances discovered through Docker",
                    )
                    .tone(Tone::Warning),
                ],
            });
        }
        let template_row = rows
            .iter()
            .find(|row| row.id == parent)
            .expect("template row was inserted");
        let directory = template_row.directory.clone();
        let compose_file = template_row.compose_file.clone();
        let compose_source = template_row.compose_source.clone();
        let instance_id = format!("instance:{}", instance.name);
        let setup_failed = instance.startup_error().is_some();
        let loading = !setup_failed
            && (instance.pending
                || instance.services.iter().any(|service| {
                    matches!(
                        service.status.as_str(),
                        "created" | "restarting" | "boot" | "removing"
                    ) || service.one_shot && service.consumes_resources()
                }));
        let playing = instance
            .services
            .iter()
            .any(|service| matches!(service.status.as_str(), "up" | "healthy" | "unhealthy"));
        let failed = setup_failed
            || instance
                .services
                .iter()
                .any(|service| service_tone(&service.status) == Tone::Error);
        rows.push(Row {
            id: instance_id.clone(),
            parent: Some(parent),
            label: format!(
                "{}{}\n{}",
                instance.name,
                if setup_failed { " · setup failed" } else { "" },
                workspace_label(&instance.workspace, snapshot.home_directory.as_deref())
            ),
            icon: if setup_failed {
                ""
            } else if playing {
                ""
            } else {
                ""
            },
            tone: if loading {
                Tone::Muted
            } else if failed {
                Tone::Error
            } else if playing {
                Tone::Success
            } else {
                Tone::Muted
            },
            loading,
            usage: ResourceUsage::total(instance.services.iter()),
            memory_limit_bytes: InstanceService::total_memory_limit(instance.services.iter()),
            template: instance.template.clone(),
            directory: directory.clone(),
            compose_file: compose_file.clone(),
            compose_source: compose_source.clone(),
            manifest_source: None,
            instance: Some(instance.name.clone()),
            workspace: Some(instance.workspace.clone()),
            running: instance
                .services
                .iter()
                .any(InstanceService::consumes_resources),
            alternate_background: false,
            gateway_url: None,
            details: details::instance(instance),
        });
        for service in instance.services.iter().filter(|service| !service.one_shot) {
            let service_id = format!("service:{}:{}", instance.name, service.name);
            let route_detail = service
                .url
                .clone()
                .map(|url| {
                    service
                        .port
                        .map_or_else(|| url.clone(), |port| format!("{url} · port {port}"))
                })
                .or_else(|| service.port.map(|port| format!("port {port}")));
            let icon = if route_detail.is_some() {
                GATEWAY_ICON
            } else {
                SERVICE_ICON
            };
            rows.push(Row {
                id: service_id.clone(),
                parent: Some(instance_id.clone()),
                label: route_detail
                    .as_ref()
                    .or(service.image.as_ref())
                    .map_or_else(
                        || service.name.clone(),
                        |detail| format!("{}\n{detail}", service.name),
                    ),
                icon,
                tone: service_tone(&service.status),
                loading: false,
                usage: service.usage,
                memory_limit_bytes: service.memory_limit_bytes,
                template: instance.template.clone(),
                directory: directory.clone(),
                compose_file: compose_file.clone(),
                compose_source: compose_source.clone(),
                manifest_source: None,
                instance: None,
                workspace: None,
                running: false,
                alternate_background: false,
                gateway_url: service.url.clone(),
                details: details::service(service),
            });
        }
    }
    for row in &mut rows {
        if row.parent.is_none() {
            details::resources(&mut row.details, row.usage, row.memory_limit_bytes);
        }
    }
    assign_alternating_backgrounds(&mut rows, "");
    rows
}
