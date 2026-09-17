use std::{collections::HashSet, path::Path};

use crate::store::environments::{
    EnvironmentSnapshot, Instance, InstanceService, ResourceUsage, StartupTiming,
};
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

struct InstanceSummary {
    label: String,
    tone: Tone,
    loading: bool,
    icon: &'static str,
}

fn instance_summary(instance: &Instance, startup: Option<&StartupTiming>) -> InstanceSummary {
    let services = &instance.services;
    let running = services
        .iter()
        .filter(|service| !service.one_shot)
        .collect::<Vec<_>>();
    let ready = running
        .iter()
        .filter(|service| matches!(service.status.as_str(), "up" | "healthy"))
        .count();
    let stopped = running.is_empty()
        || running
            .iter()
            .all(|service| matches!(service.status.as_str(), "paused" | "down (exit 0)"));
    let starting = services.iter().any(|service| {
        matches!(
            service.status.as_str(),
            "created" | "restarting" | "boot" | "starting"
        ) || service.one_shot && service.consumes_resources()
    });
    let removing = services.iter().any(|service| service.status == "removing");
    let failed = instance.startup_error().is_some()
        || services.iter().any(|service| {
            service.status == "unhealthy"
                || service
                    .status
                    .strip_prefix("down (exit ")
                    .and_then(|value| value.strip_suffix(')'))
                    .is_some_and(|code| code != "0")
        });
    let (label, tone, loading, icon) = if removing {
        ("Removing".into(), Tone::Muted, true, "")
    } else if failed {
        ("Failed".into(), Tone::Error, false, "")
    } else if let Some(startup) = startup {
        if services.is_empty() {
            (startup_label("Creating", startup), Tone::Muted, true, "")
        } else {
            (startup_label("Starting", startup), Tone::Muted, true, "")
        }
    } else if instance.pending {
        ("Creating".into(), Tone::Muted, true, "")
    } else if starting && ready > 0 {
        ("Partially running".into(), Tone::Warning, false, "")
    } else if starting {
        ("Starting".into(), Tone::Muted, true, "")
    } else if stopped {
        ("Stopped".into(), Tone::Muted, false, "")
    } else if running.iter().all(|service| service.status == "healthy") {
        ("Healthy".into(), Tone::Success, false, "")
    } else if ready == running.len() {
        ("Running".into(), Tone::Success, false, "")
    } else {
        ("Partially running".into(), Tone::Warning, false, "")
    };
    InstanceSummary {
        label,
        tone,
        loading,
        icon,
    }
}

fn startup_label(status: &str, startup: &StartupTiming) -> String {
    let Some(estimate) = startup.estimate_milliseconds else {
        return status.into();
    };
    if startup.elapsed_milliseconds < estimate {
        return format!(
            "{status} · {}",
            format_seconds(estimate - startup.elapsed_milliseconds)
        );
    }
    if startup.elapsed_milliseconds == estimate {
        return format!("{status} · taking longer than usual");
    }
    format!(
        "{status} · {} over estimate",
        format_seconds(startup.elapsed_milliseconds - estimate)
    )
}

fn format_seconds(milliseconds: u64) -> String {
    format!("{}s", milliseconds.div_ceil(1_000))
}

#[derive(Clone)]
pub(super) struct Row {
    pub id: String,
    pub parent: Option<String>,
    pub label: String,
    pub status: Option<String>,
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
    pub service: Option<(String, String)>,
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
        let first = lines.next().unwrap_or_default();
        let mut first_line = vec![Span::styled(
            format!("{icon} "),
            Style::default().fg(self.tone.color()),
        )];
        if let Some(status) = &self.status {
            let name = first.strip_suffix(status).unwrap_or(first);
            first_line.push(Span::raw(name.to_owned()));
            first_line.push(Span::styled(
                status.clone(),
                Style::default().fg(tuicore::theme().muted_fg()),
            ));
        } else {
            first_line.push(Span::raw(first.to_owned()));
        }
        let mut text = vec![Line::from(first_line)];
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

fn template_summary(count: usize, average_milliseconds: Option<&u64>) -> String {
    let mut summary = format!(" {count}");
    if let Some(milliseconds) = average_milliseconds {
        let seconds = milliseconds.div_ceil(1_000);
        let duration = if seconds < 60 {
            format!("{seconds}s")
        } else {
            format!("{}m{:02}s", seconds / 60, seconds % 60)
        };
        summary.push_str(&format!(" ·  {duration}"));
    }
    summary
}

fn ready_services<'a>(
    snapshot: &'a EnvironmentSnapshot,
    directory: &'a str,
) -> impl Iterator<Item = &'a InstanceService> {
    snapshot
        .instances
        .iter()
        .filter(move |instance| {
            instance.template_directory == directory
                && !instance.pending
                && !snapshot.startup.contains_key(&instance.name)
        })
        .flat_map(|instance| &instance.services)
}

