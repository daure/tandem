use axum::{
    Router,
    extract::Request,
    http::{StatusCode, header, uri::Authority},
    middleware::{self, Next},
    response::IntoResponse,
};
use rmcp::transport::{StreamableHttpServerConfig, StreamableHttpService};

use super::McpServer;
use crate::service::AppService;

pub(super) fn router(service: AppService, port: u16) -> Router {
    let mcp: StreamableHttpService<McpServer> = StreamableHttpService::new(
        move || Ok(McpServer::new(service.clone())),
        Default::default(),
        StreamableHttpServerConfig {
            stateful_mode: false,
            sse_keep_alive: None,
            ..Default::default()
        },
    );
    Router::new()
        .nest_service("/mcp", mcp)
        .layer(middleware::from_fn(
            move |request: Request, next: Next| async move {
                if !is_local_request(&request, port) {
                    return (
                        StatusCode::FORBIDDEN,
                        "MCP HTTP requires a loopback host and same-origin requests",
                    )
                        .into_response();
                }
                next.run(request).await
            },
        ))
}

fn is_local_request(request: &Request, port: u16) -> bool {
    let headers = request.headers();
    if headers.get_all(header::HOST).iter().count() > 1
        || headers.get_all(header::ORIGIN).iter().count() > 1
    {
        return false;
    }
    let authority = match headers.get(header::HOST) {
        Some(host) => host
            .to_str()
            .ok()
            .and_then(|host| host.parse::<Authority>().ok()),
        None => request.uri().authority().cloned(),
    };
    let Some(authority) = authority else {
        return false;
    };
    let host = authority.host().trim_matches(['[', ']']);
    if authority.as_str().contains('@')
        || authority.port_u16().unwrap_or(80) != port
        || !(host.eq_ignore_ascii_case("localhost")
            || host
                .parse::<std::net::IpAddr>()
                .is_ok_and(|address| address.is_loopback()))
    {
        return false;
    }
    let Some(origin) = headers.get(header::ORIGIN) else {
        return true;
    };
    let Some(origin) = origin
        .to_str()
        .ok()
        .and_then(|origin| reqwest::Url::parse(origin).ok())
    else {
        return false;
    };
    origin.scheme() == "http"
        && origin
            .host_str()
            .is_some_and(|value| value.trim_matches(['[', ']']).eq_ignore_ascii_case(host))
        && origin.port_or_known_default() == Some(port)
        && origin.username().is_empty()
        && origin.password().is_none()
        && origin.path() == "/"
        && origin.query().is_none()
        && origin.fragment().is_none()
}
