# Storage architecture

logger-crab uses a two-tier store: a **hot tier** for recent events that need
fast queries, and a **cold tier** for long-term archival. Each tier is behind
a trait (`HotStore`, `ColdStore` in `crates/log-server/src/store/mod.rs`) so
implementations are pluggable. Selection is env-driven.

```
┌──────────────────────────────────────────────────────────────────────┐
│  /ingest  (axum handler)                                             │
│      │                                                               │
│      ▼                                                               │
│  HotStore::ingest()                                                  │
│      │                                                               │
│      ▼                                                               │
│  ┌──────────────┐         (rotation, future)                         │
│  │ HotStore     │  ───────────────────────────────►  ColdStore       │
│  │ (SQLite)     │   drain_older_than → write_batch     (S3)          │
│  │  /var/data   │                                                    │
│  └──────────────┘                                                    │
│      │                                                               │
│      ▼ /logs, /                                                      │
│  Dashboard + query API (hot tier only — cold-tier query is V2)       │
└──────────────────────────────────────────────────────────────────────┘
```

## Hot tier

| Backend  | Env var      | Use                                              |
| -------- | ------------ | ------------------------------------------------ |
| `sqlite` | `HOT_STORE=sqlite` | Production. SQLite at `DATABASE_URL`. Persistent disk required for durability. |
| `memory` | `HOT_STORE=memory` | Tests, ephemeral demos, free-tier deploys (re-seeds on restart). |

The hot tier holds the most recent events — typically 24–48h depending on
volume. It serves all dashboard queries and the `request_id` filter that's
the headline feature. Schema is in `crates/log-server/migrations/`.

### Disk durability

For production, `DATABASE_URL=sqlite:///var/data/logs.db` and the `/var/data`
mount is a Render persistent disk. Without persistence, every restart wipes
the hot tier — events not yet rotated to cold are gone.

The dashboard footer surfaces this: `db = sqlite::memory:` indicates ephemeral
storage; `db = /var/data/logs.db` indicates disk-backed.

## Cold tier

| Backend | Env var          | Use                                        |
| ------- | ---------------- | ------------------------------------------ |
| `s3`    | `COLD_STORE=s3`  | Production. NDJSON.gz objects in S3.        |
| `noop`  | `COLD_STORE=noop` | Cold tier disabled — hot tier is the only durable store. |

### Object layout (load-bearing)

```
s3://<bucket>/{env}/{service}/{YYYY}/{MM}/{DD}/{HH}.ndjson.gz
```

- `{env}` — `prod` / `staging` / `dev` (whatever the emitter set in its event)
- `{service}` — `versable-app` / `versable-api` / etc.
- `{YYYY}/{MM}/{DD}/{HH}` — UTC timestamp of the bucket's hour boundary.
  Zero-padded.
- File body — gzipped NDJSON, one event per line, no trailing newline.

This layout is referenced by S3 lifecycle policies (e.g. transition to Glacier
after 30 days, expire after 365 days). Don't change without coordinating
bucket-side rules.

### Configuration

| Env var               | Required when COLD_STORE=s3 | Purpose                          |
| --------------------- | --------------------------- | -------------------------------- |
| `S3_LOGS_BUCKET`      | yes                         | Bucket name                      |
| `AWS_REGION`          | yes (defaults to `us-east-1`) | Bucket region                    |
| `AWS_ACCESS_KEY_ID`   | yes                         | IAM user with S3 access          |
| `AWS_SECRET_ACCESS_KEY` | yes                       | idem                             |

### IAM policy

```json
{
  "Version": "2012-10-17",
  "Statement": [
    {
      "Effect": "Allow",
      "Action": [
        "s3:HeadBucket",
        "s3:PutObject",
        "s3:ListBucket",
        "s3:GetObject"
      ],
      "Resource": [
        "arn:aws:s3:::versable-logs",
        "arn:aws:s3:::versable-logs/*"
      ]
    }
  ]
}
```

`HeadBucket` is required for boot-time and periodic reachability probes.
`GetObject` + `ListBucket` are forward-looking for the cold-tier query API.

## Health surfaces

The S3 cold store reports its state through three surfaces, all derived from
the same internal `S3State`:

### 1. `/health` JSON

