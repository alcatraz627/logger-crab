# logger-crab — V1 Architecture

Centralized logging V1: SQLite hot tier + S3 cold tier, served by a single Rust axum service on Render.

## System diagram

```txt
EMITTERS — thin client libs (TS + Python)
╭──────────────╮  ╭──────────────╮  ╭──────────────╮  ╭──────────────╮
│   Next.js    │  │   FastAPI    │  │    Worker    │  │  Cron Jobs   │
│    Vercel    │  │    Render    │  │    Render    │  │    Render    │
╰──────────────╯  ╰──────────────╯  ╰──────────────╯  ╰──────────────╯

        │         │         │         │
        │         │         │         │   batched POST /ingest
        ▼         ▼         ▼         ▼   (X-Request-ID header)

LOG SERVICE — logger-crab (Rust + axum, Render)
╭──────────────╮  ╭──────────────╮  ╭──────────────╮  ╭──────────────╮
│ POST /ingest │  │  GET /logs   │  │    GET /     │  │  GET /tail   │
│    writer    │  │  query API   │  │  dashboard   │  │   SSE tail   │
╰──────────────╯  ╰──────────────╯  ╰──────────────╯  ╰──────────────╯

                    │              │
                    ▼              ▼

┌──────────────────────────┐  rotation →  ┌──────────────────────────┐
│       HOT — SQLite       │              │     COLD — S3 NDJSON     │
│     24–48h retention     │              │     rotated from hot     │
│    indexed by req_id     │              │    long-term archive     │
└──────────────────────────┘              └──────────────────────────┘
```

## Request-ID propagation

A single `X-Request-ID` header threads through every hop so the dashboard can reconstruct
the full story of a distributed request.

```txt
  UI (Next.js)        FastAPI            Redis              Credit Worker
      │                  │                  │                     │
      │── fetch() ──────▶│                  │                     │
      │  X-Request-ID:   │                  │                     │
      │  req_abc123      │                  │                     │
      │                  │                  │                     │
      │                  │── LPUSH event ──▶│                     │
      │                  │   { request_id:  │                     │
      │                  │     "req_abc123",│                     │
      │                  │     job_id, ... }│                     │
      │                  │                  │                     │
      │                  │                  │── BRPOP ───────────▶│
      │                  │                  │   (payload carries  │
      │                  │                  │    request_id)      │
      │                  │                  │                     │
      │  ┌─────────────────────────────────────────────────────┐  │
      │  │  All emit logs tagged with request_id=req_abc123    │  │
      │  │  → /ingest                                          │  │
      │  │  Sentry.scope.set_tag("request_id", "req_abc123")   │  │
      │  └─────────────────────────────────────────────────────┘  │
      │                                                           │
      ▼                                                           ▼
   Dashboard "Request Inspector" → filter by request_id →
   full story: UI click, FastAPI handler, Redis handoff, Worker run, Sentry issue
```

## Notes

- **Ingress**: single `POST /ingest` endpoint accepts OTel-style batch envelope (`resource` +
  `scope` once per batch, `events[]` per record) — saves 60–80% bandwidth vs per-event duplication.
- **Hot tier**: SQLite, 24–48h window, indexed by `request_id`, queryable at sub-100ms for live
  debugging and the dashboard.
- **Cold tier**: NDJSON rotated to S3 for long-term retention; queryable via offline tooling.
- **Dashboard**: server-rendered `maud` HTML on `GET /`; live tail via `GET /tail` SSE backed by a
  `tokio::sync::broadcast` channel forked off the ingest path.
- **Sentry**: every emit also seeds `Sentry.scope.set_tag("request_id", …)` so error grouping
  remains queryable by the same id used in logs.

## Canonical copy

Also mirrored at `~/.claude/assets/diagrams/logger-crab-v1-architecture.md` and
`~/.claude/assets/diagrams/logger-crab-request-id-flow.md`.
