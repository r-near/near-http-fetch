# Build stage
FROM rust:1.83 AS builder

WORKDIR /usr/src/relayer

# Copy the relayer source
COPY relayer/Cargo.toml relayer/Cargo.lock ./
COPY relayer/src ./src

# Build the relayer in release mode
RUN cargo build --release

# Runtime stage
FROM debian:bookworm-slim

# Install OpenSSL and CA certificates for HTTPS
RUN apt-get update && \
    apt-get install -y --no-install-recommends \
    ca-certificates \
    libssl3 && \
    rm -rf /var/lib/apt/lists/*

# Copy the binary from builder
COPY --from=builder /usr/src/relayer/target/release/relayer /usr/local/bin/relayer

# Set environment variables with defaults
ENV RPC_URL=""
ENV CONTRACT_ID=""
ENV RELAYER_ID=""
ENV RELAYER_PRIVATE_KEY=""
ENV POLL_INTERVAL_SECS="5"
ENV RUST_LOG="info"

# Run the relayer
CMD ["relayer"]
