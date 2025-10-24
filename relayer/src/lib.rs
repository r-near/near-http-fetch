use std::{env, str::FromStr, sync::Arc, time::{Duration, Instant}};

use anyhow::{anyhow, Context, Result};
use near_api::types::{
    transaction::actions::{Action, FunctionCallAction},
    AccountId, Data, NearGas, TxExecutionStatus,
};
use near_api::{
    signer::Signer as InnerSigner,
    Contract, NetworkConfig, RPCEndpoint, Signer, Transaction,
};
use reqwest::Client;
use serde::Deserialize;
use serde_json::json;
use tokio::time::sleep;
use tracing::{debug, error, info, trace};

#[derive(Clone, Deserialize)]
struct PendingRequest {
    request_id: u64,
    url: String,
    #[serde(default)]
    #[allow(dead_code)]
    caller: String,
    #[serde(default)]
    #[allow(dead_code)]
    context: Option<Vec<u8>>,
    yield_id: Vec<u8>,
}

const CHUNK_SIZE: usize = 300_000; // 300 KB - tested to use ~207 TGas in batch transactions (300 TGas limit)

#[derive(Clone)]
pub struct Config {
    pub network: NetworkConfig,
    pub contract_id: AccountId,
    pub relayer_id: AccountId,
    pub signer: Arc<Signer>,
    pub poll_interval: Duration,
}

impl Config {
    pub fn new(
        network: NetworkConfig,
        contract_id: AccountId,
        relayer_id: AccountId,
        signer: Arc<Signer>,
        poll_interval: Duration,
    ) -> Self {
        Self {
            network,
            contract_id,
            relayer_id,
            signer,
            poll_interval,
        }
    }

    pub fn from_parts(
        rpc_url: &str,
        contract_id: &str,
        relayer_id: &str,
        secret_key: &str,
        poll_interval_secs: Option<u64>,
    ) -> Result<Self> {
        debug!("Parsing configuration from provided parameters");

        let rpc_url_parsed = url::Url::parse(rpc_url).context("invalid RPC_URL")?;
        let contract_id = AccountId::from_str(contract_id).context("invalid contract id")?;
        let relayer_id = AccountId::from_str(relayer_id).context("invalid relayer id")?;
        let secret_key = secret_key.parse().context("invalid relayer private key")?;

        let signer = Signer::new(InnerSigner::from_secret_key(secret_key))?;
        let network = build_network_config(rpc_url_parsed.clone());
        let poll = Duration::from_secs(poll_interval_secs.unwrap_or(5).max(1));

        info!(
            rpc_url = %rpc_url_parsed,
            contract_id = %contract_id,
            relayer_id = %relayer_id,
            poll_interval_secs = poll.as_secs(),
            "Relayer configuration initialized"
        );

        Ok(Self::new(network, contract_id, relayer_id, signer, poll))
    }

    pub fn from_env() -> Result<Self> {
        debug!("Loading configuration from environment variables");

        let rpc_url = env::var("RPC_URL").context("RPC_URL env var missing")?;
        let contract_id = env::var("CONTRACT_ID").context("CONTRACT_ID env var missing")?;
        let relayer_id =
            env::var("RELAYER_ACCOUNT_ID").context("RELAYER_ACCOUNT_ID env var missing")?;
        let secret_key =
            env::var("RELAYER_PRIVATE_KEY").context("RELAYER_PRIVATE_KEY env var missing")?;
        let poll = env::var("POLL_INTERVAL_SECS")
            .ok()
            .and_then(|v| v.parse().ok());

        Self::from_parts(&rpc_url, &contract_id, &relayer_id, &secret_key, poll)
    }

    pub fn http_client(&self) -> Result<Client> {
        Ok(Client::builder()
            .user_agent("http-fetch-relayer/0.1.0")
            .build()?)
    }
}

fn build_network_config(rpc_url: url::Url) -> NetworkConfig {
    NetworkConfig {
        network_name: "custom".to_string(),
        rpc_endpoints: vec![RPCEndpoint::new(rpc_url)],
        linkdrop_account_id: None,
        near_social_db_contract_account_id: None,
        faucet_url: None,
        meta_transaction_relayer_url: None,
        fastnear_url: None,
        staking_pools_factory_account_id: None,
    }
}

