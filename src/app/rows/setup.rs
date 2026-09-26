use super::{Instance, Property, Row, Severity, Status, Tone, UsageSummary};

pub(super) fn append(rows: &mut Vec<Row>, instance: &Instance) {
    let parent = rows.last().expect("instance row").clone();
    let total = instance.repositories.len()
        + instance
            .services
            .iter()
            .filter(|service| service.one_shot)
            .count();
    let completed = instance.repositories.len()
        + instance
            .services
            .iter()
            .filter(|service| {
                service.one_shot && service.status_summary().status == Status::Completed
            })
            .count();
    if total > 0 {
        let mut group = child(&parent, format!("setup:{}", instance.name));
        if completed == total {
            group.label = format!("Setup · {completed} completed");
            group.icon = "";
            group.tone = Tone::Success;
        } else {
            group.label = format!("Setup · {completed}/{total} completed");
        }
        let mut repositories = instance.repositories.iter().collect::<Vec<_>>();
        repositories.sort_by(|a, b| a.target.cmp(&b.target));
        rows.push(group.clone());
        for repository in repositories {
            let mut row = child(
                &group,
                format!("repository:{}:{}", instance.name, repository.target),
            );
            let status = if repository.cloned {
                "Cloned"
            } else {
                "Existing checkout"
            };
            row.label = format!("{} · {status}", repository.target);
            row.status = Some(status.into());
            row.icon = "";
            row.tone = Tone::Success;
            row.checkout_path = Some(repository.path.clone());
            row.details = vec![
                Property::new("Repository", &repository.target),
                Property::new("Checkout directory", &repository.path),
                Property::new("Provisioning", status).tone(Tone::Success),
            ];
            rows.push(row);
        }
    }
}

pub(super) fn services(parent: &Row, instance: &Instance, count: usize) -> Row {
    let mut group = child(parent, format!("services:{}", instance.name));
    group.label = if count == 1 { "Service" } else { "Services" }.into();
    group.icon = "󰒋";
    let services = instance
        .services
        .iter()
        .filter(|service| !service.one_shot)
        .collect::<Vec<_>>();
    let summaries = services
        .iter()
        .map(|service| service.status_summary())
        .collect::<Vec<_>>();
    let running = services.iter().filter(|service| service.ready()).count();
    group.status_detail = Some(if count == 0 {
        "(no services configured)".into()
    } else {
        format!("{running}/{count} running")
    });
    group.detail_tone = Tone::Muted;
    group.tone = if summaries.is_empty() {
        Tone::Muted
    } else if summaries.iter().any(|summary| {
        summary.severity == Severity::Error || summary.detail_severity == Severity::Error
    }) {
        Tone::Error
    } else if services.iter().all(|service| service.ready()) {
        Tone::Success
    } else if summaries.iter().all(|summary| {
        matches!(
            summary.status,
            Status::NotStarted | Status::Waiting | Status::Stopped
        )
    }) {
        Tone::Muted
    } else {
        Tone::Warning
    };
    group
}

fn child(parent: &Row, id: String) -> Row {
    Row {
        id,
        parent: Some(parent.id.clone()),
        template: parent.template.clone(),
        directory: parent.directory.clone(),
        compose_file: parent.compose_file.clone(),
        compose_source: parent.compose_source.clone(),
        template_available: parent.template_available,
        metrics: UsageSummary::default(),
        hide_resources: true,
        ..Default::default()
    }
}