```json
{
  "ok": true,
  "hot": {
    "ok": true,
    "rows": 42081,
    "oldest_ts": "2026-05-06T12:34:56Z"
  },
  "cold": {
    "ok": true,
    "backend": "s3",
    "bucket": "versable-logs",
    "last_rotation": "2026-05-08T13:00:01Z",
    "events_archived_total": 1382,
    "last_health_check": "2026-05-08T14:01:23Z"
  }
}
```

If S3 is unreachable, `cold.ok = false` and `cold.last_error` is populated:

```json
"cold": {
  "ok": false,
  "backend": "s3",
  "bucket": "versable-logs",
  "last_error": "head_bucket failed: dispatch failure | ConnectorError ...",
  "last_health_check": "2026-05-08T14:02:11Z"
}
```

The top-level `ok` is the AND of `hot.ok && cold.ok`, so a single check
catches either tier going down.

### 2. Dashboard footer

The `cold · s3` column on the dashboard footer renders the same fields with
relative timestamps. Useful for at-a-glance state.

### 3. Boot log

Boot emits one structured line:

```
INFO  S3 cold store reachable bucket=versable-logs region=us-east-1
```

…or on failure:

```
ERROR S3 cold store unreachable at boot — service will run but archives will fail
      until resolved bucket=versable-logs region=us-east-1
      error=head_bucket failed: ...
```

The service does not refuse to boot on S3 failure — the hot tier keeps working.
Operators monitor `/health` to detect cold-tier outages and act.

## Health probe semantics

`S3ColdStore::health()` caches successful probes for 30 seconds (constant
`HEALTH_CACHE_TTL_SECS` in `s3.rs`). Failed probes always re-check. This
keeps `/health` cheap when polled frequently while ensuring outages are
detected within ~30s.

`last_error` is sticky: it persists from a write failure or probe failure
until the next successful operation clears it. This means the dashboard /
`/health` retain an error message that explains *why* `ok=false` even after
S3 has recovered, until a real write succeeds.

## Rotation cron (hot → cold)

A background task migrates events from hot to cold on a fixed cadence. Runs
when `COLD_STORE=s3` and `ROTATION_ENABLED=true` (default true).

### Configuration

| Env var                  | Default | Purpose                                        |
| ------------------------ | ------- | ---------------------------------------------- |
| `ROTATION_ENABLED`       | `true`  | Set to `false` to disable rotation entirely    |
| `ROTATION_INTERVAL_SECS` | `3600`  | Seconds between rotation cycles (default 1h)   |
| `HOT_RETENTION_HOURS`    | `48`    | Events older than this get archived            |
| `ROTATION_BATCH_SIZE`    | `5000`  | Max events read per query during pagination    |

### Algorithm

```
every ROTATION_INTERVAL_SECS:
  cutoff = now - HOT_RETENTION_HOURS

  1. read all events from hot where ts < cutoff (paginated)
  2. group events by (env, service, hour-bucket)
  3. for each group: cold.write_batch(env, service, hour, events)
  4. if ALL groups wrote successfully:
       hot.drain_older_than(cutoff)   ← atomic SELECT+DELETE; stream discarded
     else:
       leave events in hot, retry next cycle
```

The first tick fires after `ROTATION_INTERVAL_SECS / 2` to give ingest time to
settle after boot.

### Cursor pagination race window

Dashboard pagination uses RFC3339 timestamps as cursors (last event's `ts`
on the previous page). When new events arrive between paging clicks, the
cursor anchor doesn't change — but new events with timestamps newer than
the anchor (the common case) won't shift older pages around. The corner case:
events with backdated timestamps (clock skew, replays) inserted into a window
the user has already paged past will appear at unexpected positions. Acceptable
for V1's debug-tool use case; mention if you start backfilling old data.

### Failure semantics

- **Single group failure → whole cycle aborts.** No partial deletes from hot.
  The next tick will retry all groups, including the ones that succeeded
  (which means duplicate writes to S3 — but each hour-bucket key is a single
  object, so re-writing replaces it cleanly).
- **No retry within a cycle.** Failure is logged at ERROR with `env`,
  `service`, `hour`, `count`, and the AWS error message. Operators see this
  in Render's log feed and the dashboard footer's `last error` line.
- **Race window:** between the read (step 1) and the delete (step 4), an
  emitter writing events with backdated timestamps (`ts < cutoff`) could
  bypass archival. With a 48h cutoff this is essentially never, but it's a
  known limitation. Documented for future hardening.

### Observability

After each successful cycle, a structured INFO line is emitted:

