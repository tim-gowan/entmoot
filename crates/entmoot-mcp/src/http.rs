//! Explicit authenticated loopback transport; shares the stdio query service.
use anyhow::{bail, Context, Result};
use axum::{
    extract::{Request, State},
    http::StatusCode,
    middleware::{self, Next},
    response::Response,
    Router,
};
use rmcp::transport::streamable_http_server::{
    session::local::LocalSessionManager, StreamableHttpServerConfig, StreamableHttpService,
};
use std::{net::SocketAddr, path::Path, sync::Arc};

async fn authenticate(
    State(token): State<Arc<String>>,
    request: Request,
    next: Next,
) -> Result<Response, StatusCode> {
    if request.headers().contains_key("origin") {
        return Err(StatusCode::FORBIDDEN);
    }
    let supplied = request
        .headers()
        .get("authorization")
        .and_then(|h| h.to_str().ok())
        .and_then(|h| h.strip_prefix("Bearer "));
    if supplied != Some(token.as_str()) {
        return Err(StatusCode::UNAUTHORIZED);
    }
    Ok(next.run(request).await)
}

pub(super) async fn serve(
    service: super::EntmootMcp,
    address: SocketAddr,
    token_file: Option<&Path>,
) -> Result<()> {
    if !address.ip().is_loopback() {
        bail!("query backend must bind a loopback address");
    }
    let token = match token_file {
        Some(path) => std::fs::read_to_string(path).context("read backend token file")?,
        None => std::env::var("ENTMOOT_QUERY_TOKEN")
            .context("set ENTMOOT_QUERY_TOKEN or --token-file before starting the backend")?,
    };
    let token = token.trim().to_owned();
    if token.len() < 32 {
        bail!("backend bearer token must contain at least 32 characters");
    }
    let mut config = StreamableHttpServerConfig::default();
    config.legacy_session_mode = false;
    config.json_response = true;
    config.max_request_body_bytes = 1024 * 1024;
    let cancellation = config.cancellation_token.clone();
    let transport = StreamableHttpService::new(
        move || Ok(service.clone()),
        Arc::new(LocalSessionManager::default()),
        config,
    );
    let router =
        Router::new()
            .nest_service("/mcp", transport)
            .layer(middleware::from_fn_with_state(
                Arc::new(token),
                authenticate,
            ));
    let listener = tokio::net::TcpListener::bind(address)
        .await
        .context("bind loopback query backend")?;
    eprintln!(
        "Entmoot query backend: http://{}/mcp",
        listener.local_addr()?
    );
    axum::serve(listener, router)
        .with_graceful_shutdown(async move {
            let _ = tokio::signal::ctrl_c().await;
            cancellation.cancel();
        })
        .await
        .context("serve query backend")
}
