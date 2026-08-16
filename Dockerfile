# Multi-stage: the Rust toolchain is ~1.5 GB and is not needed to run the binary.
FROM rust:1-slim AS builder

# tokio-tungstenite is built with the "native-tls" feature, so the build needs
# OpenSSL headers and pkg-config (rustls would not, but that is not this crate's
# configuration).
RUN apt-get update \
    && apt-get install -y --no-install-recommends pkg-config libssl-dev \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /build

# Copy manifests first so the dependency layer caches across source edits.
COPY Cargo.toml Cargo.lock ./
RUN mkdir -p src && echo 'fn main() {}' > src/main.rs \
    && cargo build --release \
    && rm -rf src

COPY src ./src
# Touch main.rs so cargo rebuilds it rather than reusing the stub above.
RUN touch src/main.rs && cargo build --release


FROM debian:bookworm-slim

# ca-certificates: the feed is wss://, so certificate verification needs a trust
# store. libssl3: the runtime half of native-tls.
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates libssl3 \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app

COPY --from=builder /build/target/release/standx-dry-run-grid /usr/local/bin/
COPY grid_config.json ./

RUN mkdir -p results

ENV RUST_LOG=info

# Dry-run simulator only: this crate has no live order path at all (see
# src/main.rs "No credentials or authentication required"), so there is nothing
# to guard against here the way there is for the Lighter makers.
CMD ["standx-dry-run-grid", "grid_config.json"]
