# logger-crab — Centralized Logging Service V1 Plan

<!-- sessions: log-v1-a3@2026-04-16 -->

**Repo:** `github.com/versable/logger-crab` (separate, standalone)
**Status:** Decisions locked, ready to scaffold
**Goal:** Crude V1 centralized logging across the Versable stack. Collect freely now, learn what queries matter, harden in V2.
**Owner:** Aakarsh
**Last updated:** 2026-04-16

---

## 0. Why this exists

Logs are scattered across five runtimes (Next.js web, Next.js serverless, FastAPI backend, credit worker, cron jobs) with three different styles (loguru, console.log, Sentry). Debugging a cross-service flow — pipeline preview (UI → FastAPI → Redis → worker → Postgres) — means tailing three places and praying timestamps line up.

This service solves one concrete pain: **"show me every log line for request X, in order, across every service it touched."** Everything else is secondary.

### Non-goals for V1

- Not a metrics/tracing platform — Sentry covers that
- Not high-throughput ingestion — we're at ~10k events/day, not millions
- Not production-grade observability — disposable stepping stone before Loki/ClickHouse if needed
- Not a Sentry replacement — Sentry handles errors + alerting; this handles narrative log streams

---

## 1. Locked decisions

| Decision                        | Choice                                                                              | Rationale                                                   |
| ------------------------------- | ----------------------------------------------------------------------------------- | ----------------------------------------------------------- |
| **Repo**                        | Separate: `github.com/versable/logger-crab`                                         | Independent lifecycle from main app; deployable standalone  |
| **Language**                    | Rust (axum + sqlx + tokio + maud + aws-sdk-s3)                                      | Learning value + low maintenance + 15 MB RAM forever        |
| **Hot store**                   | SQLite on Render persistent disk (FTS5 + JSON1)                                     | Zero managed-DB cost; survives restarts                     |
| **Cold store**                  | S3 NDJSON gzip, hourly rotation, dedicated `versable-logs` bucket                   | Cheap archive + lifecycle policies                          |
| **Hosting**                     | Render Starter ($7/mo), one deploy serves all envs                                  | Cheapest viable; same platform as backend                   |
| **Render Environment grouping** | New env "shared-infra" (separate from app dev/prod)                                 | Decouples log-service lifecycle from app deploys            |
| **Multi-env strategy**          | One service, `env` field on every event, S3 prefix scoped by env                    | $7 vs $21; cross-env queries trivial                        |
| **Auth**                        | Two env-var bearer tokens: `INGEST_TOKEN`, `DASHBOARD_TOKEN`                        | Debuggable when everything else is broken; OAuth = Phase 5+ |
| **Zero app dependencies**       | Service is fully standalone — no NextAuth proxy, no shared DB                       | Must work when main app is down                             |
| **Extractable services**        | `notify` crate (Slack + SES) lives in repo as workspace member, can be lifted later | Other Versable projects can git-dep it                      |
| **Slack alerting**              | TODO for V1.5 — error-rate spike alerts via Slack webhook                           | Just stub the integration shape now                         |
| **Pluggable storage**           | `HotStore` + `ColdStore` traits; SQLite + S3 are V1 impls, others swappable later   | Backend change must not require touching routes/dashboard   |
| **Sentry independence**         | Service runs without Sentry; integration is opt-in via `withSentry()` helper        | logger-crab is the source of truth for narrative logs       |
| **Schema shape**                | Typed nested envelope (actor / object / state / system / deploy / source / trace)   | Index dimensions cleanly; payload reserved for free-form    |
| **Event naming**                | `<domain>.<entity>.<action>[.<outcome>]` — max 4 dots, prefix-queryable             | Stops module sprawl; keeps query UI navigable               |
| **Severity model**              | OTel-style `severity_number` (1–24, four per level) + `severity_text` for display   | Express WARN+/WARN++ without inventing levels; OTel-compat  |
| **Batch envelope**              | OTel Resource/Scope/Record split — `resource` + `scope` sent once per batch         | 60–80% bandwidth saving; protocol-level, hard to retrofit   |
| **Sampling reserve**            | Reserve top-level `sample_rate: 1` field even though V1 never samples               | One byte today; no migration when sampling lands            |
| **Backpressure policy**         | Drop-newest with `dropped_count` stamped on the next successful event               | Bounded shipper memory; no silent loss                      |
| **Message templates**           | V1.5 — Seq/Serilog `@mt` + `@i` template-hash for event-type aggregation            | Biggest dev-ergonomics win in the survey; pairs with V1     |
| **Breadcrumb mode**             | V1.5 — opt-in ring buffer per `request_id`, flushed only on `error`/`fatal`         | Drops 95% debug noise; perfect for 10k/day budget           |

---

## 2. Architecture at a glance

```
┌────────────────────────────────────────────────────────────────┐
│ EMITTERS (existing Versable services)                          │
│   Next.js (Vercel) │ FastAPI (Render) │ Credit Worker │ Crons  │
│                                                                │
│   Each imports the matching shipper:                           │
│     @versable/logger-crab     (Node + Browser)                 │
│     logger_crab (PyPI / git)  (Python)                         │
│   Shippers auto-grab request_id from runtime context           │
└────────────────────────────────────────────────────────────────┘
                            │
                            │  POST /ingest  (batched, fire-and-forget)
                            │  Authorization: Bearer $INGEST_TOKEN
                            ▼
┌────────────────────────────────────────────────────────────────┐
│ logger-crab  (Render Starter, $7/mo, env=shared-infra)         │
│                                                                │
│   axum routes:  POST /ingest  │ GET /logs │ GET /dashboard     │
│   tokio tasks:  hourly cold rotation │ daily hot-tier purge    │
│   notify crate: Slack webhook (V1.5: error-rate alerts)        │
│                                                                │
│   ┌─────────────────────────────────────────────────────────┐  │
│   │ trait HotStore  / trait ColdStore  (Section 4.5)        │  │
│   │ Routes + dashboard depend on traits, not impls          │  │
│   └─────────────────────────────────────────────────────────┘  │
└────────────────────────────────────────────────────────────────┘
                │                              │
                ▼ hot (24–48h)                 ▼ cold (forever)
     ┌────────────────────┐          ┌──────────────────────┐
     │ HotStore impl      │          │ ColdStore impl       │
     │ V1: SQLite + FTS5  │          │ V1: S3 NDJSON gzip   │
     │ V2: Mongo, Postgres│          │ V2: GCS, Azure Blob  │
     │ Picked via env var │          │ Picked via env var   │
     └────────────────────┘          └──────────────────────┘
```

### Correlation backbone

```
 UI middleware  ──X-Request-ID──▶  FastAPI middleware  ──payload.request_id──▶  Redis
        │                                  │                                      │
        ▼                                  ▼                                      ▼
   AsyncLocalStorage              ContextVar                              Worker reads from
   (auto-bound to                 (auto-bound to                          payload, calls
   every log call)                every log call)                         withRequestId(...)
        │                                  │                                      │
        └──────────── all three set Sentry tag: scope.set_tag("request_id", id) ──┘
```

**Key property:** request_id is set ONCE at each runtime edge. Every `log.info(...)` deeper in the stack auto-grabs it. No threading by hand, no parameter drilling.

---

## 3. Workspace + filesystem layout

Cargo workspace with two crates plus shipper subprojects.

