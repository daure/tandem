use std::{
    collections::BTreeMap,
    fs::File,
    io::{self, Read},
    path::Path,
    time::Instant,
};

use super::Presence;
use crate::store::{
    environments::UsageSummary,
    opencode::resources::{ProcessResource, ProcessSample},
};

pub(super) fn collect(
    presences: &[Presence],
    previous: Vec<ProcessResource>,
) -> Vec<ProcessResource> {
    let previous: BTreeMap<_, _> = previous
        .into_iter()
        .map(|process| (process.pid, process))
        .collect();
    let mut latest = BTreeMap::<u32, &Presence>::new();
    for presence in presences {
        if latest
            .get(&presence.pid)
            .is_none_or(|current| presence.observed_at > current.observed_at)
        {
            latest.insert(presence.pid, presence);
        }
    }
    // These kernel constants apply to every PID; no process-table scan is needed.
    let ticks = unsafe { libc::sysconf(libc::_SC_CLK_TCK) };
    let page_size = unsafe { libc::sysconf(libc::_SC_PAGESIZE) };
    latest
        .into_values()
        .filter_map(|presence| {
            let mut resource = ProcessResource {
                pid: presence.pid,
                session_id: presence.id.clone(),
                directory: presence.directory.clone(),
                zellij_session: presence.zellij_session.clone(),
                pane_id: presence.pane_id,
                ..Default::default()
            };
            match read_process(
                Path::new("/proc"),
                presence.pid,
                page_size,
                ticks,
                previous.get(&presence.pid),
            ) {
                Ok(Some((sample, usage))) => {
                    resource.sample = Some(sample);
                    resource.usage = usage;
                }
                Ok(None) => return None,
                Err(error) if error.kind() == io::ErrorKind::NotFound => return None,
                Err(error) => {
                    if let Some(previous) = previous.get(&presence.pid) {
                        resource.sample = previous.sample.clone();
                        resource.usage = previous.usage.clone();
                    }
                    resource.mark_stale(format!("Client PID {}: {error}", presence.pid));
                }
            }
            Some(resource)
        })
        .collect()
}

fn read_process(
    root: &Path,
    pid: u32,
    page_size: i64,
    ticks: i64,
    previous: Option<&ProcessResource>,
) -> io::Result<Option<(ProcessSample, UsageSummary)>> {
    let mut text = String::new();
    File::open(root.join(pid.to_string()).join("stat"))?
        .take(16_385)
        .read_to_string(&mut text)?;
    if text.len() > 16_384 {
        return Err(invalid_stat());
    }
    sample(&text, pid, page_size, ticks, Instant::now(), previous)
}

fn invalid_stat() -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        "Invalid process resource sample",
    )
}

fn sample(
    text: &str,
    pid: u32,
    page_size: i64,
    ticks: i64,
    now: Instant,
    previous: Option<&ProcessResource>,
) -> io::Result<Option<(ProcessSample, UsageSummary)>> {
    let (identity, fields) = text.rsplit_once(')').ok_or_else(invalid_stat)?;
    if identity
        .split_whitespace()
        .next()
        .and_then(|pid| pid.parse::<u32>().ok())
        != Some(pid)
        || page_size <= 0
        || ticks <= 0
    {
        return Err(invalid_stat());
    }
    let fields: Vec<_> = fields.split_whitespace().collect();
    if matches!(fields.first().copied(), Some("Z" | "X" | "x")) {
        return Ok(None);
    }
    let number = |index: usize| {
        fields
            .get(index)
            .and_then(|value| value.parse::<u64>().ok())
            .ok_or_else(invalid_stat)
    };
    let cpu_ticks = number(11)?
        .checked_add(number(12)?)
        .ok_or_else(invalid_stat)?;
    let started_ticks = number(19)?;
    let memory_bytes = number(21)?
        .checked_mul(page_size as u64)
        .ok_or_else(invalid_stat)?;
    let cpu_basis_points = previous
        .and_then(|previous| previous.sample.as_ref())
        .filter(|previous| previous.started_ticks == started_ticks)
        .and_then(|previous| {
            let elapsed = now.checked_duration_since(previous.observed_at)?.as_nanos();
            let delta = cpu_ticks.checked_sub(previous.cpu_ticks)?;
            let divisor = elapsed.checked_mul(ticks as u128)?;
            let value = (u128::from(delta) * 10_000 * 1_000_000_000).checked_div(divisor)?;
            u64::try_from(value).ok()
        });
    Ok(Some((
        ProcessSample {
            started_ticks,
            cpu_ticks,
            observed_at: now,
        },
        UsageSummary {
            memory_bytes: Some(memory_bytes),
            cpu_basis_points,
            cpu_waiting: cpu_basis_points.is_none(),
            age_seconds: Some(0),
            ..Default::default()
        },
    )))
}

#[cfg(test)]
#[path = "tests/resources.rs"]
mod tests;
