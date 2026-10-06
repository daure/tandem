use std::{net::SocketAddr, sync::mpsc::Sender};

use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, Path, Request, State},
    http::{HeaderMap, StatusCode, header},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use serde_json::json;

use crate::{
    service::AppService,
    store::events::{Batch, Error},
};

fn token(headers: &HeaderMap) -> Result<String, Error> {
    if headers.contains_key(header::ORIGIN)
        || headers.get_all(header::AUTHORIZATION).iter().count() != 1
    {
        return Err(Error::Unauthorized);
    }
    headers
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
        .map(str::to_owned)
        .ok_or(Error::Unauthorized)
}

impl IntoResponse for Error {
    fn into_response(self) -> Response {
        let (status, message) = match self {
            Self::Unauthorized => (
                StatusCode::UNAUTHORIZED,
                "invalid provider credential".into(),
            ),
            Self::Invalid(message) => (StatusCode::UNPROCESSABLE_ENTITY, message),
            Self::Conflict(message) => (StatusCode::CONFLICT, message),
            Self::NotFound => (
                StatusCode::NOT_FOUND,
                "event or notification not found".into(),
            ),
            Self::Storage(message) => {
                crate::diagnostics::record_error(
                    "event service failed",
                    &std::io::Error::other(message),
                );
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "event storage unavailable".into(),
                )
            }
        };
        (status, Json(json!({"error": message}))).into_response()
    }
}

pub(crate) fn router(service: AppService) -> Router {
    Router::new()
        .route("/v1/events", post(ingest))
        .route("/v1/notifications", get(notifications))
        .route("/v1/notifications/{id}/ack", post(acknowledge))
        .route("/v1/streams", get(streams))
        .route("/v1/streams/ack", post(acknowledge_stream))
        .layer(DefaultBodyLimit::max(1_048_576))
        .route_layer(middleware::from_fn_with_state(
            service.clone(),
            authenticate,
        ))
        .with_state(service)
}

pub(crate) async fn run_owned(
    service: AppService,
    bind: SocketAddr,
    lease: crate::environments::provider_sidecar::Lease,
) -> Result<(), Box<dyn std::error::Error>> {
    if !bind.ip().is_loopback() {
        return Err("owned provider sidecar must be loopback".into());
    }
    let listener = tokio::net::TcpListener::bind(bind).await?;
    let origin = format!("http://{}", listener.local_addr()?);
    lease.publish(origin.clone())?;
    let identity = crate::environments::provider_sidecar::Receipt {
        pid: std::process::id(),
        origin,
        identity: lease.identity.clone(),
    };
    let worker = service.clone();
    let automation = tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_millis(500));
        loop {
            interval.tick().await;
            worker.poll_rules();
        }
    });
    let app = router(service).route("/v1/identity", get(move || async move { Json(identity) }));
    axum::serve(listener, app)
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .await?;
    drop(lease);
    automation.abort();
    Ok(())
}

async fn authenticate(State(service): State<AppService>, request: Request, next: Next) -> Response {
    let result = match token(request.headers()) {
        Ok(token) => service.authenticate_provider(token).await,
        Err(error) => Err(error),
    };
    match result {
        Ok(()) => next.run(request).await,
        Err(error) => error.into_response(),
    }
}

async fn ingest(
    State(service): State<AppService>,
    headers: HeaderMap,
    Json(batch): Json<Batch>,
) -> Result<impl IntoResponse, Error> {
    let ingestion = service.ingest_events(token(&headers)?, batch).await?;
    Ok((StatusCode::OK, Json(ingestion)))
}

async fn notifications(
    State(service): State<AppService>,
    headers: HeaderMap,
) -> Result<impl IntoResponse, Error> {
    let notifications = service.provider_notifications(token(&headers)?).await?;
    Ok(Json(json!({"notifications": notifications})))
}

async fn acknowledge(
    State(service): State<AppService>,
    headers: HeaderMap,
    Path(id): Path<i64>,
) -> Result<impl IntoResponse, Error> {
    service
        .acknowledge_provider_notification(token(&headers)?, id)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn streams(
    State(service): State<AppService>,
    headers: HeaderMap,
) -> Result<impl IntoResponse, Error> {
    let streams = service.provider_stream_controls(token(&headers)?).await?;
    Ok(Json(json!({"streams": streams})))
}

async fn acknowledge_stream(
    State(service): State<AppService>,
    headers: HeaderMap,
    Json(control): Json<crate::store::providers::StreamControl>,
) -> Result<impl IntoResponse, Error> {
    let stream = control.stream.clone();
    service
        .acknowledge_provider_stream(token(&headers)?, control)
        .await
        .map_err(|error| match error {
            Error::Storage(message) => {
                Error::Storage(format!("POST /v1/streams/ack stream={stream:?}: {message}"))
            }
            error => error,
        })?;
    Ok(StatusCode::NO_CONTENT)
}

pub(crate) async fn run(
    service: AppService,
    bind: SocketAddr,
    startup: Option<Sender<Result<(), String>>>,
) -> Result<(), Box<dyn std::error::Error>> {
    let listener = match tokio::net::TcpListener::bind(bind).await {
        Ok(listener) => listener,
        Err(error) => {
            if let Some(startup) = startup {
                let _ = startup.send(Err(error.to_string()));
            }
            return Err(Box::new(error));
        }
    };
    if let Some(startup) = startup {
        let _ = startup.send(Ok(()));
    }
    axum::serve(listener, router(service))
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .await?;
    Ok(())
}

#[cfg(test)]
#[path = "events_http/tests/mod.rs"]
mod tests;
