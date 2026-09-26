use std::collections::HashSet;

use crate::store::environments::{
    EnvironmentSnapshot, Instance, InstanceService, Operation, OperationState, ResourceUsage,
    Severity, StartupKind, StartupTiming, Status, Template, UsageSummary,
};
use ratatui::{
    style::{Color, Style},
    text::{Line, Span, Text},
};

use super::{details, properties::Property};

mod setup;

const TEMPLATE_ICON: &str = "󰠲";
const GATEWAY_ICON: &str = "";
const PORT_ICON: &str = "󰈀";
const COLD_START_ICON: &str = "󰜗";
const HOT_START_ICON: &str = "󰈸";

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
        Status::Healthy | Status::WorkspaceReady => "",
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
    InstanceSummary {
        label: summary.label,
        tone: summary.severity.into(),
        loading: summary.busy,
        icon: status_icon(summary.status),
        detail: summary.detail,
        detail_tone: summary.detail_severity.into(),
    }
}

fn startup_label(status: &str, startup: &StartupTiming) -> String {
    let status = format!(
        "{} {status}",
        match startup.kind {
            StartupKind::Cold => COLD_START_ICON,
            StartupKind::Hot => HOT_START_ICON,
        }
    );
    let Some(estimate) = startup.estimate_milliseconds else {
        return status;
    };
    if startup.elapsed_milliseconds < estimate {
        return format!(
            "{status} {}",
            format_seconds(estimate - startup.elapsed_milliseconds)
        );
    }
    if startup.elapsed_milliseconds == estimate {
        return format!("{status} taking longer than usual");
    }
    format!(
        "{status} {} over estimate",
        format_seconds(startup.elapsed_milliseconds - estimate)
    )
}

fn format_seconds(milliseconds: u64) -> String {
    format!("{}s", milliseconds.div_ceil(1_000))
}

#[derive(Clone, Default, PartialEq, Eq)]
pub(super) struct Row {
    pub opencode: Option<super::opencode::Target>,
    pub id: String,
    pub parent: Option<String>,
    pub label: String,
    pub template_capabilities: String,
    pub status: Option<String>,
    pub icon: &'static str,
    pub tone: Tone,
    pub loading: bool,
    pub secondary_icon: &'static str,
    pub secondary_tone: Tone,
    pub secondary_loading: bool,
    pub activity_timer: Option<(u64, u64)>,
    pub usage: Option<ResourceUsage>,
    pub memory_limit_bytes: Option<u64>,
    pub template: String,
    pub directory: String,
    pub compose_file: String,
    pub compose_source: String,
    pub template_available: bool,
    pub checkout_path: Option<String>,
    pub informational: bool,
    pub cleanup_target: Option<String>,
    pub manifest_source: Option<String>,
    pub guidance_source: Option<String>,
    pub instance: Option<String>,
    pub description: String,
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
        if self.instance.is_some() {
            2
        } else if self.hide_resources {
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

    pub(super) fn text(&self, spinner: &str, available_width: Option<u16>) -> Text<'static> {
        let mut lines = self.label.lines();
        let icon = if self.loading { spinner } else { self.icon };
        let first = lines.next().unwrap_or_default();
        let mut first_line = Vec::new();
        if !icon.is_empty() {
            first_line.push(Span::styled(
                format!("{icon} "),
                Style::default().fg(self.tone.color()),
            ));
        }
        if let Some(status) = &self.status {
            let name = first.strip_suffix(status).unwrap_or(first);
            first_line.push(Span::raw(name.to_owned()));
            first_line.push(Span::styled(
                status.clone(),
                Style::default().fg(self.tone.color()),
            ));
        } else if self.informational {
            first_line.push(Span::styled(
                first.to_owned(),
                Style::default().fg(self.tone.color()),
            ));
        } else {
            first_line.push(Span::raw(first.to_owned()));
        }
        if !self.template_capabilities.is_empty() {
            first_line.push(Span::styled(
                format!(" {}", self.template_capabilities),
                Style::default().fg(tuicore::theme().muted_fg()),
            ));
        }
        if let Some(detail) = &self.status_detail {
            first_line.push(Span::styled(
                " · ",
                Style::default().fg(Tone::Normal.color()),
            ));
            first_line.push(Span::styled(
                detail.lines().next().unwrap_or_default().to_owned(),
                Style::default().fg(self.detail_tone.color()),
            ));
        }
        let mut text = vec![Line::from(first_line)];
        if self.instance.is_some() {
            let description = self.description.trim();
            let description_tone = if description.is_empty() {
                tuicore::theme().subtle_fg()
            } else {
                tuicore::theme().muted_fg()
            };
            text.push(Line::from(Span::styled(
                if description.is_empty() {
                    "(no description)".to_owned()
                } else {
                    description.to_owned()
                },
                Style::default().fg(description_tone),
            )));
        } else {
            text.extend(lines.enumerate().map(|(index, line)| {
                if index == 0 && (self.secondary_loading || !self.secondary_icon.is_empty()) {
                    let icon = if self.secondary_loading {
                        spinner
                    } else {
                        self.secondary_icon
                    };
                    let icon = format!("{icon} ");
                    let icon_width = Line::from(icon.as_str()).width();
                    let line = available_width.map_or_else(
                        || line.to_owned(),
                        |width| {
                            truncate_with_ellipsis(
                                line,
                                usize::from(width).saturating_sub(icon_width),
                            )
                        },
                    );
                    Line::from(vec![
                        Span::styled(icon, Style::default().fg(self.secondary_tone.color())),
                        Span::styled(line, Style::default().fg(tuicore::theme().muted_fg())),
                    ])
                } else {
                    Line::from(Span::styled(
                        line.to_owned(),
                        Style::default().fg(tuicore::theme().muted_fg()),
                    ))
                }
            }));
        }
        Text::from(text)
    }

