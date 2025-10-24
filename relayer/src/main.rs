use anyhow::Result;
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> Result<()> {
    let _ = dotenvy::dotenv();

    // Initialize tracing with environment-based log level filtering
    // Default to INFO level, can be overridden with RUST_LOG env var
    // Example: RUST_LOG=debug or RUST_LOG=relayer=trace
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| EnvFilter::new("info"))
        )
        .with_target(true)
        .with_thread_ids(false)
        .with_line_number(true)
        .init();

    let config = relayer::Config::from_env()?;
    relayer::run(config).await
}
