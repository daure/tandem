use super::{Instance, Property, Row, Status, Tone, UsageSummary};

pub(super) fn append(rows: &mut Vec<Row>, instance: &Instance) {
    let parent = rows.last().expect("instance row").clone();
    let completed = instance.repositories.len()
        + instance
            .services
            .iter()
            .filter(|service| {
                service.one_shot && service.status_summary().status == Status::Completed
            })
            .count();
    if completed > 0 {
        let mut group = child(&parent, format!("setup:{}", instance.name));
        group.label = format!("Setup · {completed} completed");
        group.icon = "";
        group.tone = Tone::Success;
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
    if instance.workspace_only {
        let mut row = child(&parent, format!("no-services:{}", instance.name));
        row.label = "(no services configured)".into();
        row.tone = Tone::Muted;
        row.informational = true;
        rows.push(row);
    }
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