```
logger-crab/
├── Cargo.toml                    # workspace manifest
├── Cargo.lock
├── README.md                     # banner + diagram + usage + improvements
├── render.yaml                   # Render blueprint (single source of deploy truth)
├── .env.example                  # all env vars documented
├── .gitignore
├── rust-toolchain.toml           # pin to stable
│
├── crates/
│   ├── log-server/               # the actual service binary
│   │   ├── Cargo.toml
│   │   └── src/
│   │       ├── main.rs           # tokio + axum bootstrap
│   │       ├── config.rs         # env-driven AppConfig + trait wiring
│   │       ├── auth.rs           # bearer-token tower middleware
│   │       ├── error.rs          # thiserror types + IntoResponse
│   │       ├── models.rs         # LogEvent envelope (Section 4.1) + QueryParams
│   │       │
│   │       ├── store/            # pluggable storage layer (Section 4.5)
│   │       │   ├── mod.rs        # HotStore + ColdStore traits + shared types
│   │       │   ├── memory.rs     # in-process impl for tests + dev harness
│   │       │   ├── sqlite/       # V1 hot impl
│   │       │   │   ├── mod.rs    # impl HotStore for SqliteStore
│   │       │   │   ├── migrations/
│   │       │   │   │   └── 20260416_init.sql
│   │       │   │   └── query.rs  # QueryParams → SQL builder
│   │       │   └── s3/           # V1 cold impl
│   │       │       ├── mod.rs    # impl ColdStore for S3Store
│   │       │       └── ndjson.rs # NDJSON gzip writer/reader
│   │       │
│   │       ├── routes/
│   │       │   ├── mod.rs
│   │       │   ├── ingest.rs     # POST /ingest → HotStore::ingest
│   │       │   ├── query.rs      # GET /logs → HotStore::query (+ ColdStore for cold reads)
│   │       │   └── dashboard.rs  # GET /dashboard (maud templates)
│   │       │
│   │       ├── dashboard/        # maud partials (split from routes for clarity)
│   │       │   ├── live_tail.rs
│   │       │   ├── request_inspector.rs    # group/filter/sort/search per request_id
│   │       │   ├── query_builder.rs
│   │       │   └── taxonomy.rs   # collapsible event-name tree
│   │       │
│   │       └── rotation.rs       # hourly tokio task: HotStore.drain → ColdStore.write
│   │
│   └── notify/                   # extractable: Slack + SES (later)
│       ├── Cargo.toml            # exposes pub fn slack_post(), ses_send()
│       └── src/
│           ├── lib.rs            # public API
│           ├── slack.rs          # webhook poster (used by log-server alerts)
│           └── ses.rs            # AWS SES email (V1.5+, stub for now)
│
├── shippers/                     # client libraries for emitters
│   ├── typescript/               # @versable/logger-crab (npm or git)
│   │   ├── package.json
│   │   ├── tsconfig.json
│   │   ├── README.md             # per-runtime integration recipes
│   │   └── src/
│   │       ├── index.ts          # public log API
│   │       ├── shipper.ts        # batched fetch+POST
│   │       ├── context.ts        # AsyncLocalStorage (server) + sessionStorage (client)
│   │       ├── capture/          # auto-population of envelope slots
│   │       │   ├── source.ts     # stack-trace capture for source.{file,line,fn}
│   │       │   ├── system.ts     # navigator/window → system slot (browser only)
│   │       │   └── deploy.ts     # VERCEL_GIT_* / RENDER_GIT_* → deploy slot
│   │       └── types.ts          # LogEvent, LogLevel, Actor, Object, etc
│   │
│   └── python/                   # logger_crab (PyPI or git)
│       ├── pyproject.toml
│       ├── README.md
│       └── logger_crab/
│           ├── __init__.py       # public log API + REQUEST_ID contextvar
│           ├── shipper.py        # loguru sink + httpx batch poster
│           ├── context.py        # contextvar helpers + middleware factory
│           └── capture.py        # inspect.currentframe() → source slot
│
├── tests/                        # see Phase 7.5
│   ├── scenarios/                # end-to-end simulators per real flow
│   │   ├── pipeline_preview_happy_path.ts
│   │   ├── pipeline_preview_failure.ts
│   │   ├── auth_login_flow.ts
│   │   ├── cron_tick.py
│   │   ├── multi_service_burst.ts
│   │   └── README.md
│   └── seed/
│       └── seed.ts               # idempotent dummy-data seeder for deployed app
│
└── docs/
    ├── INTEGRATIONS.md           # per-emitter setup recipes
    ├── DEPLOY.md                 # Render setup walkthrough
    ├── SCHEMA.md                 # event schema reference (Section 4.1)
    ├── EVENT_TAXONOMY.md         # approved (domain, entity) pairs (Section 4.4)
    ├── STORAGE.md                # writing a new HotStore / ColdStore impl
    ├── SHIPPERS.md               # shipper API design — drafted in Phase 7.6
    └── DASHBOARD.md              # query examples + screenshots
```

### Why a workspace

- **`log-server`** can depend on **`notify`** internally (for V1.5 Slack alerts on error spikes)
- When you later want to use Slack notifications from a different Versable project, add `notify = { git = "https://github.com/versable/logger-crab", subpath = "crates/notify" }` to its `Cargo.toml` — no need to extract or republish
- Or fully extract: `git subtree split crates/notify` into its own repo when ready

---

## 4. Data model

### 4.1 Event schema (wire format)

The schema is a **typed nested envelope**: the top level holds the spine
(identity + correlation), and well-known context groups (`actor`, `object`,
`state`, `system`, `deploy`, `source`, `trace`) carry pre-shaped data the
service knows how to index and render. Anything that doesn't fit goes in
`payload`. This stops `payload` from growing into a junk drawer and gives
the dashboard typed cards instead of raw JSON dumps.

```ts
type LogEvent = {
  // Spine — required, indexed, the primary identity of the event
  request_id: string; // UUIDv4, propagated across hops
  event: string; // dotted name, see Section 4.4
  severity_number: number; // OTel 1–24 (4 per level: TRACE 1–4, DEBUG 5–8, INFO 9–12, WARN 13–16, ERROR 17–20, FATAL 21–24)
  severity_text: "debug" | "info" | "warn" | "error" | "fatal"; // display string; derived from severity_number on ingest if absent
  ts: string; // ISO8601 UTC, ms precision
  message?: string; // optional one-line human summary
  sample_rate?: number; // default 1; reserved for future dynamic sampling — multiply at query time
  dropped_count?: number; // shipper stamps onto the next event after a drop-newest backpressure event

  // service / env / deploy / scope are normally inherited from the BATCH ENVELOPE
  // (see Section 5.1). Repeating them per-event is allowed but discouraged.
  service?: string; // "next-web" | "fastapi" | "credit-worker" | ...
  env?: "dev" | "staging" | "prod";

  // Message templates (V1.5) — Seq/Serilog-style
  template?: {
    raw: string; // "user {user_id} did {action}"
    params: Record<string, unknown>; // { user_id: "u_1", action: "login" }
    id?: string; // server-derived hash of `raw`; lets you chart "events of this kind"
  };

  // Who acted — auth context
  actor?: {
    type: "user" | "guest" | "system" | "cron";
    user?: { id: string; name?: string; email?: string };
    team?: { id: string; name?: string };
    impersonator?: { user_id: string; email?: string }; // admin-as-user sessions
  };

  // What was acted on — primary entity + cross-cutting object IDs
  object?: {
    primary?: { type: string; id: string }; // the "main" entity for this event
    job_id?: string;
    task_id?: string;
    run_id?: string;
    template_id?: string;
    file_id?: string;
    upload_id?: string;
    extra?: Record<string, string>; // any other ID-shaped reference
  };

  // Process state at the time of the event
  state?: {
    ui?: {
      url?: string;
      pathname?: string;
      route?: string; // matched Next.js route pattern
      query?: Record<string, string>;
      referrer?: string;
      viewport?: { w: number; h: number };
      scroll?: { x: number; y: number };
      extra?: Record<string, unknown>; // free-form UI state slice
    };
    backend?: {
      worker_id?: string;
      queue?: string;
      pid?: number;
      hostname?: string;
      attempt?: number; // retry attempt count
      extra?: Record<string, unknown>;
    };
  };

  // Client-only environmental info (auto-captured by browser shipper)
  system?: {
    browser?: { name: string; version?: string };
    os?: { name: string; version?: string };
    user_agent?: string;
    timezone?: string; // IANA, e.g. "America/Los_Angeles"
    locale?: string; // BCP 47, e.g. "en-US"
    screen?: { w: number; h: number; dpr?: number };
    network?: { type?: string; downlink?: number }; // navigator.connection
    geo?: { country?: string; region?: string }; // from CDN headers, never IP
  };

  // Deploy context — who built this code, which version of it
  deploy?: {
    commit?: string; // git SHA (set via VERCEL_GIT_COMMIT_SHA / RENDER_GIT_COMMIT)
    branch?: string;
    version?: string; // package.json version or git describe
    build_id?: string; // Vercel build id, Render deploy id
    region?: string; // Vercel/Render region
  };

  // Where in code this fired (auto-captured from stack at log time)
  source?: {
    file?: string; // basename only, e.g. "credit_dispatch.py"
    function?: string;
    line?: number;
    column?: number;
    repo_path?: string; // path relative to repo root, e.g. "backend/lib/redis/credit_dispatch.py"
    repo?: string; // "frontend" | "backend" | "credit-worker" | "logger-crab"
  };

  // Distributed-trace info (Sentry is one consumer, not a dependency)
  trace?: {
    parent_request_id?: string; // for sub-requests/forks
    span_id?: string; // OTel-style span (V2)
    parent_span_id?: string;
    sentry_event_id?: string; // optional cross-link, never required
  };

  // Free-form bag for everything that doesn't fit a typed slot
  payload?: Record<string, unknown>;
};
```

Discipline rules:

- **The typed slots are the contract.** If `actor.user.id` exists, put it
  there — don't repeat it in `payload`. The dashboard renders typed cards
  off the slots; payload is a fallback JSON viewer.
- **`event` is a dotted name** — see Section 4.4. Human prose goes in `message`.
- **`payload` stays free-form** — V1 logs liberally, learn what to promote
  to typed slots after two weeks of real queries.
- **`source` is auto-captured** by the shipper (not the caller) using
  `Error.captureStackTrace` (JS) or `inspect.currentframe()` (Python).
  Caller can override or disable per-call.
- **No PII fields are required.** `actor.user.email` and `system.geo` are
  optional; emitters should pass them only when useful, and a V1.5 ingest
  redactor will be able to drop them.

### 4.1.1 Indexed columns (server-side, derived at ingest)

The server flattens a curated set of nested fields into top-level columns
for fast query without forcing every emitter to repeat them at the top
level:

| Column        | Source path               | Why indexed                    |
| ------------- | ------------------------- | ------------------------------ |
| `request_id`  | top                       | primary correlation            |
| `service`     | top                       | per-service filtering          |
| `env`         | top                       | env scoping                    |
| `level`       | top                       | error spike alerts             |
| `event`       | top                       | prefix queries (Section 4.4)   |
| `ts_epoch_ms` | derived from `ts`         | range scans + ordering         |
| `user_id`     | `actor.user.id`           | "all logs for this user"       |
| `team_id`     | `actor.team.id`           | tenant-shape queries           |
| `job_id`      | `object.job_id`           | pipeline correlation           |
| `run_id`      | `object.run_id`           | per-run drilldown              |
| `task_id`     | `object.task_id`          | per-task inspection            |
| `template_id` | `object.template_id`      | per-template error rates       |
| `commit`      | `deploy.commit`           | "regression after this deploy" |
| `worker_id`   | `state.backend.worker_id` | bad-worker isolation           |
| `repo`        | `source.repo`             | repo-scoped triage             |

### 4.1.2 Planned upgrade — session_id / client_id (V1.5)

**Current gap:** `request_id` is per-distributed-call. To link a user's
activity across multiple requests (same visit) or across visits (same user,
different days), we need a wider identity key. `actor.user.id` already
handles the lifetime scope; we need a mid-scope key for "one browser visit."

**Proposed addition — additive, no breaking changes:**

```ts
actor?: {
  type: "user" | "guest" | "system" | "cron";
  user?: { id: string; name?: string; email?: string };
  team?: { id: string; name?: string };
  impersonator?: { user_id: string; email?: string };

  // V1.5 additions
  session?: {
    id: string;          // "s_<nanoid>", per browser visit, ~24h inactivity TTL
    started_at?: string; // ISO8601, when the session was opened
  };
  client?: {
    id: string;          // "c_<nanoid>", persistent device/browser cookie (survives logout)
  };
};
```

**New indexed columns** (added via `ALTER TABLE events ADD COLUMN …` migration):

| Column       | Source path        | Why indexed                      |
| ------------ | ------------------ | -------------------------------- |
| `session_id` | `actor.session.id` | "this visit" scope queries       |
| `client_id`  | `actor.client.id`  | anonymous cross-session tracking |

**Shipper propagation rules** (the non-trivial part):

| Hop                   | session_id source                                                           |
| --------------------- | --------------------------------------------------------------------------- |
| Browser → Next.js     | Shipper-managed `sid` cookie; created on first event, extended on each emit |
| Next.js → FastAPI     | Shipper forwards as `X-Session-ID` header                                   |
| FastAPI → Redis queue | Dispatcher includes `session_id` in the job payload                         |
| Redis → Worker        | Worker shipper reads from payload into its scope                            |
| Cron / server scripts | `null` (no user context)                                                    |

`user_id` follows the same path but sourced from auth (NextAuth session,
FastAPI auth dependency). `client_id` is browser-only and lives in a
long-lived cookie independent of login state.

### Forward-compat audit — why V1 is safe to defer this

The V1 architecture has been checked for upgrade blockers. All clear:

- **Typed nested envelope** — `actor.session` slots in alongside
  `actor.user` / `actor.team` without changing any existing slot or
  requiring emitter changes.
- **Ingest validation** is shape-based, not field-list-based — new
  optional fields are accepted by pre-upgrade servers (they'll just
  ignore them until the indexer is updated).
- **SQLite flattening** happens at ingest from a curated path list
  (Section 4.1.1). Adding `actor.session.id → session_id` is a
  one-line indexer change plus an additive `ALTER TABLE` migration.
  No existing row rewrite.
- **Shipper public API** — `withRequestContext({ user, team, ... })` in
  Section 6.2/6.3 already accepts arbitrary scope keys. Adding `session`
  and `client` is non-breaking for existing callsites.
- **Redis handoff** — Section 7.5's FastAPI dispatcher wraps payloads in
  a `{ request_id, ...job_payload }` shape. Adding `session_id` /
  `user_id` to that wrapper is additive; worker deserialization tolerates
  unknown fields.
- **Dashboard** — maud partials are composable. The Request Inspector
  can gain a "Session" and "User" filter chip without disturbing the
  existing request-id drill-down.

**The one thing V1 must NOT do** to preserve this path:

- Do NOT use `request_id` as a user-scoped identifier (e.g., setting
  `request_id = session_id` for page views). Keep `request_id` strictly
  per-distributed-call. Shippers must generate a fresh `request_id` for
  every top-level entry point.

This is enforced by the Phase 1 implementation of the request-id
backbone — any temptation to "cheat" a session-scoped id into
`request_id` must be rejected at review time.

### 4.2 SQLite schema (sqlx migration)

```sql
-- crates/log-server/migrations/20260416_init.sql
-- [claude@2026-04-16] Hot tier. Rows rotated to S3 hourly, deleted after 48h.

CREATE TABLE events (
  id              INTEGER PRIMARY KEY AUTOINCREMENT,

  -- Spine
  request_id      TEXT NOT NULL,
  service         TEXT NOT NULL,
  env             TEXT NOT NULL,
  event           TEXT NOT NULL,
  level           TEXT NOT NULL,
  ts              TEXT NOT NULL,
  ts_epoch_ms     INTEGER NOT NULL,
  message         TEXT,

  -- Indexed columns derived from nested envelope at ingest
  user_id         TEXT,
  team_id         TEXT,
  job_id          TEXT,
  run_id          TEXT,
  task_id         TEXT,
  template_id     TEXT,
  commit_sha      TEXT,
  worker_id       TEXT,
  repo            TEXT,

  -- Full nested envelope, kept as JSON1 columns for selective querying
  -- and to render typed dashboard cards without re-parsing payload
  actor_json      TEXT,
  object_json     TEXT,
  state_json      TEXT,
  system_json     TEXT,
  deploy_json     TEXT,
  source_json     TEXT,
  trace_json      TEXT,
  payload         TEXT NOT NULL DEFAULT '{}'
);

-- Spine + correlation
CREATE INDEX idx_events_request_id  ON events(request_id);
CREATE INDEX idx_events_ts          ON events(ts_epoch_ms DESC);
CREATE INDEX idx_events_env_svc_ts  ON events(env, service, ts_epoch_ms DESC);
CREATE INDEX idx_events_level_ts    ON events(level, ts_epoch_ms DESC) WHERE level IN ('error','fatal');

-- Indexed dimensions (partial — only when present, per Section 4.1.1)
CREATE INDEX idx_events_user_id     ON events(user_id)     WHERE user_id     IS NOT NULL;
CREATE INDEX idx_events_team_id     ON events(team_id)     WHERE team_id     IS NOT NULL;
CREATE INDEX idx_events_job_id      ON events(job_id)      WHERE job_id      IS NOT NULL;
CREATE INDEX idx_events_run_id      ON events(run_id)      WHERE run_id      IS NOT NULL;
CREATE INDEX idx_events_task_id     ON events(task_id)     WHERE task_id     IS NOT NULL;
CREATE INDEX idx_events_template_id ON events(template_id) WHERE template_id IS NOT NULL;
CREATE INDEX idx_events_commit_sha  ON events(commit_sha)  WHERE commit_sha  IS NOT NULL;
CREATE INDEX idx_events_worker_id   ON events(worker_id)   WHERE worker_id   IS NOT NULL;
CREATE INDEX idx_events_repo        ON events(repo)        WHERE repo        IS NOT NULL;

-- FTS over event name, message, and payload — three things humans search
CREATE VIRTUAL TABLE events_fts USING fts5(
  event, message, payload,
  content='events', content_rowid='id'
);

CREATE TRIGGER events_ai AFTER INSERT ON events BEGIN
  INSERT INTO events_fts(rowid, event, message, payload)
  VALUES (new.id, new.event, COALESCE(new.message, ''), new.payload);
END;
CREATE TRIGGER events_ad AFTER DELETE ON events BEGIN
  INSERT INTO events_fts(events_fts, rowid, event, message, payload)
  VALUES ('delete', old.id, old.event, COALESCE(old.message, ''), old.payload);
END;

CREATE TABLE rotation_log (
  id           INTEGER PRIMARY KEY AUTOINCREMENT,
  started_at   TEXT NOT NULL,
  finished_at  TEXT,
  rows_shipped INTEGER,
  s3_keys      TEXT,
  error        TEXT
);
```

PRAGMAs set on connection open in `db.rs`:

```sql
PRAGMA journal_mode = WAL;
PRAGMA synchronous = NORMAL;
PRAGMA busy_timeout = 5000;
PRAGMA foreign_keys = OFF;
PRAGMA mmap_size = 268435456;
```

### 4.3 S3 cold-tier layout

```
s3://versable-logs/
├── prod/
│   ├── next-web/2026/04/16/00.ndjson.gz
│   ├── fastapi/2026/04/16/00.ndjson.gz
│   └── credit-worker/2026/04/16/00.ndjson.gz
├── staging/
│   └── ...
└── dev/
    └── ...
```

Lifecycle policy on the bucket:

