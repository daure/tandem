use crate::store::environments::{
    Instance, InstanceService, ResourceUsage, Template, UsageSummary,
};

use super::{properties::Property, rows::Tone};

pub(super) fn memory_tone(usage: Option<ResourceUsage>, limit: Option<u64>) -> Tone {
    match (usage, limit.filter(|limit| *limit > 0)) {
        (None, _) => Tone::Muted,
        (Some(usage), Some(limit)) => {
            let percentage = u128::from(usage.memory_bytes) * 100 / u128::from(limit);
            match percentage {
                90.. => Tone::Error,
                70..=89 => Tone::Warning,
                _ => Tone::Normal,
            }
        }
        _ => Tone::Normal,
    }
}

fn rounded_units(value: u64, divisor: u64) -> String {
    if value > 0 && value < divisor {
        "<1".into()
    } else {
        (value / divisor + u64::from(value % divisor >= divisor / 2)).to_string()
    }
}

pub(super) fn cpu(basis_points: u64) -> String {
    format!("{}%", rounded_units(basis_points, 100))
}

pub(super) fn memory(bytes: u64) -> String {
    format!("{} MiB", rounded_units(bytes, 1048576))
}

pub(super) fn template(template: &Template, count: usize) -> Vec<Property> {
    let mut rows = vec![
        Property::new("Template", &template.name),
        Property::new("Description", &template.manifest.description),
        Property::new("Instances", count),
        Property::new("Directory", &template.directory),
        Property::new("Compose file", &template.compose_file),
        Property::new("Routing manifest", &template.manifest_file),
        Property::new(
            "Status",
            if template.error.is_some() {
                "Invalid"
            } else {
                "Valid"
            },
        )
        .tone(if template.error.is_some() {
            Tone::Error
        } else {
            Tone::Success
        }),
    ];
    if let Some(error) = &template.error {
        rows.push(Property::new("Error", error).tone(Tone::Error));
    }
    rows
}

pub(super) fn instance(instance: &Instance) -> Vec<Property> {
    let summary = instance.status_summary();
    let mut rows = vec![
        Property::new("Instance", &instance.name),
        Property::new("Type", " Instance"),
        Property::new("Template", &instance.template),
        Property::new("Project", &instance.project),
        Property::new("Workspace", &instance.workspace),
        Property::new("Status", &summary.label).tone(summary.severity.into()),
        Property::new(
            "Running / expected",
            format!("{} / {}", summary.running, summary.expected),
        ),
        Property::new(
            "Topology",
            if instance.runtime.topology_known {
                "Recorded at launch"
            } else {
                "Unrecorded; start instance to capture configuration"
            },
        ),
        Property::new(
            "Open workspace",
            "Enter opens the workspace in your file explorer.",
        ),
    ];
    if let Some(detail) = summary.detail {
        rows.push(Property::new("Status detail", detail).tone(summary.detail_severity.into()));
    }
    if let Some(observed) = instance.runtime.observed_at {
        rows.push(Property::new("Observed (Unix)", observed));
    }
    if let Some(error) = instance.startup_error() {
        rows.push(Property::new("Startup error", error).tone(Tone::Error));
    }
    resources(
        &mut rows,
        ResourceUsage::total(instance.services.iter()),
        InstanceService::total_memory_limit(instance.services.iter()),
    );
    usage_details(&mut rows, &UsageSummary::instance(instance));
    for service in &instance.services {
        rows.extend(self::service(service).into_iter().map(|mut row| {
            row.name = format!("{} / {}", service.name, row.name);
            row
        }));
    }
    rows
}

