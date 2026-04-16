# syntax=docker/dockerfile:1.7

# Multi-stage build: compile a release binary, then copy into a slim runtime.
# Designed for Render (native Docker), but works anywhere.

ARG RUST_VERSION=1.83
FROM rust:${RUST_VERSION}-slim-bookworm AS builder

RUN apt-get update \
    && apt-get install -y --no-install-recommends \
       pkg-config libssl-dev ca-certificates \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /build

# Cache dependencies first — copy only manifests, stub out sources.
COPY Cargo.toml Cargo.lock rust-toolchain.toml ./
COPY crates/log-server/Cargo.toml crates/log-server/Cargo.toml
COPY crates/notify/Cargo.toml crates/notify/Cargo.toml
RUN mkdir -p crates/log-server/src crates/notify/src \
    && echo "fn main() {}" > crates/log-server/src/main.rs \
    && echo "" > crates/log-server/src/lib.rs \
    && echo "" > crates/notify/src/lib.rs \
    && cargo build --release -p log-server || true

# Copy real sources and build for real.
COPY crates crates
RUN touch crates/log-server/src/main.rs crates/log-server/src/lib.rs crates/notify/src/lib.rs \
    && cargo build --release -p log-server

# --- runtime stage --------------------------------------------------------

FROM debian:bookworm-slim AS runtime

RUN apt-get update \
    && apt-get install -y --no-install-recommends \
       ca-certificates libssl3 sqlite3 tini \
    && rm -rf /var/lib/apt/lists/*

RUN useradd --create-home --uid 1000 --shell /usr/sbin/nologin crab
WORKDIR /app

COPY --from=builder /build/target/release/log-server /usr/local/bin/log-server

# Render mounts the persistent disk at /var/data (see render.yaml).
# Pre-create so the container user owns it before the disk is overlaid.
RUN mkdir -p /var/data && chown -R crab:crab /app /var/data
USER crab

ENV PORT=8080 \
    HOT_STORE=sqlite \
    DATABASE_URL=sqlite:///var/data/logs.db \
    COLD_STORE=noop \
    RUST_LOG=info

EXPOSE 8080

ENTRYPOINT ["/usr/bin/tini", "--"]
CMD ["log-server"]