```
INFO  rotation cycle complete archived=1842 groups=12 failed_groups=0
```

Plus the cold store's `events_archived_total` increments and `last_rotation`
updates — both visible in `/health` and the dashboard footer.

If rotation is intentionally disabled (`ROTATION_ENABLED=false` or
`COLD_STORE=noop`), boot logs:

```
INFO  rotation task NOT spawned cold_store=noop rotation_enabled=true
```

## Cold-tier query

Implemented. The dashboard auto-routes to cold when the user picks a `since`
filter older than the hot tier's oldest event:

```
if params.since < hot.oldest_ts && cold.backend == "s3" && cold.ok:
    → state.cold.read_range(&params)
else:
    → state.hot.query(&params)
```

When the cold path fires, the dashboard shows a banner:

> **❄ Showing archived events from the cold tier (S3).** Queries are capped
> at 5000 events; narrow `since` / `until` for finer slices.

### How `read_range` works

1. Resolve time window — `since` and `until` from `QueryParams`. If `since`
   is missing, defaults to `until - 30d`. If both are missing, defaults to
   the last 30 days.
2. Build narrowest S3 key prefix from filters via `build_s3_prefix`:
   - `service` + `env` set → `{env}/{service}/`
   - `env` only → `{env}/`
   - neither → bucket-wide LIST (slow; warn)
3. LIST objects under that prefix, paginated via continuation token.
4. For each key whose hour bucket falls in `[since_floor_hour, until_floor_hour]`:
   GET, gunzip, parse NDJSON, apply remaining filters in-memory
   (`request_id`, `user_id`, `session_id`, `min_severity`, `event_prefix`,
   `fts`).
5. Sort newest-first, cap at `COLD_QUERY_MAX_EVENTS` (5000), return as stream.

### Pagination + cursor

`read_range` returns a `QueryPage` (the same shape as `HotStore::query`):

```
QueryPage {
  events: Vec<LogEvent>,         // sorted newest-first
  next_cursor: Option<String>,   // RFC3339 ts of the last event, when page is full
}
```

The dashboard's "older →" button passes the cursor as `params.cursor`. Cold
applies it as "ts < cursor", same semantic as hot. Combined with the dashboard's
`?page=N` URL counter, you get consistent pagination UX across both tiers.

### Hot + cold straddle merge

When the user picks `since` older than `hot.oldest_ts` AND `until` newer than
it (or absent), the dashboard runs two queries — cold for `[since, hot.oldest_ts)`
and hot for `[hot.oldest_ts, until]` — and concatenates them. The boundary is
a point in time and rotation only writes-then-deletes (cold gets the events
before hot drops them), so naturally there's no overlap to dedup.

The merged page returns hot's `next_cursor` (the newer half's continuation
cursor). Cold's pagination starts implicitly when the user picks a `since`
that puts them entirely in cold territory.

### Limits

- **Per-page cap of 5000 events**. The dashboard typically requests 50-500;
  the cap is the absolute ceiling regardless of `params.limit`. For genuine
  bulk export, use `/logs/download.ndjson` with a tighter `since`/`until`.
- **No real-time fan-out**: events still being written to hot won't appear
  in cold queries until the next rotation cycle archives them.
- **No straddle pagination**: clicking "older →" on a straddle page uses
  hot's cursor, which walks back through the hot half. To paginate through
  the cold half, narrow `since`/`until` to the cold-only range first.

## What's not implemented (yet)

- **S3 lifecycle policies**: belong on the S3 bucket itself, configured
  outside this service. Recommended starting point: transition to Standard-IA
  after 30 days, expire after 365.
- **Cross-tier cursor**: a single cursor that walks seamlessly across the
  hot/cold boundary. Today the boundary requires changing `since` to
  continue.

## Testing

`store::s3::tests` covers:

- `key_layout_matches_spec` — `prod/versable-app/2026/05/08/14.ndjson.gz`
- `key_layout_pads_single_digit_components` — single-digit month/day/hour zero-padded
- `ndjson_gz_round_trips` — encode → decompress → parse yields the original events
- `ndjson_gz_empty_batch_ok` — handles zero events without panicking
- `noop_health_reports_noop_backend` — health surface contract for noop store

Real S3 integration tests aren't included — they require AWS credentials and
network. The boot-time `head_bucket` probe + the dashboard footer + the
`/health` JSON together provide live verification post-deploy.