    pub(super) fn resource_text(&self) -> Text<'static> {
        self.resource_text_with_spinner("…")
    }

    pub(super) fn resource_text_with_spinner(&self, spinner: &str) -> Text<'static> {
        if self.hide_resources {
            return Text::default();
        }
        resource_text_with_spinner(&self.metrics, None, spinner)
    }

    pub(super) fn memory_text(&self) -> Line<'static> {
        let waiting = self.metrics.memory_waiting || self.metrics.cpu_waiting;
        let complete =
            self.metrics.memory_bytes.is_some() && self.metrics.cpu_basis_points.is_some();
        if self.hide_resources || self.metrics.memory_bytes.is_none() || waiting && !complete {
            return Line::default();
        }
        resource_memory_text_with_spinner(&self.metrics, None, None)
    }

    pub(super) fn cpu_text_with_spinner(&self, spinner: &str) -> Line<'static> {
        if self.hide_resources {
            return Line::default();
        }
        let waiting = self.metrics.memory_waiting || self.metrics.cpu_waiting;
        let complete =
            self.metrics.memory_bytes.is_some() && self.metrics.cpu_basis_points.is_some();
        if waiting && !complete {
            return Line::from(Span::styled(
                spinner.to_owned(),
                Style::default().fg(Tone::Muted.color()),
            ));
        }
        if self.metrics.cpu_basis_points.is_none() {
            return Line::default();
        }
        resource_cpu_text_with_spinner(&self.metrics, waiting.then_some(spinner))
    }

    pub(super) fn name_value(&self) -> Option<String> {
        self.checkout_path
            .clone()
            .or_else(|| self.service_name.clone())
            .or_else(|| self.instance.clone())
            .or_else(|| self.cleanup_target.clone())
            .or_else(|| self.parent.is_none().then(|| self.template.clone()))
    }
}

fn truncate_with_ellipsis(value: &str, max_width: usize) -> String {
    if Line::from(value).width() <= max_width {
        return value.to_owned();
    }
    if max_width <= 3 {
        return ".".repeat(max_width);
    }
    let content_width = max_width - 3;
    let mut result = String::new();
    let mut width = 0;
    for character in value.chars() {
        let character_width = Line::from(character.to_string()).width();
        if width + character_width > content_width {
            break;
        }
        result.push(character);
        width += character_width;
    }
    result.push_str("...");
    result
}

pub(super) fn resource_text_with_spinner(
    metrics: &UsageSummary,
    available_memory_bytes: Option<u64>,
    spinner: &str,
) -> Text<'static> {
    Text::from(vec![
        resource_memory_text_with_spinner(
            metrics,
            available_memory_bytes,
            metrics.memory_waiting.then_some(spinner),
        ),
        resource_cpu_text_with_spinner(metrics, metrics.cpu_waiting.then_some(spinner)),
    ])
}

