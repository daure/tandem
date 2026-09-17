use std::{collections::HashSet, path::Path};

use crate::store::environments::{
    EnvironmentSnapshot, Instance, InstanceService, Operation, OperationState, ResourceUsage,
    Severity, StartupTiming, Status, UsageSummary,
};
use ratatui::{
    style::{Color, Style},
    text::{Line, Span, Text},
};

use super::{details, properties::Property};

const TEMPLATE_ICON: &str = "󰠲";
const GATEWAY_ICON: &str = "";
const PORT_ICON: &str = "󰈀";

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum Tone {
    #[default]
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

impl From<Severity> for Tone {
    fn from(severity: Severity) -> Self {
        match severity {
            Severity::Muted => Self::Muted,
            Severity::Info => Self::Info,
            Severity::Success => Self::Success,
            Severity::Warning => Self::Warning,
            Severity::Error => Self::Error,
        }
    }
}

fn status_icon(status: Status) -> &'static str {
    match status {
        Status::NotStarted | Status::Waiting => "",
        Status::Running => "",
        Status::Healthy => "",
        Status::Completed => "",
        Status::Unhealthy => "",
        Status::Paused => "",
        Status::Stopped => "",
        Status::Restarting => "",
        Status::Interrupted => "",
        Status::Failed | Status::ContainerError => "",
        Status::Degraded => "",
        Status::Unknown | Status::Stale | Status::Missing => "",
        _ => "⠋",
    }
}

struct InstanceSummary {
    label: String,
    tone: Tone,
    loading: bool,
    icon: &'static str,
    detail: Option<String>,
    detail_tone: Tone,
}

fn instance_summary(instance: &Instance, startup: Option<&StartupTiming>) -> InstanceSummary {
    let mut summary = instance.status_summary();
    if let Some(startup) = startup {
        summary.label = startup_label(
            if instance.services.is_empty() {
                "Creating"
            } else {
                "Starting"
            },
            startup,
        );
        summary.severity = Severity::Info;
        summary.busy = true;
    }
    let detail = if summary.expected > 0 {
        Some(format!(
            "{}/{} running{}",
            summary.running,
            summary.expected,
            summary
                .detail
                .as_ref()
                .map(|detail| format!(" · {detail}"))
                .unwrap_or_default()
        ))
    } else {
        summary.detail.clone()
    };
    InstanceSummary {
        label: summary.label,
        tone: summary.severity.into(),
        loading: summary.busy,
        icon: status_icon(summary.status),
        detail,
        detail_tone: summary.detail_severity.into(),
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

#[derive(Clone, Default, PartialEq, Eq)]
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
    pub service_name: Option<String>,
    pub workspace: Option<String>,
    pub alternate_background: bool,
    pub gateway_url: Option<String>,
    pub details: Vec<Property>,
    pub status_detail: Option<String>,
    pub detail_tone: Tone,
    pub metrics: UsageSummary,
    pub hide_resources: bool,
    pub can_start: bool,
    pub can_stop: bool,
    pub can_restart: bool,
}

impl Row {
    pub(super) fn height(&self) -> u16 {
        if self.hide_resources {
            self.label.lines().count().clamp(1, 2) as u16
        } else {
            2
        }
    }

    pub(super) fn search_text(&self) -> String {
        format!(
            "{} {}",
            self.label,
            self.status_detail.as_deref().unwrap_or_default()
        )
    }

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
                Style::default().fg(self.tone.color()),
            ));
        } else {
            first_line.push(Span::raw(first.to_owned()));
        }
        if let Some(detail) = &self.status_detail {
            first_line.push(Span::styled(
                format!(" · {}", detail.lines().next().unwrap_or_default()),
                Style::default().fg(self.detail_tone.color()),
            ));
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
        if self.hide_resources {
            return Text::default();
        }
        resource_text(&self.metrics)
    }

    pub(super) fn name_value(&self) -> Option<String> {
        self.service_name
            .clone()
            .or_else(|| self.instance.clone())
            .or_else(|| self.parent.is_none().then(|| self.template.clone()))
    }
}

