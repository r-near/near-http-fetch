# Build stage
FROM rust:1.86 AS builder

WORKDIR /usr/src/relayer

# Copy only Cargo files first to cache dependencies
COPY relayer/Cargo.toml relayer/Cargo.lock ./

# Create a dummy main.rs to build dependencies
RUN mkdir src && \
    echo "fn main() {}" > src/main.rs && \
    cargo build --release && \
    rm -rf src

# Now copy the actual source code
COPY relayer/src ./src

# Build the relayer with dependencies cached
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
