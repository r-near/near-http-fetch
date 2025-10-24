use near_sdk::borsh::{BorshDeserialize, BorshSerialize};
use near_sdk::serde::{Deserialize, Serialize};
use near_sdk::serde_json;
use near_sdk::store::IterableMap;
use near_sdk::{
    env, near, require, AccountId, BorshStorageKey, CryptoHash, Gas, GasWeight, PromiseResult,
};

const YIELD_REGISTER: u64 = 0;
const RESUME_GAS: Gas = Gas::from_tgas(20);

#[derive(BorshDeserialize, BorshSerialize)]
struct StoredRequest {
    yield_id: CryptoHash,
    url: String,
    caller: AccountId,
    context: Option<Vec<u8>>,
}

#[near(serializers = [json])]
#[derive(Clone)]
pub struct PendingRequest {
    pub request_id: u64,
    pub url: String,
    pub caller: AccountId,
    pub context: Option<Vec<u8>>,
    pub yield_id: Vec<u8>,
}

#[near(serializers = [json])]
#[derive(Clone, Copy)]
pub enum FetchStatus {
    Completed,
    TimedOut,
}

#[near(serializers = [json])]
#[derive(Clone)]
pub struct FetchResult {
    pub request_id: u64,
    pub url: String,
    pub status: FetchStatus,
    pub body: Option<Vec<u8>>,
    pub context: Option<Vec<u8>>,
    pub caller: AccountId,
}

#[derive(Serialize, Deserialize)]
#[serde(crate = "near_sdk::serde")]
struct FetchCallbackArgs {
    request_id: u64,
}

#[derive(BorshSerialize, BorshDeserialize, BorshStorageKey)]
enum StorageKey {
    Requests,
    ResponseBodies,
}

#[near(contract_state)]
pub struct Contract {
    trusted_relayer: AccountId,
    next_request_id: u64,
    requests: IterableMap<u64, StoredRequest>,
    response_bodies: IterableMap<u64, Vec<u8>>,
}

impl Contract {
    fn ensure_trusted(&self) {
        require!(
            env::predecessor_account_id() == self.trusted_relayer,
            "Only the trusted relayer can respond"
        );
    }
}

impl Default for Contract {
    fn default() -> Self {
        env::panic_str("Contract must be initialized with new(trusted_relayer)");
    }
}

#[near]
impl Contract {
    #[init]
    pub fn new(trusted_relayer: AccountId) -> Self {
        require!(!env::state_exists(), "Already initialized");
        Self {
            trusted_relayer,
            next_request_id: 0,
            requests: IterableMap::new(StorageKey::Requests),
            response_bodies: IterableMap::new(StorageKey::ResponseBodies),
        }
    }

    pub fn trusted_relayer(&self) -> AccountId {
        self.trusted_relayer.clone()
    }

    pub fn fetch(&mut self, url: String, context: Option<Vec<u8>>) {
        let caller = env::predecessor_account_id();
        let request_id = self.next_request_id;
        self.next_request_id = self
            .next_request_id
            .checked_add(1)
            .expect("Request id overflow");

        let callback_args = FetchCallbackArgs { request_id };
        let promise_id = env::promise_yield_create(
            "on_fetch_complete",
            &serde_json::to_vec(&callback_args).expect("Serialize callback args"),
            RESUME_GAS,
            GasWeight::default(),
            YIELD_REGISTER,
        );

        let yield_id: CryptoHash = env::read_register(YIELD_REGISTER)
            .expect("Failed to read yield register")
            .try_into()
            .expect("Invalid yield id");

        let stored = StoredRequest {
            yield_id,
            url: url.clone(),
            caller: caller.clone(),
            context: context.clone(),
        };
        self.requests.insert(request_id, stored);

        let event = serde_json::json!({
            "standard": "http_fetch",
            "version": "1.0.0",
            "event": "fetch_request",
            "data": [{
                "request_id": request_id,
                "url": url,
                "caller": caller,
            }]
        });
        env::log_str(&format!("EVENT_JSON:{}", event));

        env::promise_return(promise_id);
    }

    pub fn list_requests(&self) -> Vec<PendingRequest> {
        self.requests
            .iter()
            .map(|(request_id, req)| PendingRequest {
                request_id: *request_id,
                url: req.url.clone(),
                caller: req.caller.clone(),
                context: req.context.clone(),
                yield_id: req.yield_id.to_vec(),
            })
            .collect()
    }

    pub fn respond(&mut self, request_id: u64, yield_id: Vec<u8>, body: Option<Vec<u8>>) {
        self.ensure_trusted();

        let provided: CryptoHash = yield_id
            .as_slice()
            .try_into()
            .unwrap_or_else(|_| env::panic_str("Invalid yield id"));

        let Some(request) = self.requests.get(&request_id) else {
            env::panic_str("Unknown request id");
        };

        require!(
            request.yield_id == provided,
            "Yield id does not match stored request"
        );

        if let Some(data) = body {
            self.response_bodies.insert(request_id, data);
        } else if self.response_bodies.get(&request_id).is_none() {
            env::panic_str("No stored body for request");
        }

        env::promise_yield_resume(&request.yield_id, &[]);
    }

    pub fn store_response_chunk(&mut self, request_id: u64, data: Vec<u8>, append: bool) {
        self.ensure_trusted();
        let mut current = if append {
            self.response_bodies
                .get(&request_id)
                .cloned()
                .unwrap_or_default()
        } else {
            Vec::new()
        };
        current.extend_from_slice(&data);
        self.response_bodies.insert(request_id, current);
    }

    #[private]
    pub fn on_fetch_complete(&mut self, request_id: u64) -> FetchResult {
        let request = self
            .requests
            .remove(&request_id)
            .unwrap_or_else(|| env::panic_str("Missing request for callback"));

        let stored_body = self.response_bodies.remove(&request_id);

        match env::promise_result(0) {
            PromiseResult::Successful(_) => FetchResult {
                request_id,
                url: request.url,
                status: FetchStatus::Completed,
                body: stored_body,
                context: request.context,
                caller: request.caller,
            },
            PromiseResult::Failed => FetchResult {
                request_id,
                url: request.url,
                status: FetchStatus::TimedOut,
                body: None,
                context: request.context,
                caller: request.caller,
            },
        }
    }
}
