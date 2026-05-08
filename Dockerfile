# syntax=docker/dockerfile:1.7

# Multi-stage build with BuildKit cache mounts for Cargo's registry, git
# index, and target/ directory. Render preserves the BuildKit cache
# per-service across deploys, so a typical "source-only" change rebuilds
# in ~1 min instead of ~10 min once the cache is warm.
#
# First deploy after this Dockerfile change populates the cache and is the
# same speed as before. Subsequent deploys see the speedup. Clearing the
# cache (manually in Render UI, or by deleting/recreating the service)
# resets to cold-build time.

ARG RUST_VERSION=1.83
FROM rust:${RUST_VERSION}-slim-bookworm AS builder

# Render injects RENDER_GIT_COMMIT as a build ARG; pass it through to build.rs
# so the dashboard footer shows the real commit instead of "unknown".
ARG RENDER_GIT_COMMIT=""
ENV RENDER_GIT_COMMIT=$RENDER_GIT_COMMIT

# Build-time apt deps. Cache mount avoids re-downloading .deb files on
# subsequent builds when the package list hasn't changed.
RUN --mount=type=cache,target=/var/cache/apt,sharing=locked \
    --mount=type=cache,target=/var/lib/apt,sharing=locked \
    rm -f /etc/apt/apt.conf.d/docker-clean \
    && apt-get update \
    && apt-get install -y --no-install-recommends \
       pkg-config libssl-dev ca-certificates

WORKDIR /build

# ─── Layer 1: manifests only ──────────────────────────────────────────────
# Copying just the lockfile + Cargo.toml files lets the dep-resolution
# layer cache as long as no manifest changes — even when source does.
COPY Cargo.toml Cargo.lock rust-toolchain.toml ./
COPY crates/log-server/Cargo.toml crates/log-server/Cargo.toml
COPY crates/notify/Cargo.toml crates/notify/Cargo.toml

# ─── Layer 2: stub source files ───────────────────────────────────────────
# So cargo can resolve + build deps without real source. Pre-warms the
# target/ cache mount on cold builds (when Render's per-service cache is
# empty); near-instant on warm builds.
RUN mkdir -p crates/log-server/src crates/notify/src \
    && echo "fn main() {}" > crates/log-server/src/main.rs \
    && touch crates/log-server/src/lib.rs crates/notify/src/lib.rs

# ─── Layer 3: dep-only pre-build ──────────────────────────────────────────
# Cache mounts persist between builds:
#   - /usr/local/cargo/registry  : downloaded crate index + sources
#   - /usr/local/cargo/git       : git deps (none right now, but cheap)
#   - /build/target              : compiled artifacts (the big one)
# `|| true` tolerates a stub-build failure (e.g., empty lib.rs warning
# treated as error by some toolchains) — the real build in Layer 5 catches
# any genuine compile error.
RUN --mount=type=cache,target=/usr/local/cargo/registry,sharing=locked \
    --mount=type=cache,target=/usr/local/cargo/git,sharing=locked \
    --mount=type=cache,target=/build/target,sharing=locked \
    cargo build --release -p log-server || true

# ─── Layer 4: real source ─────────────────────────────────────────────────
# This is the layer that changes most often. Everything above is cached
# unless manifests or the toolchain change.
COPY crates crates

# ─── Layer 5: real build ──────────────────────────────────────────────────
# Cache mounts again — Cargo's incremental compilation reuses object
# files in target/ from the dep build, so only log-server itself recompiles.
#
# CRITICAL: target/ is a cache mount, which means it is NOT preserved in
# the resulting image layer. The binary must be copied out to a regular
# path (/build/log-server-bin) before the next stage can `COPY --from=builder`.
RUN --mount=type=cache,target=/usr/local/cargo/registry,sharing=locked \
    --mount=type=cache,target=/usr/local/cargo/git,sharing=locked \
    --mount=type=cache,target=/build/target,sharing=locked \
    touch crates/log-server/src/main.rs crates/log-server/src/lib.rs crates/notify/src/lib.rs \
    && cargo build --release -p log-server \
    && cp target/release/log-server /build/log-server-bin

# ─── runtime stage ────────────────────────────────────────────────────────

FROM debian:bookworm-slim AS runtime

RUN --mount=type=cache,target=/var/cache/apt,sharing=locked \
    --mount=type=cache,target=/var/lib/apt,sharing=locked \
    rm -f /etc/apt/apt.conf.d/docker-clean \
    && apt-get update \
    && apt-get install -y --no-install-recommends \
       ca-certificates libssl3 sqlite3 tini curl

RUN useradd --create-home --uid 1000 --shell /usr/sbin/nologin crab
WORKDIR /app

# Copy from /build/log-server-bin (NOT /build/target/release/...) — the
# target dir is a cache mount and doesn't exist in the image layer.
COPY --from=builder /build/log-server-bin /usr/local/bin/log-server

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