pub(super) fn resource_text(metrics: &UsageSummary) -> Text<'static> {
    let memory = metrics
        .memory_bytes
        .map_or_else(|| "—".into(), details::memory);
    let cpu = metrics.cpu_basis_points;
    let suffix = |partial: bool| {
        if partial { " · partial" } else { "" }
    };
    Text::from(vec![
        Line::from(Span::styled(
            format!("󰑹 {memory}{}", suffix(metrics.memory_partial)),
            Style::default().fg(details::memory_tone(
                metrics.memory_bytes.map(|memory_bytes| ResourceUsage {
                    memory_bytes,
                    ..Default::default()
                }),
                metrics.memory_limit_bytes,
            )
            .color()),
        )),
        Line::from(Span::styled(
            format!(
                " {}{}",
                cpu.map_or_else(|| "—".into(), details::cpu),
                if metrics.paused {
                    " · paused"
                } else {
                    suffix(metrics.cpu_partial)
                }
            ),
            Style::default().fg(if cpu.is_some() {
                Tone::Normal
            } else {
                Tone::Muted
            }
            .color()),
        )),
    ])
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
        || [&row.search_text(), &row.resource_text().to_string()]
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
    from_snapshot_with_operations(snapshot, &[])
}

fn operation_progress<'a>(
    operations: &'a [Operation],
    action: &str,
    name: &str,
) -> Option<&'a str> {
    operations
        .iter()
        .find(|operation| {
            operation.state == OperationState::Running
                && operation.action == action
                && operation.name == name
        })?
        .progress
        .iter()
        .rev()
        .flat_map(|progress| progress.lines())
        .find(|line| !line.trim().is_empty())
        .map(str::trim)
}

