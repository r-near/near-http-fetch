use anyhow::Result;
use near_workspaces::network::NetworkInfo;
use relayer::{process_once, Config as RelayerConfig};
use serde_json::json;
use tokio::time::{sleep, Duration};

#[tokio::test]
async fn weather_contract_flow() -> Result<()> {
    let fetcher_wasm = near_workspaces::compile_project("./").await?;
    let weather_wasm = near_workspaces::compile_project("./examples/weather").await?;

    let worker = near_workspaces::sandbox().await?;

    let relayer = worker.dev_create_account().await?;
    let rpc_url = worker.info().rpc_url.to_string();

    let fetcher = worker.dev_deploy(&fetcher_wasm).await?;
    fetcher
        .call("new")
        .args_json(json!({ "trusted_relayer": relayer.id() }))
        .transact()
        .await?
        .into_result()?;

    let weather = worker.dev_deploy(&weather_wasm).await?;
    weather
        .call("new")
        .args_json(json!({ "fetcher_account": fetcher.id() }))
        .transact()
        .await?
        .into_result()?;

    let city = "Barcelona".to_string();

    let request_future = weather
        .call("request_weather")
        .args_json(json!({ "city": city }))
        .max_gas()
        .transact_async()
        .await?;

    let relayer_config = RelayerConfig::from_parts(
        &rpc_url,
        fetcher.id().as_str(),
        relayer.id().as_str(),
        &relayer.secret_key().to_string(),
        Some(1),
    )?;
    let http_client = relayer_config.http_client()?;

    let mut processed = false;
    for _ in 0..10 {
        if process_once(&relayer_config, &http_client).await? {
            processed = true;
            break;
        }
        sleep(Duration::from_millis(200)).await;
    }
    assert!(processed, "relayer did not process pending request");

    let request_result: bool = request_future.await?.json()?;
    assert!(request_result, "weather contract reported failure");

    let cached: Option<String> = weather
        .view("get_cached_weather")
        .args_json(json!({ "city": "Barcelona" }))
        .await?
        .json()?;

    let cached = cached.expect("weather cache missing");
    assert!(cached.contains("Barcelona"));
    assert!(cached.contains("°C"));

    Ok(())
}
