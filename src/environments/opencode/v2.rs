use serde_json::{Value, json};

use super::transport;

pub(super) async fn read(
    client: &reqwest::Client,
    server: &str,
    path: &str,
) -> Result<Option<Value>, String> {
    let url = reqwest::Url::parse(&format!("http://localhost{path}"))
        .map_err(|error| error.to_string())?;
    let route = url.path();
    let directory = url
        .query_pairs()
        .find(|(key, _)| key == "directory")
        .map(|(_, value)| value.into_owned());
    let location = directory
        .as_ref()
        .map(|directory| {
            let mut url = reqwest::Url::parse("http://localhost").expect("static URL");
            url.query_pairs_mut()
                .append_pair("location[directory]", directory);
            format!("?{}", url.query().unwrap_or_default())
        })
        .unwrap_or_default();
    if route == "/global/health" {
        return transport::raw(client, server, "/api/info", reqwest::Method::GET).await;
    }
    if route == "/session/status" {
        let Some(mut data) =
            transport::raw(client, server, "/api/session/active", reqwest::Method::GET).await?
        else {
            return Err("OpenCode activity unavailable".into());
        };
        let data = data
            .get_mut("data")
            .ok_or("Invalid OpenCode activity envelope")?;
        for status in data
            .as_object_mut()
            .ok_or("Invalid OpenCode activity map")?
            .values_mut()
        {
            status["type"] = json!("busy");
        }
        return Ok(Some(data.clone()));
    }
    if matches!(route, "/question" | "/permission") {
        let route = if route == "/question" {
            "/api/form"
        } else {
            "/api/permission/request"
        };
        return envelope(client, server, &format!("{route}{location}")).await;
    }
    if route == "/experimental/session" {
        let limit = url
            .query_pairs()
            .find(|(key, _)| key == "limit")
            .and_then(|(_, value)| value.parse().ok())
            .unwrap_or(50);
        let roots = url
            .query_pairs()
            .any(|(key, value)| key == "roots" && value == "true");
        let mut query = vec![];
        if let Some(directory) = directory {
            query.push(("directory", directory));
        }
        if roots {
            query.push(("parentID", "null".into()));
        }
        let sessions = sessions(client, server, query, limit).await?;
        return Ok(Some(Value::Array(
            sessions.into_iter().map(session).collect(),
        )));
    }
    if let Some(id) = route.strip_prefix("/session/") {
        if let Some(id) = id.strip_suffix("/children") {
            let children = sessions(client, server, vec![("parentID", id.into())], 10_000).await?;
            return Ok(Some(Value::Array(
                children.into_iter().map(session).collect(),
            )));
        }
        if let Some(id) = id.strip_suffix("/message") {
            let Some(export) = envelope(
                client,
                server,
                &format!("/api/experimental/session/{id}/export"),
            )
            .await?
            else {
                return Ok(None);
            };
            let messages = export["messages"]
                .as_array()
                .ok_or("Invalid OpenCode export")?;
            return Ok(Some(Value::Array(
                messages.iter().filter_map(message).collect(),
            )));
        }
        return Ok(envelope(client, server, &format!("/api/session/{id}"))
            .await?
            .map(session));
    }
    Err("Unsupported OpenCode V2 operation".into())
}

pub(super) async fn envelope(
    client: &reqwest::Client,
    server: &str,
    path: &str,
) -> Result<Option<Value>, String> {
    transport::raw(client, server, path, reqwest::Method::GET)
        .await?
        .map(|mut response| {
            response
                .get_mut("data")
                .map(Value::take)
                .ok_or_else(|| "Invalid OpenCode response envelope".into())
        })
        .transpose()
}

async fn sessions(
    client: &reqwest::Client,
    server: &str,
    query: Vec<(&str, String)>,
    limit: usize,
) -> Result<Vec<Value>, String> {
    let mut found = Vec::new();
    let mut cursor: Option<String> = None;
    let mut seen = std::collections::BTreeSet::new();
    loop {
        let mut url = reqwest::Url::parse("http://localhost/api/session").expect("static URL");
        url.query_pairs_mut()
            .append_pair("limit", &(limit - found.len()).clamp(1, 200).to_string());
        if let Some(cursor) = &cursor {
            url.query_pairs_mut().append_pair("cursor", cursor);
        } else {
            for (key, value) in &query {
                url.query_pairs_mut().append_pair(key, value);
            }
        }
        let path = format!("{}?{}", url.path(), url.query().unwrap_or_default());
        let page = transport::raw(client, server, &path, reqwest::Method::GET)
            .await?
            .ok_or("OpenCode session inventory unavailable")?;
        found.extend(
            page["data"]
                .as_array()
                .ok_or("Invalid OpenCode session inventory")?
                .iter()
                .cloned(),
        );
        cursor = page["cursor"]["next"].as_str().map(str::to_owned);
        if cursor.is_none() || found.len() >= limit {
            return Ok(found);
        }
        if !seen.insert(cursor.clone()) {
            return Err("OpenCode repeated a pagination cursor".into());
        }
    }
}

fn session(mut value: Value) -> Value {
    value["directory"] = value["location"]["directory"].clone();
    if value["title"].is_null() {
        value["title"] = json!("Untitled");
    }
    value
}

fn message(value: &Value) -> Option<Value> {
    let kind = value["type"].as_str()?;
    if !matches!(kind, "user" | "assistant" | "compaction") {
        return None;
    }
    let mut info = json!({"id":value["id"], "role":if kind == "compaction" { "assistant" } else { kind }, "time":value["time"], "agent":value["agent"]});
    info["modelID"] = value["model"]["id"].clone();
    info["providerID"] = value["model"]["providerID"].clone();
    info["variant"] = value["model"]["variant"].clone();
    let mut parts = Vec::new();
    if kind == "assistant" {
        for mut content in value["content"].as_array().into_iter().flatten().cloned() {
            if content["type"] == "tool" {
                content["tool"] = content["name"].clone();
            }
            parts.push(content);
        }
    } else {
        parts.push(json!({"type":"text", "text":if kind == "compaction" { &value["summary"] } else { &value["text"] }}));
        parts.extend(
            value["files"]
                .as_array()
                .into_iter()
                .flatten()
                .map(|file| json!({"type":"file","filename":file["name"]})),
        );
    }
    Some(json!({"info":info,"parts":parts}))
}