pub(super) fn from_snapshot_with_operations(
    snapshot: &EnvironmentSnapshot,
    operations: &[Operation],
) -> Vec<Row> {
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
            alternate_background: false,
            gateway_url: None,
            details: details::template(template, instance_count),
            metrics: UsageSummary::instances(
                snapshot
                    .instances
                    .iter()
                    .filter(|instance| instance.template_directory == template.directory),
            ),
            ..Row::default()
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
                metrics: UsageSummary::instances(
                    snapshot
                        .instances
                        .iter()
                        .filter(|other| other.template_directory == instance.template_directory),
                ),
                ..Row::default()
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
        let secondary_label = operation_progress(operations, "create_instance", &instance.name)
            .map(str::to_owned)
            .unwrap_or_else(|| {
                workspace_label(&instance.workspace, snapshot.home_directory.as_deref())
            });
        rows.push(Row {
            id: instance_id.clone(),
            parent: Some(parent),
            label: format!("{} · {}\n{}", instance.name, summary.label, secondary_label),
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
            service_name: None,
            workspace: Some(instance.workspace.clone()),
            alternate_background: false,
            gateway_url: None,
            details: details::instance(instance),
            status_detail: summary.detail,
            detail_tone: summary.detail_tone,
            metrics: UsageSummary::instance(instance),
            hide_resources: false,
            can_start: instance.can_start() && !compose_file.is_empty(),
            can_stop: instance.can_stop(),
            can_restart: instance.can_restart(),
        });
        let completed = instance
            .services
            .iter()
            .filter(|service| {
                service.one_shot && service.status_summary().status == Status::Completed
            })
            .count();
        let setup_id = format!("setup:{}", instance.name);
        if completed > 0 {
            let mut group = rows.last().expect("instance row").clone();
            group.id = setup_id.clone();
            group.parent = Some(instance_id.clone());
            group.label = format!("Setup · {completed} completed");
            group.status = None;
            group.status_detail = None;
            group.icon = "";
            group.tone = Tone::Success;
            group.loading = false;
            group.instance = None;
            group.can_start = false;
            group.can_stop = false;
            group.can_restart = false;
            group.usage = None;
            group.metrics = UsageSummary::default();
            group.hide_resources = true;
            rows.push(group);
        }
        for service in &instance.services {
            let service_id = if service.runtime.replica > 1 {
                format!(
                    "service:{}:{}:{}",
                    instance.name, service.name, service.runtime.replica
                )
            } else {
                format!("service:{}:{}", instance.name, service.name)
            };
            let service_summary = service.status_summary();
            let route_detail = service
                .url
                .clone()
                .map(|url| {
                    service.port.map_or_else(
                        || format!("{GATEWAY_ICON} {url}"),
                        |port| format!("{GATEWAY_ICON} {url} · {PORT_ICON} {port}"),
                    )
                })
                .or_else(|| service.port.map(|port| format!("{PORT_ICON} {port}")));
            let label = if service.runtime.replica > 1 {
                format!("{} #{}", service.name, service.runtime.replica)
            } else {
                service.name.clone()
            };
            let label = format!("{label} · {}", service_summary.label);
            rows.push(Row {
                id: service_id.clone(),
                parent: Some(
                    if service.one_shot && service_summary.status == Status::Completed {
                        setup_id.clone()
                    } else {
                        instance_id.clone()
                    },
                ),
                label: route_detail
                    .as_ref()
                    .or(service.image.as_ref())
                    .map_or_else(|| label.clone(), |detail| format!("{label}\n{detail}")),
                status: Some(service_summary.label),
                icon: status_icon(service_summary.status),
                tone: service_summary.severity.into(),
                loading: service_summary.busy,
                usage: service.usage,
                memory_limit_bytes: service.memory_limit_bytes,
                template: instance.template.clone(),
                directory: directory.clone(),
                compose_file: compose_file.clone(),
                compose_source: compose_source.clone(),
                manifest_source: None,
                instance: None,
                service: (!service.one_shot).then(|| (instance.name.clone(), service.name.clone())),
                service_name: Some(service.name.clone()),
                workspace: None,
                alternate_background: false,
                gateway_url: service.url.clone(),
                details: details::service(service),
                status_detail: service_summary.detail,
                detail_tone: service_summary.detail_severity.into(),
                metrics: UsageSummary::service(service),
                hide_resources: service.one_shot,
                can_start: service.can_start(),
                can_stop: service.can_stop(),
                can_restart: service.can_restart(),
            });
        }
    }
    for activity in &snapshot.activities {
        if snapshot
            .instances
            .iter()
            .any(|instance| instance.name == activity.name)
        {
            continue;
        }
        let Some(name) = activity.template.as_ref() else {
            continue;
        };
        if !rows
            .iter()
            .any(|row| row.parent.is_none() && &row.template == name)
        {
            rows.push(Row {
                id: format!("template:missing:{name}"),
                label: format!("{name} [missing]"),
                template: name.clone(),
                icon: TEMPLATE_ICON,
                tone: Tone::Warning,
                details: vec![
                    Property::new("Status", "Missing template; retained operation evidence")
                        .tone(Tone::Warning),
                ],
                ..Default::default()
            });
        }
        let template = rows
            .iter()
            .find(|row| row.parent.is_none() && &row.template == name)
            .expect("activity template inserted");
        let mut row = template.clone();
        row.id = format!("operation:{}:{}", activity.id, activity.name);
        row.parent = Some(template.id.clone());
        let label = if activity.active() {
            activity.status().label().to_owned()
        } else {
            format!(
                "{} failed",
                if matches!(
                    activity.action.as_str(),
                    "delete_instance" | "delete_template" | "remove_template"
                ) {
                    "Cleanup"
                } else {
                    "Operation"
                }
            )
        };
        row.label = format!("{} · {label}", activity.name);
        let progress = operation_progress(operations, &activity.action, &activity.name).or_else(|| {
            (activity.active() && activity.action == "create_instance")
                .then_some("Preparing workspace and services")
        });
        if let Some(progress) = progress {
            row.label.push('\n');
            row.label.push_str(progress);
        }
        row.status = Some(label);
        row.status_detail = activity.error.clone();
        row.detail_tone = Tone::Error;
        row.icon = if activity.active() { "⠋" } else { "" };
        row.tone = if activity.active() {
            Tone::Info
        } else {
            Tone::Error
        };
        row.loading = activity.active();
        row.usage = None;
        row.metrics = UsageSummary::default();
        row.hide_resources = true;
        row.details = vec![
            Property::new("Operation", &activity.action),
            Property::new("Instance", &activity.name),
        ];
        if let Some(error) = &activity.error {
            row.details
                .push(Property::new("Error", error).tone(Tone::Error));
        }
        rows.push(row);
    }
    for row in &mut rows {
        if row.parent.is_none() {
            details::resources(&mut row.details, row.usage, row.memory_limit_bytes);
            details::usage_details(&mut row.details, &row.metrics);
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
