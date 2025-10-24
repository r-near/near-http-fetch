use anyhow::Result;

#[tokio::main]
async fn main() -> Result<()> {
    let _ = dotenvy::dotenv();
    let config = relayer::Config::from_env()?;
    relayer::run(config).await
}