pub async fn process_once(config: &Config, http: &Client) -> Result<bool> {
    trace!("Fetching pending requests from contract");
    let pending = fetch_pending_requests(config).await?;

    if pending.is_empty() {
        debug!("No pending requests found");
        return Ok(false);
    }

    info!(count = pending.len(), "Found pending requests to process");

    for request in pending {
        info!(
            request_id = request.request_id,
            url = %request.url,
            caller = %request.caller,
            "Processing request"
        );
        handle_request(config, http, request).await?;
    }

    Ok(true)
}

pub async fn run(config: Config) -> Result<()> {
    info!("Starting relayer main loop");
    let http = config.http_client()?;

    loop {
        match process_once(&config, &http).await {
            Ok(true) => {
                debug!("Processed requests, checking for more immediately");
            }
            Ok(false) => {
                trace!(
                    poll_interval_secs = config.poll_interval.as_secs(),
                    "No requests found, sleeping before next poll"
                );
                sleep(config.poll_interval).await;
            }
            Err(e) => {
                error!(error = %e, "Error processing requests, will retry after poll interval");
                sleep(config.poll_interval).await;
            }
        }
    }
}

async fn fetch_pending_requests(config: &Config) -> Result<Vec<PendingRequest>> {
    let start = Instant::now();
    let contract = Contract(config.contract_id.clone());

    debug!(contract_id = %config.contract_id, "Calling list_requests on contract");

    let response: Data<Vec<PendingRequest>> = contract
        .call_function("list_requests", ())
        .context("serializing list_requests args")?
        .read_only()
        .fetch_from(&config.network)
        .await?;

    let elapsed = start.elapsed();
    debug!(
        count = response.data.len(),
        elapsed_ms = elapsed.as_millis(),
        "Fetched pending requests"
    );

    Ok(response.data)
}

async fn handle_request(config: &Config, http: &Client, request: PendingRequest) -> Result<()> {
    let request_id = request.request_id;
    let url = &request.url;

    info!(request_id, url = %url, "Starting HTTP fetch");
    let fetch_start = Instant::now();

    let response = http
        .get(url)
        .send()
        .await
        .with_context(|| format!("issuing GET to {}", url))?;

    let status = response.status();
    let fetch_elapsed = fetch_start.elapsed();

    info!(
        request_id,
        url = %url,
        status = status.as_u16(),
        elapsed_ms = fetch_elapsed.as_millis(),
        "HTTP request completed"
    );

    let bytes = response
        .bytes()
        .await
        .context("reading HTTP body")?
        .to_vec();

    let body_size = bytes.len();
    info!(
        request_id,
        body_size_bytes = body_size,
        "HTTP response body received"
    );

    if bytes.is_empty() {
        debug!(request_id, "Response body is empty, sending inline");
        send_response(config, request.request_id, request.yield_id, Some(bytes)).await
    } else if bytes.len() <= CHUNK_SIZE {
        // Single chunk - use batch transaction
        info!(
            request_id,
            body_size_bytes = body_size,
            "Response fits in single chunk, using batch transaction"
        );
        send_batch_chunk_and_respond(config, request.request_id, request.yield_id, bytes).await
    } else {
        let chunk_count = body_size.div_ceil(CHUNK_SIZE);
        info!(
            request_id,
            body_size_bytes = body_size,
            chunk_count,
            chunk_size_bytes = CHUNK_SIZE,
            "Response body will be stored in chunks"
        );
        store_response_chunks(config, request.request_id, &bytes).await?;
        send_response(config, request.request_id, request.yield_id, None).await
    }
}

async fn send_response(
    config: &Config,
    request_id: u64,
    yield_id: Vec<u8>,
    body: Option<Vec<u8>>,
) -> Result<()> {
    let body_size = body.as_ref().map(|b| b.len());
    info!(
        request_id,
        body_size_bytes = body_size,
        has_inline_body = body.is_some(),
        "Submitting 'respond' transaction"
    );

    let tx_start = Instant::now();
    let outcome = Contract(config.contract_id.clone())
        .call_function(
            "respond",
            json!({
                "request_id": request_id,
                "yield_id": yield_id,
                "body": body,
            }),
        )
        .context("serializing respond args")?
        .transaction()
        .gas(NearGas::from_tgas(50))
        .with_signer(config.relayer_id.clone(), config.signer.clone())
        .wait_until(TxExecutionStatus::Executed)
        .send_to(&config.network)
        .await?;

    let tx_elapsed = tx_start.elapsed();

    match outcome.into_result() {
        Ok(_) => {
            info!(
                request_id,
                elapsed_ms = tx_elapsed.as_millis(),
                "Response transaction succeeded"
            );
            Ok(())
        }
        Err(failure) => {
            error!(
                request_id,
                error = ?failure,
                elapsed_ms = tx_elapsed.as_millis(),
                "Response transaction failed"
            );
            Err(anyhow!("respond failed: {:?}", failure))
        }
    }
}

