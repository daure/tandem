use std::{sync::Arc, time::Duration};

use serde_json::{Value, json};
use tokio::{
    io::{AsyncBufReadExt, AsyncReadExt, BufReader},
    net::UnixStream,
    sync::{OwnedSemaphorePermit, watch},
};

use crate::{environments::state_stream::Store, store::state_stream};

use super::{encode, write_frame};

pub(super) async fn client(
    stream: UnixStream,
    store: Arc<Store>,
    mut updates: watch::Receiver<String>,
    permit: Arc<OwnedSemaphorePermit>,
) -> Result<(), String> {
    let (read, mut write) = stream.into_split();
    let mut read = BufReader::new(read).take(4097);
    let mut request = String::new();
    let since = match tokio::time::timeout(Duration::from_millis(100), read.read_line(&mut request))
        .await
    {
        Ok(Ok(_)) if request.len() <= 4096 => {
            if request.is_empty() {
                None
            } else {
                let value: Value =
                    serde_json::from_str(&request).map_err(|error| error.to_string())?;
                value["since"].as_str().map(str::to_owned)
            }
        }
        Err(_) if request.is_empty() => None,
        _ => return Err("invalid state subscription".into()),
    };
    let mut read = read.into_inner();
    let mut extra = [0u8; 1];
    tokio::select! {
        result = async {
            let adapter = store.clone();
            let requested = since.clone();
            let lease = permit.clone();
            let boundary = tokio::task::spawn_blocking(move || {
                let _lease = lease;
                adapter.boundary(requested.as_deref())
            })
                .await.map_err(|error| error.to_string())??;
            if boundary.reset {
                let mut snapshot = state_stream::envelope(
                    "snapshot", &boundary.cursor,
                    serde_json::to_value(boundary.snapshot).map_err(|error| error.to_string())?,
                );
                if since.is_some() { snapshot["reset"] = json!(true); }
                write_frame(&mut write, &encode(&snapshot)?).await?;
            } else {
                for event in boundary.events {
                    write_frame(&mut write, &encode(&event)?).await?;
                }
                write_frame(&mut write, &encode(&state_stream::envelope(
                    "replay.complete", &boundary.cursor, Value::Null,
                ))?).await?;
            }
            let mut cursor = boundary.cursor;
            loop {
                updates.borrow_and_update();
                loop {
                    let adapter = store.clone();
                    let after = cursor.clone();
                    let lease = permit.clone();
                    let page = tokio::task::spawn_blocking(move || {
                        let _lease = lease;
                        adapter.live_page(&after)
                    })
                        .await.map_err(|error| error.to_string())??;
                    if page.events.is_empty() { break; }
                    for event in page.events {
                        write_frame(&mut write, &encode(&event)?).await?;
                    }
                    cursor = page.cursor;
                }
                updates.changed().await.map_err(|error| error.to_string())?;
            }
        } => result,
        _ = read.read(&mut extra) => Ok(()),
    }
}
