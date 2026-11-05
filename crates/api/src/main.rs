//! HTTP edge: health, version and, later, job submission and SSE.
//!
//! SDD section 10.2. The edge does only I/O; it never runs DSP. Routes that
//! need the compute pipeline are added with the job API task.

use axum::{Router, routing::get};
use tracing_subscriber::EnvFilter;

/// Default listen address. Deployment overrides this later.
const BIND_ADDR: &str = "0.0.0.0:8080";

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    init_tracing();

    let app = Router::new()
        .route("/healthz", get(health))
        .route("/v1/health", get(health))
        .route("/v1/version", get(version));

    let address = bind_address();
    let listener = tokio::net::TcpListener::bind(&address).await?;
    tracing::info!(address = %address, "api listening");

    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await?;
    Ok(())
}

fn bind_address() -> String {
    std::env::var("DOPPLER_BIND").unwrap_or_else(|_| BIND_ADDR.to_owned())
}

fn init_tracing() {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    if let Err(error) = tracing_subscriber::fmt().with_env_filter(filter).try_init() {
        eprintln!("failed to install tracing subscriber: {error}");
    }
}

// Axum's Handler trait requires an async function; these bodies have no await.
#[allow(clippy::unused_async)]
async fn health() -> &'static str {
    "ok"
}

#[allow(clippy::unused_async)]
async fn version() -> String {
    format!("{} {}", env!("CARGO_PKG_NAME"), env!("CARGO_PKG_VERSION"))
}

async fn shutdown_signal() {
    if let Err(error) = tokio::signal::ctrl_c().await {
        tracing::error!(%error, "failed to listen for shutdown signal");
    }
}
