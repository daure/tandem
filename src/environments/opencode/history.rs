use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
    time::{Duration, Instant},
};

use serde::Deserialize;

use super::{Observer, RemoteQuestion, RemoteStatus, transport, valid_id};

mod server;

const SESSION_LIMIT: usize = 10_000;

#[derive(Deserialize)]
struct SavedSession {
    id: String,
    directory: String,
    #[serde(rename = "parentID")]
    parent_id: Option<String>,
}

pub(crate) async fn clear(directory: &str, deadline: Instant) -> Result<(), String> {
    let timeout = deadline
        .saturating_duration_since(Instant::now())
        .min(Duration::from_secs(60));
    tokio::time::timeout(timeout, clear_with(&Observer::from_env(), directory))
        .await
        .unwrap_or_else(|_| Err("OpenCode history cleanup timed out".into()))
        .map_err(|error| format!("Cannot clear OpenCode history: {error}. Close active clients and retry, or disable Clear OpenCode history on new instance in Settings"))
}

pub(super) async fn clear_with(observer: &Observer, directory: &str) -> Result<(), String> {
    if !Path::new(directory).is_absolute() {
        return Err("workspace path must be absolute".into());
    }
    ensure_detached(observer, directory)?;
    let (_, known) = observer.inventory();
    let mut servers = known
        .into_iter()
        .filter_map(|(server, directories)| directories.contains(directory).then_some(server))
        .collect::<Vec<_>>();
    let mut temporary = if servers.is_empty() {
        let (child, url) = server::start(directory).await?;
        servers.push(url);
        Some(child)
    } else {
        None
    };
    let result = clear_servers(observer, directory, &servers).await;
    if let Some(child) = &mut temporary {
        child
            .kill()
            .await
            .map_err(|error| format!("cannot stop cleanup server: {error}"))?;
    }
    result
}

fn target(path: &str, directory: &str) -> Result<String, String> {
    let mut url = reqwest::Url::parse(&format!("http://localhost{path}"))
        .map_err(|error| error.to_string())?;
    url.query_pairs_mut().append_pair("directory", directory);
    Ok(format!(
        "{}?{}",
        url.path(),
        url.query().unwrap_or_default()
    ))
}

async fn sessions(
    client: &reqwest::Client,
    server: &str,
    directory: &str,
) -> Result<Vec<SavedSession>, String> {
    let path = target("/experimental/session", directory)?;
    let found: Vec<SavedSession> = transport::get(
        client,
        server,
        &format!("{path}&archived=true&limit={SESSION_LIMIT}"),
    )
    .await?;
    // Refuse a potentially truncated inventory; timestamp cursors can skip tied sessions.
    if found.len() >= SESSION_LIMIT {
        return Err("workspace history exceeds the safe cleanup limit".into());
    }
    if found
        .iter()
        .any(|session| session.directory != directory || !valid_id(&session.id))
    {
        return Err("OpenCode returned sessions outside the exact workspace directory".into());
    }
    Ok(found)
}

fn deletion_order(
    mut pending: BTreeMap<String, SavedSession>,
) -> Result<Vec<SavedSession>, String> {
    let mut children: BTreeMap<_, usize> = pending.keys().map(|id| (id.clone(), 0)).collect();
    for session in pending.values() {
        if let Some(parent) = session
            .parent_id
            .as_ref()
            .and_then(|id| children.get_mut(id))
        {
            *parent += 1;
        }
    }
    let mut leaves: BTreeSet<_> = children
        .iter()
        .filter(|(_, count)| **count == 0)
        .map(|(id, _)| id.clone())
        .collect();
    let mut ordered = Vec::new();
    while let Some(id) = leaves.pop_first() {
        let session = pending.remove(&id).ok_or("session inventory changed")?;
        if let Some(parent) = &session.parent_id
            && let Some(count) = children.get_mut(parent)
        {
            *count -= 1;
            if *count == 0 {
                leaves.insert(parent.clone());
            }
        }
        ordered.push(session);
    }
    if !pending.is_empty() {
        return Err("OpenCode session ancestry contains a cycle".into());
    }
    Ok(ordered)
}

async fn clear_servers(
    observer: &Observer,
    directory: &str,
    servers: &[String],
) -> Result<(), String> {
    let client = transport::client()?;
    let mut plans = Vec::new();
    for server in servers {
        let found = sessions(&client, server, directory).await?;
        let pending: BTreeMap<_, _> = found
            .into_iter()
            .map(|session| (session.id.clone(), session))
            .collect();
        for session in pending.values() {
            let children: Vec<SavedSession> = transport::get(
                &client,
                server,
                &target(&format!("/session/{}/children", session.id), directory)?,
            )
            .await?;
            if children
                .iter()
                .any(|child| child.directory != directory || !pending.contains_key(&child.id))
            {
                return Err("session deletion would include unverified child conversations".into());
            }
        }
        let ordered = deletion_order(pending)?;
        ensure_idle(&client, server, directory, &ordered).await?;
        plans.push((server, ordered));
    }
    ensure_detached(observer, directory)?;
    for (server, ordered) in &plans {
        ensure_idle(&client, server, directory, ordered).await?;
    }
    for (server, ordered) in plans {
        for session in &ordered {
            ensure_detached(observer, directory)?;
            ensure_idle(&client, server, directory, &ordered).await?;
            let path = target(&format!("/session/{}", session.id), directory)?;
            let Some(current) =
                transport::get_optional::<SavedSession>(&client, server, &path).await?
            else {
                continue;
            };
            if current.id != session.id || current.directory != directory {
                return Err("session ownership changed during cleanup".into());
            }
            // A parent delete cascades. Recheck children after deleting leaves to protect
            // conversations created while the cleanup plan was being inspected.
            let children: Vec<SavedSession> = transport::get(
                &client,
                server,
                &target(&format!("/session/{}/children", session.id), directory)?,
            )
            .await?;
            if !children.is_empty() {
                return Err("session children changed during cleanup; retry".into());
            }
            transport::delete(&client, server, &path).await?;
            if transport::get_optional::<SavedSession>(&client, server, &path)
                .await?
                .is_some()
            {
                return Err("OpenCode did not delete the session".into());
            }
        }
        if !sessions(&client, server, directory).await?.is_empty() {
            return Err("workspace history changed during cleanup; retry".into());
        }
    }
    Ok(())
}

fn ensure_detached(observer: &Observer, directory: &str) -> Result<(), String> {
    if observer
        .presences()
        .iter()
        .any(|presence| presence.directory == directory)
    {
        return Err("an OpenCode client is attached to the workspace".into());
    }
    Ok(())
}

async fn ensure_idle(
    client: &reqwest::Client,
    server: &str,
    directory: &str,
    sessions: &[SavedSession],
) -> Result<(), String> {
    let statuses: BTreeMap<String, RemoteStatus> =
        transport::get(client, server, &target("/session/status", directory)?).await?;
    let questions: Vec<RemoteQuestion> =
        transport::get(client, server, &target("/question", directory)?).await?;
    if sessions.iter().any(|session| {
        statuses
            .get(&session.id)
            .is_some_and(|status| status.kind != "idle")
            || questions
                .iter()
                .any(|question| question.session_id == session.id)
    }) {
        return Err("workspace conversations are active or awaiting an answer".into());
    }
    Ok(())
}