- `dev/*` → delete after 7 days
- `staging/*` → delete after 30 days
- `prod/*` → transition to S3 Glacier Instant after 30 days, delete after 365

### 4.4 Event naming convention

Event names are the **primary navigation axis** in the dashboard, so the
shape matters as much as the contents. Loose conventions = unbrowsable
dropdown of 800 distinct event names within a quarter.

**Format:** `<domain>.<entity>.<action>[.<outcome>]`

| Part      | Meaning                                | Examples                                |
| --------- | -------------------------------------- | --------------------------------------- |
| `domain`  | Top-level area of the product          | `auth`, `pipeline`, `payment`, `upload` |
| `entity`  | Specific thing inside the domain       | `login`, `preview`, `charge`, `file`    |
| `action`  | Verb describing what happened          | `requested`, `submitted`, `started`     |
| `outcome` | Optional: terminal state of the action | `succeeded`, `failed`, `timed_out`      |

**Examples (good):**

```
auth.login.attempted
auth.login.failed
pipeline.preview.requested
pipeline.preview.completed
pipeline.task.dispatched
pipeline.task.failed.timeout
payment.charge.succeeded
upload.file.parsed
```

**Counter-examples (bad):**

```
pipeline.preview.task.subtask.image-gen.openai.dalle3.timeout
  → 8 levels, 5 noun phrases. Use:
    event:   pipeline.task.failed.timeout
    payload: { subsystem: "image-gen", provider: "openai", model: "dalle3" }

LoginAttempt
  → CamelCase, no domain. Use: auth.login.attempted

submit_form
  → no domain, snake_case at name level. Use: ui.form.submitted
```

**Rules:**

1. Max **4 dots** (5 parts). Hard limit enforced by the shipper validator.
   If you need more depth, that's a sign the detail belongs in `payload`.
2. **Lowercase + snake_case** within each part.
3. **Past tense for outcomes** (`completed`, `failed`, `timed_out`) so
   filtering by `.failed` finds all failure events across domains.
4. **Don't encode IDs in the name** (`pipeline.run_abc123.failed`) — use
   `object.run_id` instead. Otherwise the dashboard's event-name dropdown
   explodes.
5. **Domain is owned**. The repo will keep `docs/EVENT_TAXONOMY.md` listing
   approved (domain, entity) pairs, with new ones requiring a one-line PR.

**Why this helps querying:**

- Prefix queries collapse hierarchies: `event_prefix=auth` → all auth
  events; `event_prefix=auth.login` → all login flow events;
  `event_prefix=*.failed` (suffix variant via FTS) → all failures.
- The dashboard renders a **collapsible tree** keyed on dots — click
  `pipeline` → expand to see `preview`, `task`, `run` — click `preview`
  → see `requested`, `completed`, `failed.timeout`. No overwhelming flat
  list.
- The `level + outcome` cross-product gives you alerting handles:
  "alert on `level=error` AND `event LIKE '%.failed%'` for the last 5
  min" without needing per-event configuration.

### 4.5 Pluggable storage interface

V1 ships SQLite + S3, but the **service must not depend on either
concretely**. Every store goes behind a trait so we can swap MongoDB for
the hot tier or GCS for the cold tier without touching routes, dashboard,
or shippers.

**Rust traits (`crates/log-server/src/store/mod.rs`):**

```rust
use async_trait::async_trait;
use futures::Stream;
use chrono::{DateTime, Utc};

#[async_trait]
pub trait HotStore: Send + Sync {
    /// Insert a batch of events. Returns per-event accept/reject result.
    async fn ingest(&self, events: &[LogEvent]) -> Result<IngestSummary>;

    /// Query events with filters + pagination. Backed by indexes the
    /// implementation chose; the QueryParams contract is store-agnostic.
    async fn query(&self, params: &QueryParams) -> Result<QueryPage>;

    /// Drain (read + delete) events older than `before`. Used by the
    /// rotation job. Implementations may stream to keep memory bounded.
    async fn drain_older_than(
        &self,
        before: DateTime<Utc>,
    ) -> Result<Box<dyn Stream<Item = LogEvent> + Send + Unpin>>;

    /// Health check — must be cheap.
    async fn health(&self) -> Result<HotHealth>;
}

#[async_trait]
pub trait ColdStore: Send + Sync {
    /// Append a batch as one immutable object, scoped by env+service+hour.
    /// Returns the object key/path so it can be logged in `rotation_log`.
    async fn write_batch(
        &self,
        env: &str,
        service: &str,
        hour: DateTime<Utc>,
        events: &[LogEvent],
    ) -> Result<String>;

    /// Stream events back from the cold tier matching QueryParams.
    /// Implementations may push down filters or stream-and-filter.
    async fn read_range(
        &self,
        params: &QueryParams,
    ) -> Result<Box<dyn Stream<Item = LogEvent> + Send + Unpin>>;

    /// Health check — must not actually round-trip if expensive.
    async fn health(&self) -> Result<ColdHealth>;
}
```

**V1 implementations (in `crates/log-server/src/store/`):**

```
store/
├── mod.rs                # traits + shared types (QueryParams, LogEvent)
├── sqlite/
│   ├── mod.rs            # impl HotStore for SqliteStore
│   ├── migrations.rs     # sqlx-managed schema
│   └── query.rs          # QueryParams → SQL builder
├── s3/
│   ├── mod.rs            # impl ColdStore for S3Store
│   └── ndjson.rs         # NDJSON gzip writer/reader
└── memory.rs             # in-process impl, used by tests + dev harness
```

**Future implementations (V2+, sketched only):**

```
store/
├── mongo/                # impl HotStore for MongoStore (TTL indexes for purge)
├── postgres/             # impl HotStore for PostgresStore (when SQLite outgrown)
├── gcs/                  # impl ColdStore for GcsStore (NDJSON gzip on GCS)
└── azure_blob/           # impl ColdStore for AzureBlobStore
```

**Wiring (`config.rs`):**

```rust
let hot: Arc<dyn HotStore> = match cfg.hot_store.as_str() {
    "sqlite" => Arc::new(SqliteStore::new(&cfg.sqlite_path).await?),
    "memory" => Arc::new(MemoryStore::new()),
    other => bail!("unknown HOT_STORE={other}"),
};
let cold: Arc<dyn ColdStore> = match cfg.cold_store.as_str() {
    "s3"     => Arc::new(S3Store::from_env(&cfg.s3).await?),
    "memory" => Arc::new(MemoryStore::new()),
    "none"   => Arc::new(NullColdStore),
    other => bail!("unknown COLD_STORE={other}"),
};
```

**Env vars:**

```
HOT_STORE=sqlite       # sqlite | mongo | postgres | memory
COLD_STORE=s3          # s3 | gcs | azure_blob | memory | none
```

**Constraints on impls:**

- `query()` must respect the same `QueryParams` shape across stores.
  Implementations choose whether to translate to SQL, MongoDB query, or
  DynamoDB GSI lookups, but the wire contract is identical.
- Filters that an impl can't push down should be applied in-process on
  the stream rather than rejected.
- `health()` is called by `/health` and the rotation job — must complete
  in <100ms or return `HotHealth::Degraded { reason }`.

---

## 5. API contract

### 5.1 `POST /ingest`

**Auth:** `Authorization: Bearer $INGEST_TOKEN`

**Body — OTel-style Resource/Scope/Record split.** Stable per-source fields ride the
batch envelope so they aren't repeated N times per batch (60–80% bandwidth saving):

```json
{
  "resource": {
    "service": "next-web",
    "env": "prod",
    "deploy": { "commit": "abc123", "branch": "main", "region": "iad1" },
    "system": { "hostname": "next-iad-3" }      // optional, server-only resource bits
  },
  "scope": { "name": "@versable/logger-crab", "version": "0.1.0" },
  "events": [
    { "request_id": "...", "event": "...", "severity_number": 9, "ts": "...", ... }
  ]
}
```

The server MERGES `resource` + `scope` into each persisted event before indexing.
Per-event override wins: if an event sets its own `service`, that overrides the
batch resource. Emitters MAY repeat fields per-event for backward compat; the
server is tolerant either way.

**Response:** `202 Accepted`

```json
{
  "accepted": 998,
  "rejected": [{ "index": 12, "reason": "missing event name" }]
}
```

Per-event validation; bad events go to a rejected log (written back into SQLite
as `service=log-server, event=ingest.rejected`) but never fail the whole batch.

Backpressure protocol: when the shipper drops events due to a full buffer, the
NEXT successful event in the batch carries `dropped_count: N`. The server
indexes this and the dashboard surfaces "N events dropped before this one."

Rate limits (soft V1):

- 1000 events / request
- 100 requests / sec / source IP

### 5.2 `GET /logs`

**Auth:** `Authorization: Bearer $DASHBOARD_TOKEN`