async fn store_response_chunks(config: &Config, request_id: u64, body: &[u8]) -> Result<()> {
    let total_chunks = body.len().div_ceil(CHUNK_SIZE);
    info!(
        request_id,
        total_chunks,
        total_size_bytes = body.len(),
        "Starting to store response chunks"
    );

    let mut first = true;
    let mut chunk_index = 0;

    for chunk in body.chunks(CHUNK_SIZE) {
        chunk_index += 1;
        debug!(
            request_id,
            chunk_index,
            total_chunks,
            chunk_size_bytes = chunk.len(),
            is_first = first,
            "Submitting chunk transaction"
        );

        let tx_start = Instant::now();
        let outcome = Contract(config.contract_id.clone())
            .call_function(
                "store_response_chunk",
                json!({
                    "request_id": request_id,
                    "data": chunk,
                    "append": !first,
                }),
            )?
            .transaction()
            .gas(NearGas::from_tgas(100))
            .with_signer(config.relayer_id.clone(), config.signer.clone())
            .wait_until(TxExecutionStatus::Executed)
            .send_to(&config.network)
            .await?;

        let tx_elapsed = tx_start.elapsed();

        match outcome.into_result() {
            Ok(_) => {
                debug!(
                    request_id,
                    chunk_index,
                    total_chunks,
                    elapsed_ms = tx_elapsed.as_millis(),
                    "Chunk transaction succeeded"
                );
            }
            Err(failure) => {
                error!(
                    request_id,
                    chunk_index,
                    total_chunks,
                    error = ?failure,
                    elapsed_ms = tx_elapsed.as_millis(),
                    "Chunk transaction failed"
                );
                return Err(anyhow!("store_response_chunk failed: {:?}", failure));
            }
        }

        first = false;
    }

    info!(
        request_id,
        total_chunks,
        "All chunks stored successfully"
    );

    Ok(())
}

async fn send_batch_chunk_and_respond(
    config: &Config,
    request_id: u64,
    yield_id: Vec<u8>,
    data: Vec<u8>,
) -> Result<()> {
    let data_size = data.len();
    info!(
        request_id,
        data_size_bytes = data_size,
        "Submitting batch transaction: store_response_chunk + respond"
    );

    let tx_start = Instant::now();

    let outcome = Transaction::construct(config.relayer_id.clone(), config.contract_id.clone())
        .add_action(Action::FunctionCall(Box::new(FunctionCallAction {
            method_name: "store_response_chunk".to_string(),
            args: serde_json::to_vec(&json!({
                "request_id": request_id,
                "data": data,
                "append": false,
            }))?,
            gas: NearGas::from_tgas(250),
            deposit: Default::default(),
        })))
        .add_action(Action::FunctionCall(Box::new(FunctionCallAction {
            method_name: "respond".to_string(),
            args: serde_json::to_vec(&json!({
                "request_id": request_id,
                "yield_id": yield_id,
                "body": json!(null),
            }))?,
            gas: NearGas::from_tgas(50),
            deposit: Default::default(),
        })))
        .with_signer(config.signer.clone())
        .send_to(&config.network)
        .await?;

    let tx_elapsed = tx_start.elapsed();

    match outcome.into_result() {
        Ok(_) => {
            info!(
                request_id,
                elapsed_ms = tx_elapsed.as_millis(),
                "Batch transaction succeeded"
            );
            Ok(())
        }
        Err(failure) => {
            error!(
                request_id,
                error = ?failure,
                elapsed_ms = tx_elapsed.as_millis(),
                "Batch transaction failed"
            );
            Err(anyhow!("batch transaction failed: {:?}", failure))
        }
    }
}