pub(super) fn resource_text_with_single_spinner(
    metrics: &UsageSummary,
    available_memory_bytes: Option<u64>,
    spinner: &str,
) -> Text<'static> {
    Text::from(vec![
        resource_memory_text_with_spinner(metrics, available_memory_bytes, None),
        resource_cpu_text_with_spinner(
            metrics,
            (metrics.memory_waiting || metrics.cpu_waiting).then_some(spinner),
        ),
    ])
}

fn resource_memory_text_with_spinner(
    metrics: &UsageSummary,
    available_memory_bytes: Option<u64>,
    spinner: Option<&str>,
) -> Line<'static> {
    let memory_bytes = metrics.memory_bytes;
    let memory = memory_bytes.map_or_else(|| "—".into(), details::memory);
    let mut memory = available_memory_bytes.map_or(memory.clone(), |available| {
        format!("{memory} / {}", details::memory(available))
    });
    if let Some(spinner) = spinner {
        memory.push_str(&format!(" {spinner}"));
    }
    let tone = if metrics.memory_waiting {
        Tone::Muted
    } else {
        details::memory_tone(
            memory_bytes.map(|memory_bytes| ResourceUsage {
                memory_bytes,
                ..Default::default()
            }),
            metrics.memory_limit_bytes,
        )
    };
    Line::from(Span::styled(memory, Style::default().fg(tone.color())))
}

fn resource_cpu_text_with_spinner(metrics: &UsageSummary, spinner: Option<&str>) -> Line<'static> {
    let cpu = metrics.cpu_basis_points;
    let mut cpu_text = cpu.map_or_else(|| "—".into(), details::cpu);
    if let Some(spinner) = spinner {
        cpu_text.push_str(&format!(" {spinner}"));
    }
    Line::from(Span::styled(
        format!(
            "{}{}",
            cpu_text,
            if metrics.paused { " · paused" } else { "" }
        ),
        Style::default().fg(if cpu.is_some() && !metrics.cpu_waiting {
            Tone::Normal
        } else {
            Tone::Muted
        }
        .color()),
    ))
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

pub(super) fn compact_duration(milliseconds: u64) -> String {
    let seconds = milliseconds.div_ceil(1_000);
    if seconds < 60 {
        format!("{seconds}s")
    } else {
        format!("{}m{:02}s", seconds / 60, seconds % 60)
    }
}

fn template_summary(
    count: usize,
    total_count: Option<usize>,
    cold_average_milliseconds: Option<&u64>,
    hot_average_milliseconds: Option<&u64>,
) -> String {
    let count = total_count
        .filter(|total| *total != count)
        .map_or_else(|| count.to_string(), |total| format!("{count}/{total}"));
    let mut summary = format!(" {count}");
    if let Some(milliseconds) = cold_average_milliseconds {
        summary.push_str(&format!(
            " · {COLD_START_ICON} {}",
            compact_duration(*milliseconds)
        ));
    }
    if let Some(milliseconds) = hot_average_milliseconds {
        summary.push_str(&format!(
            " · {HOT_START_ICON} {}",
            compact_duration(*milliseconds)
        ));
    }
    summary
}

fn template_capabilities(template: &Template) -> String {
    if template.error.is_some() {
        return String::new();
    }
    let mut icons = Vec::new();
    if !template.workspace_only() {
        icons.push("󰡨");
    }
    match template.manifest.repositories.len() {
        0 => {}
        1 => icons.push("󰳏"),
        _ => icons.push("󰳐"),
    }
    if !template.manifest.routes.is_empty() {
        icons.push(PORT_ICON);
    }
    if template.guidance_source.is_some() {
        icons.push("󱓷");
    }
    icons.join(" ")
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

#[cfg(test)]
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
    from_snapshot_with_operations_and_totals(snapshot, operations, None)
}

pub(super) fn from_filtered_snapshot_with_operations(
    snapshot: &EnvironmentSnapshot,
    operations: &[Operation],
    full_snapshot: &EnvironmentSnapshot,
) -> Vec<Row> {
    from_snapshot_with_operations_and_totals(snapshot, operations, Some(full_snapshot))
}