| Param                                 | Type                      | Notes                                            |
| ------------------------------------- | ------------------------- | ------------------------------------------------ |
| `request_id`                          | string                    | exact match                                      |
| `service`                             | string (repeatable)       | `?service=fastapi&service=next-web`              |
| `env`                                 | string                    | default `prod`                                   |
| `level`                               | string                    | minimum level (e.g. `error` returns error+fatal) |
| `since`                               | ISO8601                   | default: 1h ago                                  |
| `until`                               | ISO8601                   | default: now                                     |
| `event_prefix`                        | string                    | e.g. `pipeline.preview.`                         |
| `user_id` `team_id` `job_id` `run_id` | string                    | indexed                                          |
| `q`                                   | string                    | FTS5 MATCH over `event` + `payload`              |
| `limit`                               | int                       | default 200, max 2000                            |
| `cursor`                              | int                       | opaque                                           |
| `source`                              | `hot` \| `cold` \| `both` | default `hot`; cold streams from S3              |

### 5.3 `GET /dashboard`

**Auth:** Basic auth using `DASHBOARD_TOKEN` (browser prompts).

Server-rendered maud templates. htmx for partial refresh and filter
re-renders. Dark mode default with light toggle (per global rule).

#### 5.3.1 Panels

1. **Live tail** — last 50 events across the chosen env, htmx auto-refresh
   every 3s. Per-row badges for service, level, and event domain.
   Click any row → opens Request Inspector for that `request_id`.
   Backed by `GET /tail` SSE endpoint (`text/event-stream`) using a
   `tokio::sync::broadcast` channel from the ingest path; htmx
   `hx-ext="sse"` consumes the stream so updates are push, not poll.
   Filter chips (service, level, event_prefix) translate to SSE query
   params and cause a reconnect with the new filter.

2. **Request Inspector** (the headline feature) — paste a `request_id` or
   click in from the live tail / query results. Shows **every event in
   that request, in order, across every service it touched**:

   - Default: chronological list, color-coded by `service`, with elapsed-time
     deltas between events.
   - **Group by**: service, event domain (first dotted part), event prefix
     (configurable depth), level. Switching groups re-renders without
     losing the `request_id` filter.
   - **Filter within request**: by level (>= warn / >= error), by service
     (multi-select), by event prefix, by time slice within the request.
   - **Sort**: ts asc (default), ts desc, by service+ts, by level severity.
   - **Search within request**: FTS across event names, messages, payloads.
   - **Typed cards**: each event renders a small structured card with the
     `actor`, `object`, and `state` slots. Payload shows as collapsed JSON.
   - **Span-style timeline view** (toggle): horizontal bars per service
     showing when each one was active during the request lifecycle.
   - **Sentry/trace links** (only if present): `trace.sentry_event_id`
     deep-links to Sentry; never required to be set.

3. **Query builder** — form mapping 1:1 to `GET /logs` parameters. Results
   table supports the same group/filter/sort controls as the Request
   Inspector. "Download as NDJSON" button for offline analysis. Saved
   queries (V1.5+) keyed by URL hash for shareable links.

4. **Event taxonomy browser** — collapsible tree of distinct event names
   discovered from the hot tier (Section 4.4). Lets you click into any
   prefix to see its sub-events and run a query on it. Acts as living
   documentation of what the system actually emits.

#### 5.3.2 Sentry independence

The dashboard **never requires Sentry** to be configured. Sentry-specific
fields (`trace.sentry_event_id`) render as opt-in "Open in Sentry" links
only when present. All correlation, error inspection, stack trace display,
and span timelines are computed from the event stream itself — not from
Sentry's API.

This means logger-crab is a **complete log inspection tool standalone**.
Sentry stays useful for what it's good at (error grouping, alerting,
release tracking) without being a hard dependency of debugging cross-service
flows.

---

## 6. Request-ID auto-context — the minimal-boilerplate design

The whole point: **callsites are one line**. No request_id passing, no parameter drilling.

### 6.1 How it works per runtime

| Runtime                                          | Context primitive                              | Where it's set                                                        | Where it's read                      |
| ------------------------------------------------ | ---------------------------------------------- | --------------------------------------------------------------------- | ------------------------------------ |
| Next.js server (RSC, API routes, server actions) | `AsyncLocalStorage`                            | `middleware.ts` once per request                                      | inside `log.*()` calls automatically |
| Next.js edge                                     | `AsyncLocalStorage` (edge runtime supports it) | edge middleware                                                       | same                                 |
| Next.js client                                   | module-level + `sessionStorage`                | first import / first interaction                                      | same                                 |
| Credit worker (Node)                             | `AsyncLocalStorage`                            | wrap each `BRPOP` callback in `withRequestId(payload.request_id, fn)` | same                                 |
| FastAPI                                          | `ContextVar[str]`                              | middleware once per request                                           | same                                 |
| Python cron / job worker                         | `ContextVar[str]`                              | set once per tick at function entry                                   | same                                 |
| Pipeline runner / one-shots                      | `ContextVar[str]`                              | set at script entry                                                   | same                                 |

**You import `log` and call it. The shipper handles request_id, service name, env, ts.**

### 6.2 Public API surface (TypeScript shipper)

```ts
// shippers/typescript/src/index.ts
import type { LogEventInput, LogLevel } from "./types";
import { Shipper } from "./shipper";
import { ensureRequestId, getRequestId, withRequestId } from "./context";

const shipper = new Shipper({
  endpoint: process.env.LOG_SERVICE_URL!,
  token: process.env.LOG_SERVICE_TOKEN!,
  service: detectService(), // reads NEXT_RUNTIME / typeof window / explicit override
  env: (process.env.APP_ENV ?? process.env.NODE_ENV ?? "dev") as Env,
});

function emit(
  level: LogLevel,
  event: string,
  payload: Record<string, unknown> = {},
) {
  shipper.enqueue({
    level,
    event,
    payload,
    request_id: getRequestId() ?? ensureRequestId(),
    // [claude@2026-04-16] Auto-hoist known tag names from payload for indexing
    user_id: payload.user_id as string | undefined,
    team_id: payload.team_id as string | undefined,
    job_id: payload.job_id as string | undefined,
    run_id: payload.run_id as string | undefined,
    task_id: payload.task_id as string | undefined,
  });
}

export const log = {
  debug: (event: string, payload?: Record<string, unknown>) =>
    emit("debug", event, payload),
  info: (event: string, payload?: Record<string, unknown>) =>
    emit("info", event, payload),
  warn: (event: string, payload?: Record<string, unknown>) =>
    emit("warn", event, payload),
  error: (event: string, payload?: Record<string, unknown>, err?: unknown) =>
    emit("error", event, { ...payload, error: serializeError(err) }),
  fatal: (event: string, payload?: Record<string, unknown>, err?: unknown) =>
    emit("fatal", event, { ...payload, error: serializeError(err) }),
};

export { withRequestId, getRequestId, ensureRequestId };
```

### 6.3 Public API surface (Python shipper)

```python
# shippers/python/logger_crab/__init__.py
from contextvars import ContextVar
from contextlib import contextmanager
import uuid
from .shipper import Shipper

REQUEST_ID: ContextVar[str] = ContextVar("request_id", default="")

_shipper = Shipper(
    endpoint=os.environ["LOG_SERVICE_URL"],
    token=os.environ["LOG_SERVICE_TOKEN"],
    service=os.environ.get("LOG_SERVICE_NAME", "fastapi"),
    env=os.environ.get("APP_ENV", "dev"),
)

@contextmanager
def request_id_scope(rid: str | None = None):
    rid = rid or str(uuid.uuid4())
    token = REQUEST_ID.set(rid)
    try:
        yield rid
    finally:
        REQUEST_ID.reset(token)

def _emit(level, event, payload=None, error=None):
    payload = dict(payload or {})
    if error is not None:
        payload["error"] = {"type": type(error).__name__, "message": str(error)}
    _shipper.enqueue({
        "level": level, "event": event, "payload": payload,
        "request_id": REQUEST_ID.get() or str(uuid.uuid4()),
        "user_id": payload.get("user_id"),
        "team_id": payload.get("team_id"),
        "job_id":  payload.get("job_id"),
        "run_id":  payload.get("run_id"),
        "task_id": payload.get("task_id"),
    })

class log:
    @staticmethod
    def debug(event, **kw): _emit("debug", event, kw)
    @staticmethod
    def info (event, **kw): _emit("info",  event, kw)
    @staticmethod
    def warn (event, **kw): _emit("warn",  event, kw)
    @staticmethod
    def error(event, error=None, **kw): _emit("error", event, kw, error)
    @staticmethod
    def fatal(event, error=None, **kw): _emit("fatal", event, kw, error)
```

---

## 7. Integration stubs — copy-paste recipes per emitter

### 7.1 Next.js middleware (Vercel + serverless + edge)

```ts
// frontend/src/middleware.ts
import { NextResponse } from "next/server";
import { withRequestId } from "@versable/logger-crab";
import crypto from "node:crypto";

export function middleware(req: Request) {
  const rid = req.headers.get("x-request-id") ?? crypto.randomUUID();
  return withRequestId(rid, () => {
    const res = NextResponse.next();
    res.headers.set("x-request-id", rid);
    return res;
  });
}

export const config = {
  matcher: "/((?!_next/static|_next/image|favicon.ico).*)",
};
```

