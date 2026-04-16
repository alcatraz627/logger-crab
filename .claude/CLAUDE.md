# logger-crab — Centralized Logging Service

Standalone logging service for the Versable stack. Rust + axum + sqlx + maud,
deployed on Render Starter ($7/mo). SQLite hot tier, S3 NDJSON cold tier.

**The whole point:** show every log line for a single `request_id`, in order,
across every service it touched.

## Repo

This repo is **independent of the main Versable app**. The mirror at
`~/Code/Versable/enhancement-product/{frontend,backend}` is for **reference
only** — copy patterns, lift event names, study existing emitters there. All
new code goes in this repo. Do not write into `enhancement-product/`.

The reference repo is added to the session via `/add-dir` — you can read
files from it to understand callsite patterns, but treat it as read-only.

## Tech Stack

Rust (stable) | axum (HTTP) | sqlx (SQLite + migrations) | tokio (runtime) |
maud (server-rendered HTML) | aws-sdk-s3 (cold storage) | tower-http (middleware) |
tracing (structured logs of the log service itself, ironically) |
htmx (dashboard interactivity) | thiserror (error types) | serde (JSON)

Shippers: TypeScript (`@versable/logger-crab`) + Python (`logger_crab`).

## Commands

```bash
cargo run                    # Dev server (default port 8080)
cargo check                  # Type check + borrow check (no build)
cargo clippy --all-targets   # Lints
cargo test                   # Unit + integration tests
cargo fmt                    # Format

# Database
sqlx migrate run             # Apply pending migrations to dev DB
sqlx migrate add <name>      # Generate new migration

# Local dev with in-memory store (no SQLite/S3 needed)
HOT_STORE=memory COLD_STORE=memory cargo run

# Smoke tests against deployed service
INGEST_TOKEN=... npm --prefix tests/seed start  # seed dummy data
INGEST_TOKEN=... ts-node tests/scenarios/pipeline_preview_happy_path.ts
```

Package manager: cargo + npm (for shipper + tests). MSRV: stable Rust.

## Directory Conventions

```
crates/
├── log-server/      The actual service binary (axum + sqlx)
│   └── src/
│       ├── store/   Pluggable HotStore + ColdStore traits + impls
│       ├── routes/  /ingest, /logs, /dashboard
│       ├── dashboard/  maud partials (live tail, request inspector, etc)
│       └── ...
└── notify/          Extractable Slack + SES helpers (workspace member)

shippers/
├── typescript/      @versable/logger-crab — Node + Browser
└── python/          logger_crab — loguru sink + ContextVar helpers

tests/
├── scenarios/       End-to-end simulators per real flow
└── seed/            Dummy data seeder for deployed app

docs/
├── INTEGRATIONS.md  per-emitter setup recipes
├── DEPLOY.md        Render walkthrough
├── SCHEMA.md        event schema reference
├── EVENT_TAXONOMY.md  approved (domain, entity) pairs
├── STORAGE.md       writing a new HotStore / ColdStore impl
├── SHIPPERS.md      shipper API design (drafted in Phase 7.6)
└── DASHBOARD.md     query examples
```

**Naming:**

- Rust modules: `snake_case.rs`
- Maud templates: lowercase with role suffix (`live_tail.rs`, `request_inspector.rs`)
- Migration files: `YYYYMMDDHHMMSS_description.sql` (sqlx default)
- Scenario tests: `<flow>_<variant>.ts` (e.g. `pipeline_preview_failure.ts`)

## Core Patterns

- **All store access goes through traits.** Routes depend on
  `Arc<dyn HotStore>` / `Arc<dyn ColdStore>`, never on concrete types.
  See `crates/log-server/src/store/mod.rs` and Section 4.5 of PLAN.md.
- **Validation at ingest, not query.** `POST /ingest` validates the envelope
  shape + event-name format (Section 4.4). Query API trusts whatever's in the
  store.
- **Per-event accept/reject.** A single bad event in a batch never fails the
  whole batch — it gets logged as `service=log-server, event=ingest.rejected`
  and the rest go through.
- **Server-rendered dashboards.** No SPA. Maud + htmx + Tailwind (or
  hand-written CSS). Dark mode default per global rule.
- **Sentry is opt-in.** Service must run fully without Sentry configured.

## Key Gotchas

1. SQLite WAL mode + persistent disk: don't snapshot mid-checkpoint; rotation
   job uses `BEGIN IMMEDIATE` to avoid races with the live tail.
2. The hot tier's `payload` column is JSON-as-TEXT — query it with
   SQLite's `json_extract()` or push payload filters into the JSON1 path.
3. Cold-tier S3 keys are `{env}/{service}/{YYYY}/{MM}/{DD}/{HH}.ndjson.gz`.
   Don't change this layout casually — lifecycle policies key off it.
4. `request_id` ALWAYS comes in via `X-Request-ID` header from emitters.
   The service does NOT generate request_ids — emitters' shippers do.
5. The dashboard's Request Inspector is the headline feature. If a query
   for one `request_id` is slow, that's a P0 — `idx_events_request_id`
   should make it sub-millisecond at our hot-tier sizes.
6. Cargo workspace: `notify` crate must NOT depend on `log-server`. It can
   be lifted into other Versable repos via `notify = { git = "..." }`.

## Planning Notes

Authoritative spec: `.claude/whiteboard/logging/PLAN.md` (~1500 lines, 15
sections). Read this before making architectural decisions.

Research synthesis: `.claude/whiteboard/logging/RESEARCH.md` — survey of
13 existing logging systems with feature ideas tagged V1/V1.5/V2.

## Behavioral Rules

- **Do not edit `~/Code/Versable/enhancement-product/`** — that's a separate
  project, read-only reference for this repo. Lift event names and
  understand callsite patterns from there, but write all changes here.
- **🚫 NEVER COMMIT OR PUSH unless explicitly asked.** Same rule as the
  reference repo. The user will run git operations themselves.
- **Test written code.** Per global rule, every non-trivial change gets
  verified. After writing a route, curl it. After writing a store impl,
  run the in-memory test scenarios against it.
- **Diagrams go in `docs/` AND `~/.claude/assets/diagrams/`.** Any diagram
  rendered for the user in this project is saved to both locations as
  markdown with the ASCII art in a `txt` code block. Convention set
  2026-04-17.
- **First live demo must use dummy-marked seeded data.** When the app reaches
  a runnable state, launch it with seeded events AND a background mock
  script that keeps POSTing events marked with `"dummy"` (either in the
  `event` name like `dummy.heartbeat` or as a `dummy: true` flag in the
  payload) so the dashboard shows live activity. This lets real traffic be
  distinguished and filtered out later.
- **Post-session**: prepend a runtime note to `.claude/skills/runtime-notes.md`
  (will be created on first session that produces insights).

## Notes Migration

This repo's planning notes were originally drafted in the
`enhancement-product/frontend` repo at
`.claude/whiteboard/logging/{PLAN.md,README.md}` because the discussion
started there. They were copied here on 2026-04-16. The originals in
the reference repo can be considered obsolete from now on — this is the
source of truth.
