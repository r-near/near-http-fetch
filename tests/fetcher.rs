use anyhow::Result;
use serde::Deserialize;
use serde_json::json;

#[derive(Clone, Deserialize)]
struct PendingRequestView {
    request_id: u64,
    url: String,
    #[allow(dead_code)]
    caller: String,
    #[serde(default)]
    #[allow(dead_code)]
    context: Option<Vec<u8>>,
    yield_id: Vec<u8>,
}

#[derive(Deserialize)]
struct FetchResultView {
    request_id: u64,
    url: String,
    status: FetchStatusView,
    #[serde(default)]
    body: Option<Vec<u8>>,
    #[serde(default)]
    #[allow(dead_code)]
    context: Option<Vec<u8>>,
    caller: String,
}

#[derive(Deserialize)]
enum FetchStatusView {
    Completed,
    TimedOut,
}

#[tokio::test]
async fn fetcher_yield_resume_flow() -> Result<()> {
    let fetcher_wasm = near_workspaces::compile_project("./").await?;
    let worker = near_workspaces::sandbox().await?;

    let relayer = worker.dev_create_account().await?;
    let fetcher = worker.dev_deploy(&fetcher_wasm).await?;

    fetcher
        .call("new")
        .args_json(json!({ "trusted_relayer": relayer.id() }))
        .transact()
        .await?
        .into_result()?;

    let fetch_tx = fetcher
        .call("fetch")
        .args_json(json!({
            "url": "https://example.com/data",
            "context": null
        }))
        .max_gas()
        .transact_async()
        .await?;

    let pending = loop {
        let requests: Vec<PendingRequestView> = fetcher
            .view("list_requests")
            .args_json(json!({}))
            .await?
            .json()?;
        if let Some(first) = requests.first() {
            break first.clone();
        }
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    };

    let response_payload = br#"{"status":"ok"}"#.to_vec();
    relayer
        .call(fetcher.id(), "store_response_chunk")
        .args_json(json!({
            "request_id": pending.request_id,
            "data": response_payload.clone(),
            "append": false,
        }))
        .max_gas()
        .transact()
        .await?
        .into_result()?;

    relayer
        .call(fetcher.id(), "respond")
        .args_json(json!({
            "request_id": pending.request_id,
            "yield_id": pending.yield_id.clone(),
            "body": json!(null),
        }))
        .max_gas()
        .transact()
        .await?
        .into_result()?;

    let fetch_result: FetchResultView = fetch_tx.await?.json()?;
    match fetch_result.status {
        FetchStatusView::Completed => (),
        FetchStatusView::TimedOut => panic!("fetch unexpectedly timed out"),
    }

    let body_bytes = fetch_result
        .body
        .as_ref()
        .expect("body should be present in completed result");
    assert_eq!(body_bytes, &response_payload);
    assert_eq!(fetch_result.request_id, pending.request_id);
    assert_eq!(fetch_result.url, pending.url);
    assert_eq!(fetch_result.caller, fetcher.id().to_string());

    let remaining: Vec<PendingRequestView> = fetcher
        .view("list_requests")
        .args_json(json!({}))
        .await?
        .json()?;
    assert!(
        remaining.is_empty(),
        "requests should be cleared after resume"
    );

    Ok(())
}
