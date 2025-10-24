# HTTP Fetch Relayer

A relayer service for the NEAR HTTP fetch contract that monitors pending requests and fulfills them by making HTTP calls.

## Docker Usage

### Pull the Image

```bash
docker pull ghcr.io/r-near/near-http-fetch/relayer:latest
```

### Run the Container

```bash
docker run -d \
  --name http-fetch-relayer \
  -e RPC_URL="https://rpc.testnet.near.org" \
  -e CONTRACT_ID="your-contract.testnet" \
  -e RELAYER_ID="your-relayer.testnet" \
  -e RELAYER_PRIVATE_KEY="ed25519:YOUR_PRIVATE_KEY_HERE" \
  -e POLL_INTERVAL_SECS="5" \
  -e RUST_LOG="info" \
  ghcr.io/r-near/near-http-fetch/relayer:latest
```

### Environment Variables

| Variable | Required | Default | Description |
|----------|----------|---------|-------------|
| `RPC_URL` | Yes | - | NEAR RPC endpoint URL |
| `CONTRACT_ID` | Yes | - | HTTP fetch contract account ID |
| `RELAYER_ID` | Yes | - | Relayer account ID |
| `RELAYER_PRIVATE_KEY` | Yes | - | Relayer private key (ed25519:...) |
| `POLL_INTERVAL_SECS` | No | `5` | Polling interval in seconds |
| `RUST_LOG` | No | `info` | Log level (trace, debug, info, warn, error) |

### View Logs

```bash
docker logs -f http-fetch-relayer
```

### Stop the Container

```bash
docker stop http-fetch-relayer
```

## Building Locally

From the repository root:

```bash
docker build -t http-fetch-relayer -f relayer/Dockerfile .
```

## Development

### Running Locally

Create a `.env` file in the `relayer` directory:

```env
RPC_URL=https://rpc.testnet.near.org
CONTRACT_ID=your-contract.testnet
RELAYER_ID=your-relayer.testnet
RELAYER_PRIVATE_KEY=ed25519:YOUR_PRIVATE_KEY_HERE
POLL_INTERVAL_SECS=5
RUST_LOG=debug
```

Then run:

```bash
cd relayer
cargo run
```

### Log Levels

The relayer uses `tracing` for structured logging. You can control log verbosity with `RUST_LOG`:

- `RUST_LOG=error` - Only errors
- `RUST_LOG=warn` - Warnings and errors
- `RUST_LOG=info` - General info (default)
- `RUST_LOG=debug` - Detailed debug info
- `RUST_LOG=trace` - Very verbose trace-level logs

For module-specific logging:

```bash
RUST_LOG=relayer=debug,hyper=warn
```
