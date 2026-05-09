# Event schema reference

The wire format for events. Source of truth: `crates/log-server/src/models.rs`
(`LogEvent` struct). For evolution rules (how to add fields safely), see
[`schema-evolution.md`](./schema-evolution.md).

## Event envelope

Each event is one JSON object with these fields:

| Field             | Required | Type                | Notes                                                                       |
| ----------------- | -------- | ------------------- | --------------------------------------------------------------------------- |
| `event`           | yes      | string              | Dotted lowercase, stable forever — `pipeline.start`, `auth.login.fail`      |
| `severity_text`   | no       | string              | `trace` / `debug` / `info` / `warn` / `error` / `fatal` (default `info`)    |
| `severity_number` | no       | int (1-24)          | OTel severity number; defaults to 9 (info)                                  |
| `ts`              | no       | RFC3339 string      | Defaults to ingest time on the server                                       |
| `request_id`      | no       | string              | The correlation ID. Empty allowed but headline filter won't work without it |
| `service`         | yes\*    | string              | Stable identifier of the emitter (`versable-app`)                           |
| `env`             | yes\*    | string              | `prod` / `staging` / `dev`                                                  |
| `message`         | no       | string              | Human-readable one-liner                                                    |
| `payload`         | no       | object              | Free-form JSON. FTS-searched; server stamps `_auth_consumer` here           |
| `user_id`         | no       | string              | Persistent user identity                                                    |
| `session_id`      | no       | string              | Per-browser-visit identity                                                  |
| `client_id`       | no       | string              | Per-device identity                                                         |

\* Required for the dashboard filters to be meaningful, but the server
accepts events with these fields missing — they show as `—` in the UI.

## Server-stamped fields

After ingest, **the server adds**:

| Field                       | Where it lives          | Source                                                                  |
| --------------------------- | ----------------------- | ----------------------------------------------------------------------- |
| `payload._auth_consumer`    | inside `payload`        | The consumer name bound to whichever `INGEST_TOKEN_<NAME>` authed       |

The emitter cannot fake `_auth_consumer` — any value supplied in the request
payload is overwritten by the server with the authenticated consumer's name.

## Severity scale

OTel-style; the dashboard color-codes rows by these:

| number | text  | meaning                                              |
| ------ | ----- | ---------------------------------------------------- |
| 1      | trace | ultra-verbose; follow-the-call-stack debug          |
| 5      | debug | developer-only state info                           |
| 9      | info  | default — business events worth recording           |
| 13     | warn  | degraded state; not yet broken                      |
| 17     | error | user-visible failure                                |
| 21     | fatal | service crash / unrecoverable                       |

## Ingest envelope

`POST /ingest` accepts a batch wrapper:

```json
{
  "resource": { "service": "versable-app", "env": "prod" },
  "scope":    { "name": "logger-crab.ts", "version": "1" },
  "events":   [ /* array of LogEvent shapes above */ ]
}
```

`resource.service` and `resource.env` apply to events in the batch that don't
set their own. Per-event `service`/`env` always wins.

## Storage representation

- **Hot tier (SQLite)** — one row per event. Indexed on `ts`, `request_id`,
  `service`, `event`, `severity_number`. FTS5 virtual table mirrors the row
  for full-text search on `message` + `payload`.
- **Cold tier (S3)** — NDJSON.gz, one event per line, gzipped per hour bucket.
  See [`STORAGE.md`](./STORAGE.md).

## Adding fields

See [`schema-evolution.md`](./schema-evolution.md) — short version: add freely,
never rename, never remove without a deprecation window.