fn from_snapshot_with_operations_and_totals(
    snapshot: &EnvironmentSnapshot,
    operations: &[Operation],
    full_snapshot: Option<&EnvironmentSnapshot>,
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
            template_capabilities: template_capabilities(template),
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
                    full_snapshot.map(|snapshot| {
                        snapshot
                            .instances
                            .iter()
                            .filter(|instance| instance.template_directory == template.directory)
                            .count()
                    }),
                    snapshot
                        .cold_startup_averages_milliseconds
                        .get(&template.name),
                    snapshot
                        .hot_startup_averages_milliseconds
                        .get(&template.name),
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
            template_available: true,
            hide_resources: template.workspace_only(),
            manifest_source: template.manifest_source.clone(),
            guidance_source: template.guidance_source.clone(),
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
                        full_snapshot.map(|snapshot| {
                            snapshot
                                .instances
                                .iter()
                                .filter(|other| {
                                    other.template_directory == instance.template_directory
                                })
                                .count()
                        }),
                        snapshot
                            .cold_startup_averages_milliseconds
                            .get(&instance.template),
                        snapshot
                            .hot_startup_averages_milliseconds
                            .get(&instance.template),
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
        let template_available = template_row.template_available;
        let instance_id = format!("instance:{}", instance.name);
        let summary = instance_summary(instance, snapshot.startup.get(&instance.name));
        let operation_progress =
            operation_progress(operations, "create_instance", &instance.name).map(str::to_owned);
        rows.push(Row {
            id: instance_id.clone(),
            opencode: None,
            template_capabilities: String::new(),
            parent: Some(parent),
            label: format!("{} · {}", instance.name, summary.label),
            status: Some(summary.label),
            icon: summary.icon,
            tone: summary.tone,
            loading: summary.loading,
            secondary_icon: "",
            secondary_tone: Tone::default(),
            secondary_loading: false,
            activity_timer: None,
            usage: ResourceUsage::total(instance.services.iter()),
            memory_limit_bytes: InstanceService::total_memory_limit(instance.services.iter()),
            template: instance.template.clone(),
            directory: directory.clone(),
            compose_file: compose_file.clone(),
            compose_source: compose_source.clone(),
            template_available,
            checkout_path: None,
            informational: false,
            cleanup_target: None,
            manifest_source: None,
            instance: Some(instance.name.clone()),
            description: instance.description.clone(),
            guidance_source: None,
            service: None,
            service_name: None,
            workspace: Some(instance.workspace.clone()),
            alternate_background: false,
            gateway_url: None,
            details: details::instance(instance),
            status_detail: operation_progress.clone().or(summary.detail),
            detail_tone: if operation_progress.is_some() {
                Tone::Muted
            } else {
                summary.detail_tone
            },
            metrics: UsageSummary::instance(instance),
            hide_resources: instance.workspace_only,
            can_start: instance.can_start() && !compose_file.is_empty(),
            can_stop: instance.can_stop(),
            can_restart: instance.can_restart(),
        });
        let setup_id = format!("setup:{}", instance.name);
        let instance_row = rows.last().expect("instance row").clone();
        setup::append(&mut rows, instance);
        let service_count = instance
            .services
            .iter()
            .filter(|service| !service.one_shot)
            .count();
        let services = setup::services(&instance_row, instance, service_count);
        let services_id = services.id.clone();
        rows.push(services);
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
                opencode: None,
                template_capabilities: String::new(),
                parent: Some(if service.one_shot {
                    setup_id.clone()
                } else {
                    services_id.clone()
                }),
                label: route_detail
                    .as_ref()
                    .or(service.image.as_ref())
                    .map_or_else(|| label.clone(), |detail| format!("{label}\n{detail}")),
                status: Some(service_summary.label),
                icon: status_icon(service_summary.status),
                tone: service_summary.severity.into(),
                loading: service_summary.busy,
                secondary_icon: "",
                secondary_tone: Tone::default(),
                secondary_loading: false,
                activity_timer: None,
                usage: service.usage,
                memory_limit_bytes: service.memory_limit_bytes,
                template: instance.template.clone(),
                directory: directory.clone(),
                compose_file: compose_file.clone(),
                compose_source: compose_source.clone(),
                template_available,
                checkout_path: None,
                informational: false,
                cleanup_target: None,
                manifest_source: None,
                instance: None,
                guidance_source: None,
                service: (!service.one_shot).then(|| (instance.name.clone(), service.name.clone())),
                service_name: Some(service.name.clone()),
                description: String::new(),
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
        let progress =
            operation_progress(operations, &activity.action, &activity.name).or_else(|| {
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
        row.cleanup_target =
            (activity.action == "delete_instance" && activity.finished && activity.error.is_some())
                .then(|| activity.name.clone());
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
        if row.parent.is_none() && !row.hide_resources {
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