Usage anywhere downstream (RSC, API route, server action):

```ts
import { log } from "@versable/logger-crab";

log.info("auth.login.attempt", { email });
log.error("payment.charge.failed", { user_id, amount }, err);
```

### 7.2 Next.js client (browser)

```ts
// frontend/src/utils/logger/client-init.ts (imported once at app root)
import { ensureRequestId } from "@versable/logger-crab";
ensureRequestId(); // pulls from sessionStorage or generates one
```

Then in any client component:

```ts
import { log } from "@versable/logger-crab";

function onSubmit() {
  log.info("ui.form.submitted", { form: "preview-pipeline" });
}
```

### 7.3 FastAPI middleware

```python
# backend/api/middleware/request_id.py
from fastapi import Request
from logger_crab import REQUEST_ID, log
import uuid

async def request_id_middleware(request: Request, call_next):
    rid = request.headers.get("x-request-id") or str(uuid.uuid4())
    token = REQUEST_ID.set(rid)
    try:
        response = await call_next(request)
        response.headers["x-request-id"] = rid
        return response
    finally:
        REQUEST_ID.reset(token)

# Wire in api/api.py:
# app.middleware("http")(request_id_middleware)
```

Usage anywhere:

```python
from logger_crab import log

log.info("pipeline.preview.requested", job_id=job_id, parts=len(parts))
try:
    do_work()
except Exception as e:
    log.error("pipeline.preview.failed", error=e, job_id=job_id)
    raise
```

### 7.4 Credit worker (Node)

```ts
// frontend/credit-worker/run.ts (modification, not replacement)
import { withRequestId, log } from "@versable/logger-crab";
import crypto from "node:crypto";

while (true) {
  const popped = await redis.brpop(QUEUE_NAME, 5);
  if (!popped) continue;
  const payload = JSON.parse(popped[1]);

  await withRequestId(payload.request_id ?? crypto.randomUUID(), async () => {
    log.info("worker.event.received", {
      event_id: payload.id,
      job_id: payload.job_id,
    });
    try {
      await processEvent(payload);
      log.info("worker.event.completed", { event_id: payload.id });
    } catch (err) {
      log.error("worker.event.failed", { event_id: payload.id }, err);
      throw err;
    }
  });
}
```

### 7.5 Backend Redis dispatcher (FastAPI side)

```python
# backend/lib/redis/credit_dispatch.py — modification
from logger_crab import REQUEST_ID, log

def enqueue_job_credit(job_id, run_id, task_id, ...):
    event_id = str(uuid.uuid4())
    event = {
        "id": event_id,
        "request_id": REQUEST_ID.get() or str(uuid.uuid4()),  # propagate
        "job_id": job_id, "run_id": run_id, "task_id": task_id,
        # ... rest unchanged ...
    }
    redis.lpush(REDIS_CREDIT_QUEUE, json.dumps(event))
    log.info("credit.event.enqueued", event_id=event_id, job_id=job_id)
```

### 7.6 Cron jobs (Python)

```python
# backend/cron/notify_jobs.py — modification
from logger_crab import request_id_scope, log
import uuid

async def notify_tick():
    with request_id_scope() as rid:  # one rid per tick
        log.info("cron.notify.tick.started")
        try:
            count = await scan_and_notify()
            log.info("cron.notify.tick.completed", jobs_notified=count)
        except Exception as e:
            log.error("cron.notify.tick.failed", error=e)
            raise
```

### 7.7 Pipeline runner / job worker (Python)

```python
# backend/pipeline_runner.py — modification
from logger_crab import request_id_scope, log

def run_pipeline(job_id, run_id):
    # Inherit request_id from MongoDB doc if present (set by API at submit time)
    rid = mongo.runs.find_one({"_id": run_id}).get("request_id")
    with request_id_scope(rid):
        log.info("pipeline.run.started", job_id=job_id, run_id=run_id)
        # ... existing loguru calls migrate to log.info(...) ...
```

### 7.8 Sentry integration (both sides)

Add to both `sentry.server.config.ts` and `pipeline_runner.py`'s sentry init:

```ts
// frontend/sentry.server.config.ts
import { getRequestId } from "@versable/logger-crab";

Sentry.init({
  // ... existing config ...
  beforeSend(event) {
    const rid = getRequestId();
    if (rid) event.tags = { ...event.tags, request_id: rid };
    return event;
  },
});
```

```python
# backend/pipeline_runner.py
import sentry_sdk
from logger_crab import REQUEST_ID

sentry_sdk.init(
    # ... existing ...
    before_send=lambda event, hint: {**event, "tags": {**event.get("tags", {}), "request_id": REQUEST_ID.get()}}
        if REQUEST_ID.get() else event,
)
```

Now any Sentry event has a `request_id` tag → click through to the log dashboard's request inspector → see every event for that flow.

---

## 8. Shipper internals (Rust-side context)

Shippers are the same design across languages: in-memory queue, batch flush every 2s or at 500 events, fire-and-forget POST.

Hard rules:

1. **Never throws / never blocks.** Every failure is silent in prod, `console.warn` in dev.
2. **Never depends on the log service being up.** If POST fails, drop the batch (V1) or buffer to disk (V2).
3. **Detects its own runtime** and stamps `service` accordingly.
4. **Auto-grabs request_id** from context — never asks the caller.
5. **Auto-hoists indexed tags** (`user_id`, `job_id`, etc) from payload.

---

## 9. Rotation job (Rust, in-process)

One tokio task fires hourly via `tokio::time::interval`:

```rust
// crates/log-server/src/rotation.rs
pub async fn rotation_loop(state: AppState) {
    let mut tick = tokio::time::interval(Duration::from_secs(3600));
    loop {
        tick.tick().await;
        if let Err(e) = rotate_once(&state).await {
            tracing::error!("rotation failed: {e:?}");
            // [claude@2026-04-16] V1.5: notify::slack_post(...) on persistent failure
        }
    }
}

async fn rotate_once(state: &AppState) -> Result<()> {
    let cutoff = Utc::now() - Duration::hours(1);
    let groups = state.db.fetch_grouped_for_rotation(cutoff).await?;
    for ((env, service, hour_bucket), rows) in groups {
        let key = format!("{env}/{service}/{hour_bucket}.ndjson.gz");
        let body = gzip_ndjson(rows)?;
        state.s3.put(&key, body).await?;
    }
    state.db.purge_older_than(Duration::hours(48)).await?;
    Ok(())
}
```

---

## 10. Slack alerting (V1.5 todo, stubbed in V1)

The `notify` crate gets a `slack` module that exposes:

```rust
// crates/notify/src/slack.rs
pub async fn post(webhook_url: &str, text: &str) -> Result<()> { /* ... */ }

pub struct AlertThrottle { /* in-memory rate limiter */ }
impl AlertThrottle {
    pub fn should_alert(&mut self, key: &str) -> bool { /* dedup within 5min */ }
}
```

V1.5 wiring (deferred — write the trigger logic but feature-flag it off):

```rust
// In ingest.rs after writing events:
if has_error_spike(&state.metrics) && env_flag("ENABLE_ALERTS") {
    notify::slack::post(&state.slack_webhook, "Error spike detected: ...").await.ok();
}
```

For V1: just stub the `notify` crate with `slack_post()` and a `// TODO: wire in V1.5` comment in ingest.rs.

