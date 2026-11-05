//! Queue worker binary.
//!
//! SDD section 11.6. The real claim loop, lease heartbeat and job runtime are
//! added with the job queue task. This entry point already uses the shared
//! `Scope` so shutdown and task ownership are exercised from the start.

use std::time::Duration;

use runtime::Scope;
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    init_tracing();

    let mut scope = Scope::new();
    let heartbeat_token = scope.token();
    scope.spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(30));
        loop {
            tokio::select! {
                () = heartbeat_token.cancelled() => break,
                _ = interval.tick() => tracing::debug!("worker heartbeat"),
            }
        }
    });

    tracing::info!("worker started; press Ctrl+C to stop");
    if let Err(error) = tokio::signal::ctrl_c().await {
        tracing::error!(%error, "failed to listen for shutdown signal");
    }

    scope.close().await;
    tracing::info!("worker stopped");
    Ok(())
}

fn init_tracing() {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    if let Err(error) = tracing_subscriber::fmt().with_env_filter(filter).try_init() {
        eprintln!("failed to install tracing subscriber: {error}");
    }
}
