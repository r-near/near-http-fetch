use std::{env, str::FromStr, sync::Arc, time::Duration};

use anyhow::{anyhow, Context, Result};
use near_api::types::{AccountId, Data, NearGas, TxExecutionStatus};
use near_api::{
    signer::Signer as InnerSigner,
    Contract, NetworkConfig, RPCEndpoint, Signer,
};
use reqwest::Client;
use serde::Deserialize;
use serde_json::json;
use tokio::time::sleep;

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

const CHUNK_SIZE: usize = 900;

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
        let rpc_url_parsed = url::Url::parse(rpc_url).context("invalid RPC_URL")?;
        let contract_id = AccountId::from_str(contract_id).context("invalid contract id")?;
        let relayer_id = AccountId::from_str(relayer_id).context("invalid relayer id")?;
        let secret_key = secret_key.parse().context("invalid relayer private key")?;

        let signer = Signer::new(InnerSigner::from_secret_key(secret_key))?;
        let network = build_network_config(rpc_url_parsed);
        let poll = Duration::from_secs(poll_interval_secs.unwrap_or(5).max(1));

        Ok(Self::new(network, contract_id, relayer_id, signer, poll))
    }

    pub fn from_env() -> Result<Self> {
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
    let pending = fetch_pending_requests(config).await?;
    if pending.is_empty() {
        return Ok(false);
    }

    for request in pending {
        println!(
            "Processing request {} for URL {}",
            request.request_id, request.url
        );
        handle_request(config, http, request).await?;
    }

    Ok(true)
}

pub async fn run(config: Config) -> Result<()> {
    let http = config.http_client()?;
    loop {
        if !process_once(&config, &http).await? {
            sleep(config.poll_interval).await;
        }
    }
}

async fn fetch_pending_requests(config: &Config) -> Result<Vec<PendingRequest>> {
    let contract = Contract(config.contract_id.clone());
    let response: Data<Vec<PendingRequest>> = contract
        .call_function("list_requests", ())
        .context("serializing list_requests args")?
        .read_only()
        .fetch_from(&config.network)
        .await?;
    Ok(response.data)
}

async fn handle_request(config: &Config, http: &Client, request: PendingRequest) -> Result<()> {
    let bytes = http
        .get(&request.url)
        .send()
        .await
        .with_context(|| format!("issuing GET to {}", request.url))?
        .bytes()
        .await
        .context("reading HTTP body")?
        .to_vec();

    if bytes.is_empty() {
        send_response(config, request.request_id, request.yield_id, Some(bytes)).await
    } else {
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

    outcome
        .into_result()
        .map_err(|failure| anyhow!("respond failed: {:?}", failure))?;

    Ok(())
}

async fn store_response_chunks(config: &Config, request_id: u64, body: &[u8]) -> Result<()> {
    let mut first = true;
    for chunk in body.chunks(CHUNK_SIZE) {
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
            .gas(NearGas::from_tgas(30))
            .with_signer(config.relayer_id.clone(), config.signer.clone())
            .wait_until(TxExecutionStatus::Executed)
            .send_to(&config.network)
            .await?;

        outcome
            .into_result()
            .map_err(|failure| anyhow!("store_response_chunk failed: {:?}", failure))?;

        first = false;
    }

    Ok(())
}
