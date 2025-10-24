# HTTP Fetcher for NEAR

A NEAR smart contract that enables on-chain code to request and receive off-chain HTTP data using NEAR's yield/resume mechanism. When a contract calls the fetcher, the transaction pauses (yields) while an off-chain relayer executes the HTTP request, then resumes execution with the response data.

## Live Contracts on Testnet

You can interact with these deployed contracts right now:

- **Fetcher**: `http-fetcher.testnet`
- **Relayer**: `http-relayer.testnet`
- **Weather Example**: `weather-example.testnet`

Anyone can call these contracts to test the HTTP fetching functionality.

## How It Works

```
┌─────────────┐         ┌──────────────┐         ┌─────────────┐
│   Weather   │ ──────▶ │  HTTP Fetch  │         │   Relayer   │
│  Contract   │  fetch  │   Contract   │         │  (off-chain)│
└─────────────┘         └──────────────┘         └─────────────┘
       │                       │                         │
       │                       │  (yields promise)       │
       │                       │                         │
       │                       │ ◀───── polls ────────   │
       │                       │         requests        │
       │                       │                         │
       │                       │                         │ ──▶ HTTP GET
       │                       │                         │     (OpenWeather API)
       │                       │                         │
       │                       │ ◀───── respond ──────   │ ◀── Response
       │                       │   (resume promise)      │
       │                       │                         │
       │ ◀──── callback ───────┤                         │
       │      (parsed data)    │                         │
       │                       │                         │
```

1. A consumer contract (like the weather example) calls `fetch(url, context)`
2. The fetcher contract creates a yielded promise and logs a fetch request
3. An off-chain relayer polls for pending requests via `list_requests()`
4. The relayer performs the HTTP GET request
5. For large responses, the relayer calls `store_response_chunk()` multiple times
6. The relayer calls `respond()` to resume the yielded promise
7. The fetcher returns a `FetchResult` to the caller's callback

## Quick Start with Deployed Contracts

Try fetching weather data using the deployed contracts:

```bash
# Request weather for a city
near call weather-example.testnet request_weather '{"city": "London"}' \
  --accountId your-account.testnet \
  --gas 300000000000000

# Wait a few seconds for the relayer to process the request

# Retrieve cached weather data
near view weather-example.testnet get_cached_weather '{"city": "London"}'
```

You can also directly use the HTTP fetcher:

```bash
# Make a custom HTTP request
near call http-fetcher.testnet fetch \
  '{"url": "https://api.example.com/data", "context": null}' \
  --accountId your-account.testnet \
  --gas 300000000000000
```

## Repository Structure

```
http-fetch/
├── src/lib.rs              # Core HTTP fetcher contract
├── examples/
│   └── weather/            # Example: weather data fetching contract
├── relayer/                # Off-chain HTTP relayer (Rust CLI + library)
├── tests/
│   ├── fetcher.rs          # Unit tests for fetcher contract
│   └── weather.rs          # Integration tests with weather example
├── Cargo.toml              # Workspace configuration
└── README.md
```

## Development Setup

### Prerequisites