pub(super) fn service(service: &InstanceService) -> Vec<Property> {
    let summary = service.status_summary();
    let mut rows = vec![
        Property::new("Service", &service.name),
        Property::new(
            "Type",
            if service.one_shot {
                " Setup job"
            } else {
                " Service"
            },
        ),
        Property::new("Status", &summary.label).tone(summary.severity.into()),
        Property::new("Docker status", &service.status),
        Property::new("Container", &service.container_id),
    ];
    if let Some(detail) = summary.detail {
        rows.push(Property::new("Status detail", detail).tone(summary.detail_severity.into()));
    }
    if let Some(code) = service.exit_code().filter(|_| {
        matches!(
            service.state(),
            crate::store::environments::ContainerState::Exited
                | crate::store::environments::ContainerState::Dead
        )
    }) {
        rows.push(Property::new("Exit code", code));
    }
    if service.runtime.oom_killed {
        rows.push(Property::new("OOM killed", "Yes").tone(Tone::Error));
    }
    if let Some(at) = service.runtime.readiness_checked_at {
        rows.push(Property::new("Last route readiness passed (Unix)", at));
    }
    if let Some(error) = &service.runtime.resource_error {
        rows.push(Property::new("Resource error", error).tone(Tone::Warning));
    }
    for (label, value) in [
        ("Image", service.image.as_deref()),
        ("Health", service.health.as_deref()),
        ("Restart policy", service.restart_policy.as_deref()),
        ("Created", service.created_at.as_deref()),
        ("Started", service.started_at.as_deref()),
        ("Gateway URL", service.url.as_deref()),
    ] {
        if let Some(value) = value {
            rows.push(Property::new(label, value));
        }
    }
    if let Some(port) = service.port {
        rows.push(Property::new("Port", port));
    }
    rows.push(Property::new("Restarts", service.restart_count));
    resources(&mut rows, service.usage, service.memory_limit_bytes);
    usage_details(&mut rows, &UsageSummary::service(service));
    rows
}

pub(super) fn resources(
    rows: &mut Vec<Property>,
    usage: Option<ResourceUsage>,
    limit: Option<u64>,
) {
    rows.push(Property::new(
        "CPU",
        usage
            .and_then(|usage| usage.cpu_basis_points)
            .map_or_else(|| "—".into(), cpu),
    ));
    rows.push(
        Property::new(
            "Memory",
            usage.map_or_else(|| "—".into(), |usage| memory(usage.memory_bytes)),
        )
        .tone(memory_tone(usage, limit)),
    );
    rows.push(Property::new(
        "Memory limit",
        limit.map_or_else(|| "No explicit cap / uncapped services".into(), memory),
    ));
    if let Some(usage) = usage {
        rows.push(Property::new(
            "Resource sample (Unix)",
            usage.sampled_at_unix_seconds,
        ));
    }
    rows.push(Property::new(
        "Resource refresh",
        "60s when focused; about 5min unfocused; manual refresh forces sampling",
    ));
}

pub(super) fn usage_details(rows: &mut Vec<Property>, usage: &UsageSummary) {
    for row in rows.iter_mut() {
        if row.name == "Memory" {
            row.value = usage.memory_bytes.map_or_else(|| "—".into(), memory);
            row.tone = memory_tone(
                usage.memory_bytes.map(|memory_bytes| ResourceUsage {
                    memory_bytes,
                    ..Default::default()
                }),
                usage.memory_limit_bytes,
            );
            if usage.memory_partial {
                row.value.push_str(" · partial");
            }
            if usage.memory_stale {
                row.value.push_str(" · stale");
                row.tone = Tone::Muted;
            }
        } else if row.name == "CPU" {
            row.value = usage.cpu_basis_points.map_or_else(|| "—".into(), cpu);
            row.tone = if usage.cpu_basis_points.is_some() {
                Tone::Normal
            } else {
                Tone::Muted
            };
            if usage.paused {
                row.value.push_str(" · paused");
            }
            if usage.cpu_partial {
                row.value.push_str(" · partial");
            }
            if usage.cpu_stale {
                row.value.push_str(" · stale");
                row.tone = Tone::Muted;
            }
        }
    }
    if let Some(age) = usage.age_seconds {
        rows.push(Property::new("Resource sample age", format!("{age}s")));
    }
}