pub(super) fn from_snapshot(snapshot: &EnvironmentSnapshot) -> Vec<Row> {
    let mut rows = Vec::new();
    let mut instances = snapshot.instances.iter().collect::<Vec<_>>();
    instances.sort_by(|left, right| {
        left.template_directory
            .cmp(&right.template_directory)
            .then_with(|| left.name.cmp(&right.name))
    });
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
                "{}{}\n{}",
                template.name,
                if template.error.is_some() {
                    " · invalid template"
                } else {
                    ""
                },
                template_summary(
                    instance_count,
                    snapshot.startup_averages_milliseconds.get(&template.name)
                ),
            ),
            status: None,
            icon: TEMPLATE_ICON,
            tone: if template.error.is_some() {
                Tone::Error
            } else {
                Tone::Normal
            },
            loading: false,
            usage: ResourceUsage::total(ready_services(snapshot, &template.directory)),
            memory_limit_bytes: InstanceService::total_memory_limit(ready_services(
                snapshot,
                &template.directory,
            )),
            template: template.name.clone(),
            directory: template.directory.clone(),
            compose_file: template.compose_file.clone(),
            compose_source: template.compose_source.clone(),
            manifest_source: template.manifest_source.clone(),
            instance: None,
            service: None,
            workspace: None,
            running: false,
            alternate_background: false,
            gateway_url: None,
            details: details::template(template, instance_count),
        });
    }
    for instance in instances {
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
                    "{} [missing]\n{}",
                    instance.template,
                    template_summary(
                        instance_count,
                        snapshot
                            .startup_averages_milliseconds
                            .get(&instance.template)
                    )
                ),
                status: None,
                icon: TEMPLATE_ICON,
                tone: Tone::Warning,
                loading: false,
                usage: ResourceUsage::total(ready_services(snapshot, &instance.template_directory)),
                memory_limit_bytes: InstanceService::total_memory_limit(ready_services(
                    snapshot,
                    &instance.template_directory,
                )),
                template: instance.template.clone(),
                directory: instance.template_directory.clone(),
                compose_file: String::new(),
                compose_source: String::new(),
                instance: None,
                service: None,
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
        let summary = instance_summary(instance, snapshot.startup.get(&instance.name));
        rows.push(Row {
            id: instance_id.clone(),
            parent: Some(parent),
            label: format!(
                "{} · {}\n{}",
                instance.name,
                summary.label,
                workspace_label(&instance.workspace, snapshot.home_directory.as_deref())
            ),
            status: Some(summary.label),
            icon: summary.icon,
            tone: summary.tone,
            loading: summary.loading,
            usage: ResourceUsage::total(instance.services.iter()),
            memory_limit_bytes: InstanceService::total_memory_limit(instance.services.iter()),
            template: instance.template.clone(),
            directory: directory.clone(),
            compose_file: compose_file.clone(),
            compose_source: compose_source.clone(),
            manifest_source: None,
            instance: Some(instance.name.clone()),
            service: None,
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
                status: None,
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
                service: Some((instance.name.clone(), service.name.clone())),
                workspace: None,
                running: service.consumes_resources() || service.status == "restarting",
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
    rows.sort_by(|left, right| {
        let left_has_instances = snapshot
            .instances
            .iter()
            .any(|instance| instance.template_directory == left.directory);
        let right_has_instances = snapshot
            .instances
            .iter()
            .any(|instance| instance.template_directory == right.directory);
        right_has_instances
            .cmp(&left_has_instances)
            .then_with(|| left.template.cmp(&right.template))
            .then_with(|| left.directory.cmp(&right.directory))
    });
    assign_alternating_backgrounds(&mut rows, "");
    rows
}
