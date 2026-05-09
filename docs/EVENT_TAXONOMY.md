# Event taxonomy — naming conventions + registered events

How to name events and the canonical list of names already in use.

## Naming rules

- **Lowercase, dotted, stable.** `pipeline.start`, `auth.login.fail`, never
  `pipelineStart` or `Pipeline_Start`.
- **`<subsystem>.<verb>[.<modifier>]`** — first segment is the subsystem
  (HTTP layer, database, cron, payment provider, etc.); second is what
  happened; optional third is the variant.
- **Never rename once shipped.** Dashboards, alerts, and downstream queries
  pin to event names. To change semantics, add a NEW name.
- **Stable across services.** `http.request` means the same thing whether
  emitted by Next.js or FastAPI. Don't service-prefix.

## Registered service names

These are the canonical service identifiers used in `service:` event tags.
Documenting here so the dashboard filters render consistently:

| Service             | Where it runs       | Notes                                |
| ------------------- | ------------------- | ------------------------------------ |
| `versable-app`      | Vercel (Next.js)    | Frontend + server actions + API routes |
| `versable-api`      | Render (FastAPI)    | Backend HTTP API                     |
| `credit-worker`     | Render (Node)       | Redis-driven job worker              |
| `cron-daily`        | Render (cron)       | Daily scheduled jobs                 |
| `cron-hourly`       | Render (cron)       | Hourly scheduled jobs                |
| `logger-crab`       | Render (self)       | Emitted by logger-crab itself when relevant |

## Registered event names

Living list — append as new events ship. Don't rename existing entries.

### HTTP

- `http.request` — one per inbound HTTP request (status in payload)
- `http.error` — request failed with 5xx or unhandled exception
- `http.slow` — duration above the slow threshold (warn-grade)

### UI / browser

- `ui.page.view` — client-side route change
- `ui.upload.start` / `ui.upload.done` / `ui.upload.error`
- `ui.click.<feature>` — user-initiated actions worth tracing

### Pipeline / worker

- `pipeline.start` / `pipeline.done` / `pipeline.error`
- `pipeline.retry` — worker requeueing
- `pipeline.skip` — explicit skip (unsupported input, etc.)

### Cron

- `cron.<job>.start` / `cron.<job>.done` / `cron.<job>.error`
- `cron.<job>.skip` — gate-failed (e.g., upstream not ready)

### Auth

- `auth.login.ok` / `auth.login.fail`
- `auth.logout`
- `auth.session.expired`
- `auth.token.rotated`

### Database

- `db.query.slow` — slow-query logger; `duration_ms` in payload
- `db.connection.error`
- `db.migration.applied`

### External services

- `openai.call.start` / `openai.call.error`
- `redis.enqueue` / `redis.dequeue`
- `s3.upload.error`

### System / lifecycle

- `boot.started` — service startup
- `boot.smoke` — manual smoke test
- `system.config.warning` — surfaced config issue at boot

## Event payload conventions

Common payload keys and what they mean — keep these consistent across services
so dashboard filters can rely on them:

| Key             | Type    | Meaning                                                    |
| --------------- | ------- | ---------------------------------------------------------- |
| `duration_ms`   | int     | Operation duration in ms (drives `db.slow` highlighting)   |
| `status`        | int     | HTTP status code                                           |
| `method`        | string  | HTTP method                                                |
| `path`          | string  | HTTP path or RPC method                                    |
| `error_name`    | string  | Exception class name                                       |
| `error_message` | string  | Exception message (PII-scrubbed)                           |
| `job_id`        | string  | Worker job identifier                                      |
| `attempt`       | int     | Retry attempt number                                       |
| `_auth_consumer`| string  | **Server-stamped** — do NOT set from emitter, will be overwritten |

## Adding a new event

1. Pick a name following `<subsystem>.<verb>[.<modifier>]`
2. Add to the section above with one-line description
3. Emit consistently across services that share the action
4. Don't rename later — add a new event if semantics change