- **Rust**: Install via [rustup](https://rustup.rs) (this repo uses Rust 1.86)
- **cargo-near**: For building NEAR contracts
  ```bash
  cargo install cargo-near
  ```
- **NEAR CLI**: For deploying and interacting with contracts
  ```bash
  npm install -g near-cli
  ```

### Building the Contracts

Build all contracts to WASM:

```bash
cargo near build
```

The output appears in `target/near/`:
- `http_fetch.wasm` - The main fetcher contract
- `weather.wasm` - The weather example contract

Build specific contracts:
```bash
cargo near build -p weather
```

### Running Tests

The test suite uses `near-workspaces` to spin up local sandbox chains:

```bash
cargo test -- --nocapture
```

Key tests:
- `fetcher_yield_resume_flow` - Tests basic yield/resume mechanism
- `weather_contract_flow` - Full integration test with the weather example

Note: The weather test makes real HTTP calls to OpenWeather API, so network access is required.

## Deploying Your Own Contracts

### 1. Deploy the Fetcher Contract

```bash
# Create a new account for the fetcher
near create-account fetcher.your-account.testnet \
  --masterAccount your-account.testnet

# Deploy the contract
cargo near deploy fetcher.your-account.testnet \
  --wasmFile target/near/http_fetch.wasm

# Initialize with your relayer account
near call fetcher.your-account.testnet new \
  '{"trusted_relayer": "relayer.your-account.testnet"}' \
  --accountId your-account.testnet
```

### 2. Deploy the Weather Example

```bash
# Create account
near create-account weather.your-account.testnet \
  --masterAccount your-account.testnet

# Deploy
cargo near deploy weather.your-account.testnet \
  --wasmFile target/near/weather.wasm

# Initialize with fetcher account
near call weather.your-account.testnet new \
  '{"fetcher_account": "fetcher.your-account.testnet"}' \
  --accountId your-account.testnet
```

## Running Your Own Relayer

The relayer is a Rust CLI that monitors fetch requests and fulfills them.

### Configuration

Create a `.env` file in the `relayer/` directory:

```env
RPC_URL=https://rpc.testnet.near.org
CONTRACT_ID=http-fetcher.testnet
RELAYER_ACCOUNT_ID=http-relayer.testnet
RELAYER_PRIVATE_KEY=ed25519:your_private_key_here
POLL_INTERVAL_SECS=5
```

Or export as environment variables.

### Running

```bash
cd relayer
cargo run
```

The relayer will:
1. Poll the fetcher contract every `POLL_INTERVAL_SECS` seconds
2. Execute HTTP GET requests for pending items
3. Upload large responses in chunks via `store_response_chunk()`
4. Resume the yielded promises via `respond()`

### Using as a Library

You can also use the relayer as a library in your own Rust projects:

```rust
use relayer::{RelayerConfig, run_relayer};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let config = RelayerConfig {
        rpc_url: "https://rpc.testnet.near.org".to_string(),
        contract_id: "http-fetcher.testnet".parse()?,
        relayer_account_id: "relayer.testnet".parse()?,
        relayer_private_key: "ed25519:...".to_string(),
        poll_interval_secs: 5,
    };

    run_relayer(config).await
}
```

## Building Your Own Consumer Contracts

Here's a minimal example of a contract that uses the HTTP fetcher:

```rust
use near_sdk::*;

#[ext_contract(http_fetcher)]
trait HttpFetcher {
    fn fetch(&mut self, url: String, context: Option<Vec<u8>>) -> FetchResult;
}

#[near(contract_state)]
pub struct MyContract {
    fetcher_account: AccountId,
}

#[near]
impl MyContract {
    pub fn fetch_data(&mut self, url: String) -> Promise {
        http_fetcher::ext(self.fetcher_account.clone())
            .with_static_gas(Gas::from_tgas(40))
            .fetch(url, None)
            .then(
                Self::ext(env::current_account_id())
                    .with_static_gas(Gas::from_tgas(20))
                    .on_fetch_result()
            )
    }

    #[private]
    pub fn on_fetch_result(
        &mut self,
        #[callback_result] result: Result<FetchResult, PromiseError>,
    ) {
        match result {
            Ok(fetch_result) => {
                if let Some(body) = fetch_result.body {
                    // Process the response body
                    env::log_str(&format!("Received {} bytes", body.len()));
                }
            }
            Err(_) => env::log_str("Fetch failed"),
        }
    }
}
```

## API Reference

### Fetcher Contract Methods

#### `new(trusted_relayer: AccountId)`
Initialize the contract with the relayer account that's authorized to fulfill requests.

#### `fetch(url: String, context: Option<Vec<u8>>)`
Request HTTP data from a URL. The `context` parameter is passed through to your callback for request tracking. This function yields and returns a `FetchResult`.

#### `list_requests() -> Vec<PendingRequest>`
Returns all pending fetch requests (used by relayers).

#### `respond(request_id: u64, yield_id: Vec<u8>, body: Option<Vec<u8>>)`
Resume a yielded promise with response data. Only callable by the trusted relayer.

#### `store_response_chunk(request_id: u64, data: Vec<u8>, append: bool)`
Store response data in chunks (for large payloads). Only callable by the trusted relayer.

### FetchResult Structure

```rust
pub struct FetchResult {
    pub request_id: u64,
    pub url: String,
    pub status: FetchStatus,      // Completed or TimedOut
    pub body: Option<Vec<u8>>,
    pub context: Option<Vec<u8>>,
    pub caller: AccountId,
}
```

## Security Considerations

1. **Trusted Relayer**: Only the configured relayer account can fulfill requests. Choose this account carefully.
2. **Gas Limits**: Ensure sufficient gas for the full yield/resume cycle (typically 40+ TGas).
3. **Response Size**: Large responses use chunked storage to avoid receipt size limits.
4. **Validation**: Always validate response data in your callback before using it.

## Extending the Project

- **Add more examples**: Create consumer contracts under `examples/` for different use cases (price feeds, API integrations, etc.)
- **Multi-language relayers**: Implement relayers in other languages using NEAR APIs
- **Enhanced relayer logic**: Add authentication, rate limiting, caching, or webhook support
- **POST/PUT support**: Extend the fetcher to support HTTP methods beyond GET

## Helpful Resources

- [NEAR Rust SDK Documentation](https://docs.near.org/sdk/rust/introduction)
- [Yield/Resume Documentation](https://docs.near.org/develop/smart-contracts/anatomy/yield-resume)
- [near-workspaces Testing Framework](https://github.com/near/near-workspaces-rs)
- [cargo-near](https://github.com/near/cargo-near)
- [NEAR CLI Documentation](https://docs.near.org/tools/near-cli)

## License

MIT