**Future generalization** (per user's note): `notify` crate gets extracted to its own repo or kept as workspace member that other Versable services can git-dep:

```toml
# In some other Versable Rust service:
[dependencies]
notify = { git = "https://github.com/versable/logger-crab", branch = "main" }
```

Same pattern for SES email when added.

---

## 11. Render deployment

### 11.1 Codebase changes

Create `render.yaml` in repo root:

```yaml
# logger-crab/render.yaml
services:
  - type: web
    name: logger-crab
    env: rust
    plan: starter
    region: oregon # match other Versable services
    buildCommand: cargo build --release --bin log-server
    startCommand: ./target/release/log-server
    envVars:
      - key: DATABASE_URL
        value: sqlite:///var/data/logs.db
      - key: INGEST_TOKEN
        sync: false # set in Render dashboard
      - key: DASHBOARD_TOKEN
        sync: false
      - key: AWS_ACCESS_KEY_ID
        sync: false
      - key: AWS_SECRET_ACCESS_KEY
        sync: false
      - key: AWS_REGION
        value: us-east-1
      - key: S3_LOGS_BUCKET
        value: versable-logs
      - key: SLACK_WEBHOOK_URL
        sync: false
      - key: ENABLE_ALERTS
        value: "false" # V1: off
      - key: RUST_LOG
        value: info
    disk:
      name: logger-crab-data
      mountPath: /var/data
      sizeGB: 1
    healthCheckPath: /health
    autoDeploy: true
```

Add `/health` route returning `200 OK` with `{"ok": true, "uptime_s": N}`.

### 11.2 Manual setup steps (one-time)

For the user to do in Render dashboard / AWS console:

1. **Create Render Environment**

   - Render dashboard → Environments → "+ New Environment" → name: `shared-infra`
   - This decouples the log service from app dev/prod environments

2. **Create the service from blueprint**

   - In `shared-infra` env: "+ New" → Blueprint → connect `versable/logger-crab` repo
   - Render auto-reads `render.yaml`

3. **Create the persistent disk**

   - Already declared in `render.yaml` as `logger-crab-data`, 1 GB
   - Render auto-provisions on first deploy

4. **Create S3 bucket** (AWS console)

   - Bucket name: `versable-logs`
   - Region: us-east-1
   - Block all public access: ON
   - Versioning: OFF (logs are append-only, versioning adds cost)
   - Lifecycle rules:
     - `dev/*`: expire after 7 days
     - `staging/*`: expire after 30 days
     - `prod/*`: transition to Glacier Instant after 30 days, expire after 365

5. **Create IAM user for log service** (AWS console)

   - User: `versable-logger-crab`
   - Inline policy: `s3:PutObject`, `s3:ListBucket` scoped to `versable-logs/*`
   - Generate access key, paste into Render env vars (`AWS_ACCESS_KEY_ID`, `AWS_SECRET_ACCESS_KEY`)

6. **Generate tokens**

   - `INGEST_TOKEN`: `openssl rand -hex 32`
   - `DASHBOARD_TOKEN`: `openssl rand -hex 32`
   - Paste into Render env vars

7. **Deploy** — push to `main` branch → Render auto-builds + deploys

8. **Verify**
   - `curl https://logger-crab.onrender.com/health` → `{"ok": true}`
   - `curl -X POST https://logger-crab.onrender.com/ingest -H "Authorization: Bearer $INGEST_TOKEN" -d '{"events":[{"request_id":"test","service":"smoke","env":"dev","event":"test.smoke","level":"info","ts":"2026-04-16T12:00:00Z","payload":{"hello":"world"}}]}'`
   - Browser: `https://logger-crab.onrender.com/dashboard` → basic auth prompt with `DASHBOARD_TOKEN`

### 11.3 Configuring emitter projects

In each emitter project's Render/Vercel env vars:

```
LOG_SERVICE_URL=https://logger-crab.onrender.com
LOG_SERVICE_TOKEN=<the INGEST_TOKEN value>
APP_ENV=prod  # or "dev" / "staging" depending on env
```

---

## 12. Task plan

### Phase 0 — Repo scaffold `1 day`

- [ ] Create `github.com/versable/logger-crab`
- [ ] `cargo init --workspace` with `log-server` + `notify` crates
- [ ] Add `render.yaml`, `.env.example`, `README.md` (banner + diagram + usage)
- [ ] Set up `rustfmt.toml`, `clippy.toml`, GitHub Actions for `cargo check + clippy`

### Phase 1 — Request-ID backbone (no log service yet) `~2 days`

Same as before — this delivers ~60% of value with zero infra. See Section 7 for stubs.

- [ ] Next.js middleware
- [ ] FastAPI middleware
- [ ] Credit dispatcher payload + worker pickup
- [ ] Cron tick scoping
- [ ] Sentry tag wiring (both sides)
- [ ] Verify: one Sentry error has matching `request_id` tag across all related events

### Phase 2 — Storage trait + SQLite hot impl `~3 days`

- [ ] `store/mod.rs`: define `HotStore` + `ColdStore` traits, shared
      `LogEvent`, `QueryParams`, `IngestSummary`, `QueryPage` types
- [ ] `store/memory.rs`: in-process impl for tests + dev harness
- [ ] `store/sqlite/`: `HotStore` impl, sqlx pool + PRAGMAs, migrations,
      `QueryParams → SQL` builder
- [ ] `store/s3/`: stub `ColdStore` impl (real impl in Phase 5)
- [ ] `config.rs`: env-var driven trait wiring
      (`HOT_STORE`, `COLD_STORE`)
- [ ] `main.rs`: tokio + axum boot, `/health` route hitting both stores

### Phase 3 — Ingest + Query API `~3 days`

- [ ] `auth.rs`: bearer-token tower middleware (two tokens)
- [ ] `models.rs`: `LogEvent` envelope (Section 4.1) as serde struct,
      validators for spine + event-name format (Section 4.4)
- [ ] `routes/ingest.rs`: `POST /ingest` calling `HotStore::ingest`,
      per-event validation, hoist nested fields → indexed columns
- [ ] `routes/query.rs`: `GET /logs` calling `HotStore::query`, cursor
      pagination, FTS5 query path (`q=` parameter), `source=cold`
      streaming via `ColdStore::read_range`
- [ ] Smoke test: curl 100 events in, query by request_id, by event_prefix,
      by FTS

### Phase 4 — Shippers (V1) `~2 days`

- [ ] `shippers/typescript/`: full implementation + per-runtime detection,
      auto-capture of `source` via `Error.captureStackTrace`,
      auto-population of `system` slot from navigator/window in browser,
      auto-population of `deploy` from `VERCEL_GIT_*` / `RENDER_GIT_*`
- [ ] `shippers/python/`: loguru sink + `ContextVar` helpers, auto-capture
      of `source` via `inspect.currentframe()`
- [ ] **W3C `traceparent` propagation helpers** (both shippers): extract
      incoming `traceparent` header → seed `request_id` + `trace.span_id` + `trace.parent_span_id`; inject outgoing `traceparent` on `fetch` /
      `requests` / `httpx` wrappers. Free standards-compliant correlation.
- [ ] **Batch envelope assembly**: shipper collects N events sharing the
      same `service`/`env`/`deploy` and emits one POST with a single
      `resource` block + N records (Section 5.1). Per-event service
      override only when explicitly set.
- [ ] **Drop-newest backpressure**: bounded buffer (default 1000); on
      overflow, increment a local counter; first successful event after
      a drop carries `dropped_count: N`.
- [ ] **Severity number mapping**: callers pass `info`/`warn`/`error`;
      shipper maps to OTel `severity_number` (9 / 13 / 17 default; allow
      `+1`/`+2` modifier for WARN+ etc.) and sets `severity_text`.
- [ ] Wire into 5-10 high-value callsites in main app to validate the
      contract — but defer the **shipper API design review** to Phase 7.6

### Phase 5 — Rotation + S3 cold impl `~2 days`

- [ ] `store/s3/`: real `ColdStore` impl using `aws-sdk-s3`, NDJSON gzip
      writer, key layout from Section 4.3
- [ ] `rotation.rs`: hourly tokio task using `HotStore::drain_older_than` + `ColdStore::write_batch`, retries, `rotation_log` table writes
- [ ] Manual test: populate SQLite, trigger rotation, verify gzipped
      NDJSON in S3, verify hot-tier rows deleted

### Phase 6 — Render deployment `~1 day`

- [ ] All steps in Section 11.2
- [ ] First production traffic: smoke shipper logs from dev
- [ ] Verify dashboard loads, queries return real data

### Phase 7 — Dashboard `~3 days`

- [ ] `routes/dashboard.rs` with maud templates
- [ ] Live tail panel + htmx auto-refresh
- [ ] Request Inspector with group/filter/sort/search inside one
      `request_id` (Section 5.3.1)
- [ ] Query builder + NDJSON download
- [ ] Event taxonomy browser (collapsible event-name tree)
- [ ] Span-style timeline view toggle
- [ ] Dark/light toggle

### Phase 7.5 — Test scenarios + dummy seed `~2 days`

The deployed app needs runnable proof. Two artifacts:

- [ ] `tests/scenarios/`: end-to-end simulators that mirror real Versable
      flows. Each scenario is a single script that POSTs the full sequence
      of events for a fake request_id, then validates the dashboard +
      query API return them in the expected shape:
  - [ ] `pipeline_preview_happy_path.ts` — UI submit → FastAPI accept →
        Redis enqueue → worker pickup → 5 task events → completion
  - [ ] `pipeline_preview_failure.ts` — same flow, but worker throws
        midway; verify error event has stack + correlated request_id
  - [ ] `auth_login_flow.ts` — UI form → API attempt → success + actor
        slot populated
  - [ ] `cron_tick.ts` — Python-side scenario: `request_id_scope` per tick + N notification events
  - [ ] `multi_service_burst.ts` — 10 parallel request_ids, mixed
        services, mixed levels — used to validate dashboard grouping
- [ ] `tests/seed/seed.ts`: idempotent script that POSTs ~2k representative
      events spanning all event domains, levels, services, recent + older
      timestamps. Run after a fresh deploy to populate the dashboard with
      realistic data for review. `--clear` flag to wipe first.
- [ ] `tests/scenarios/README.md`: how to run each scenario locally and
      against deployed `logger-crab.onrender.com` with `INGEST_TOKEN`.

### Phase 7.6 — Shipper API design review doc `~half day`

Once V1 is deployed and we've used the shippers for 1-2 weeks, the API
surface of `@versable/logger-crab` and `logger_crab` will need a focused
design pass. **Don't lock the API in Phase 4** — let real callsites
inform the shape.

- [ ] `docs/SHIPPERS.md` covering:
  - Public API surface (`log.*` methods, helpers, types)
  - Per-runtime initialization patterns (Next.js server vs edge vs client,
    FastAPI middleware, worker BRPOP wrap, cron scope)
  - `actor` / `object` / `state` population helpers — how should the
    caller hand these in? `log.info("event", { actor, object, payload })`
    or chained `log.with({ actor }).info(...)` builder?
  - Error capture conventions (`log.error("event", err, { extra })` vs
    `log.error("event", { error: err })`)
  - Drop-on-failure vs queue-to-disk semantics
  - Distribution: git-dep vs npm/PyPI publish
- [ ] Review with user before implementing V2 shipper API

### Phase 7.7 — Message templates + event-type rollups (V1.5) `~3 days`

- [ ] Add `template: { raw, params, id }` slot to event schema (already
      reserved in 4.1; this phase wires the server-side hashing + indexing)
- [ ] Server-side `template.id` derivation: SHA1(raw)[0..16] hex, computed
      at ingest if absent; indexed column `template_id` for fast aggregation
- [ ] Shipper helpers: `log.info("user {user_id} did {action}", { user_id, action })`
      auto-splits raw + params; rendered `message` is set as a fallback
- [ ] Materialized rollup table `event_type_rollup_1m`: `(template_id,
service, env, ts_minute, count, p50_ms?, p95_ms?)` refreshed every
      60s via background tokio task
- [ ] Dashboard panel: "Top event templates last 24h" — table with
      template_raw, count, error_rate, sparkline; click → drill into all
      events of that template

### Phase 7.8 — Breadcrumb mode (V1.5) `~2 days`

- [ ] Shipper-side opt-in: `withBreadcrumbs(request_id, () => ...)` opens a
      ring buffer (default 100 entries) keyed by `request_id`
- [ ] `debug`/`info` calls inside the scope are NOT shipped immediately —
      they go to the buffer
- [ ] Any `error`/`fatal` event triggers a flush: the buffered breadcrumbs
      are POSTed alongside the error event, marked `breadcrumb: true`
- [ ] Server-side: breadcrumb events are indexed normally but rendered
      collapsed in the Request Inspector under the triggering error
- [ ] Configurable buffer size + TTL (auto-discard after 5min if no error)

### Phase 7.9 — Exclusion filters at ingest (V1.5) `~1 day`

- [ ] `exclusion_rules.toml` config: list of `{ service, event_prefix,
severity_max, action: "drop" | "cold_only" }` rules
- [ ] Ingest path: matching events are either dropped entirely or written
      directly to S3 cold buffer, skipping SQLite hot tier
- [ ] `dropped_count` aggregation per rule, surfaced on dashboard
- [ ] Hot-reload rules without restart (file watch + atomic swap)

### Phase 7.10 — Session + user correlation keys (V1.5) `~2 days`

Adds the mid-scope identity hierarchy (see Section 4.1.2): `session_id` for
"one browser visit" queries and `client_id` for anonymous cross-session
tracking. `user_id` already exists via `actor.user.id` — this phase wires
its propagation end-to-end alongside the new session key.

- [ ] Schema: add `actor.session` and `actor.client` nested slots to the
      wire format (Section 4.1); keep both optional.
- [ ] SQLite migration: `ALTER TABLE events ADD COLUMN session_id TEXT;
    ALTER TABLE events ADD COLUMN client_id TEXT;` + matching indexes.
- [ ] Ingest indexer: flatten `actor.session.id → session_id` and
      `actor.client.id → client_id` (Section 4.1.1 table update).
- [ ] TS shipper: manage `sid` cookie (create on first event, extend TTL
      on each emit, 24h inactivity expiry) and include in `actor.session`.
      Add persistent `cid` cookie for `actor.client`.
- [ ] TS shipper: forward `X-Session-ID` header on fetches so FastAPI
      inherits the session for downstream emits.
- [ ] Python shipper: read `X-Session-ID` + auth context into a
      `ContextVar`-backed scope; emit on every event in the request.
- [ ] FastAPI Redis dispatcher: include `session_id` + `user_id` in the
      job payload wrapper so workers can inherit.
- [ ] Credit worker shipper: pick both ids off the payload and tag emits.
- [ ] Dashboard: add Session Inspector (`/session/:id`) that lists all
      request_ids in the session with per-request drill-down.
- [ ] Dashboard: add user/session filter chips on the main log view.
- [ ] `docs/identity-hierarchy.md` already written — reference from
      `docs/INTEGRATIONS.md` when this lands.

Blocks on: nothing in V1. Forward-compat audit in Section 4.1.2 confirms
V1 design accommodates this without breaking changes.

### Phase 8 — Instrument the important flows `~1 week, rolling`

- [ ] Auth: login, signup, team creation, invite
- [ ] Pipeline: preview, task enqueue, worker pickup, completion
- [ ] Credit: deduct, refund, ledger, allocation
- [ ] Uploads: S3 start, parse, job creation
- [ ] Startup/seed events
- [ ] Cron tick events

### Phase 9 — Learn & iterate `ongoing`

After 2 weeks:

- [ ] Review most-run dashboard queries → add indexes if slow
- [ ] Drop payloads that turned out useless
- [ ] Add missing payloads
- [ ] Decide if Slack alerts (V1.5) are worth building

### Phase 10 (V1.5) — Slack alerting `~2 days when needed`

- [ ] `notify::slack::post()` implementation
- [ ] `AlertThrottle` for 5-min dedup
- [ ] Wire into ingest.rs: error-rate spike detection
- [ ] Configurable thresholds via env vars

### Phase 11 (V1.5+) — SES email service `when needed`

- [ ] `notify::ses::send()` implementation
- [ ] Document for cross-project reuse

---

## 13. Open questions (slim list)

Most decisions are now locked. These remain:

1. **PII / secrets in payload.** Shippers receive arbitrary `Record<str, unknown>`. V1: document "don't log secrets" + rely on discipline. V1.5+: add a redaction pass on ingest (regex over AWS keys, JWTs, emails).

2. **Ingest endpoint reachability.** Vercel-hosted Next.js will POST to `https://logger-crab.onrender.com` over public internet. At our scale (~10k events/day) the bandwidth is free. Confirm OK or proxy via `/api/logs/ingest` on Next.js (extra hop, keeps log service off public internet).

3. **Shipper distribution.** TS via npm publish or git-dep? Python via PyPI publish or git-dep? V1 simplest: git-dep both (no publishing infra needed). Final call deferred to Phase 7.6 design review.

4. **Repo creation.** Need user to create `github.com/versable/logger-crab` (empty) before Phase 0 scaffold. I can prep the files locally, you create + push.

5. **Shipper API surface.** Deliberately deferred — see Phase 7.6. Real
   callsite usage will tell us whether `log.info("event", { actor, object,
payload })` or a chained `log.with({ actor }).info(...)` builder is
   nicer in practice.

6. **`source` capture cost.** Stack-trace capture on every log call adds
   ~5-50µs per event in JS and ~10µs in Python. Likely fine at our volume,
   but the shipper should expose `disable_source_capture` option for
   hot-path callsites that don't want it.

7. **Storage trait shape — read-cold-while-writing-hot.** When a query
   spans both tiers (`source=both`), the route merges streams. Open
   question: do we sort-merge in the route or push the merge down into
   each store? V1: sort-merge in the route (simpler, scales fine for
   our volume).

---

## 14. Out of scope for V1

- Metrics / counters / gauges (different problem; revisit if Sentry is removed)
- Multi-tenancy (one Versable, one log service)
- Auth beyond two bearer tokens (Google OAuth = Phase 5+)
- Indexing arbitrary payload fields (only hoisted slots indexed; rest is FTS or JSON1 scan)
- Replay / reprocess (cold tier is archival, not replayable)
- Buffered-to-disk shipper retries (V1 drops-newest in memory; on-disk queue = V1.5)
- Hard Sentry dependency — Sentry integration is opt-in only; the dashboard works fully standalone
- ABR-style multi-resolution cold storage (Cloudflare-grade; overkill at 10k/day)
- BubbleUp-style attribute-diff UI (needs much higher volume to be useful)
- Schema-on-read with vacuum job (only matters at 1k+ distinct attribute fields)
- Dynamic per-key sampling (only when 10k/day → 10M/day)
- Multi-tenant `X-Scope-OrgID` partitioning (one Versable, one log service)
- Out-of-process shipper sidecar (in-process async fine at our volume)

**Note on OpenTelemetry:** the V1 schema is OTel-shaped (severity_number,
Resource/Scope/Record split, attributes namespace, traceparent propagation),
so OTel SDK interop is **available**, not deferred. We just don't depend on
the OTel collector or its wire format directly — emitters use our shippers,
and our wire format maps 1:1 to OTel logs whenever we want a bridge.

---

## 15. References

- Existing backend logger: `backend/pipeline_runner.py:40-72`
- Existing Sentry config: `frontend/sentry.server.config.ts:8-17`, `backend/pipeline_runner.py:40-47`
- Redis dispatch: `backend/lib/redis/credit_dispatch.py:40-64`
- Credit worker: `frontend/credit-worker/run.ts:14-189`
- Cron: `backend/cron/notify_jobs.py`
- No existing request-id middleware → Phase 1 is net-new
