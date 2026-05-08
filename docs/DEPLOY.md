# Deploy — Render walkthrough

`logger-crab` deploys as a single Render web service (Starter, $7/mo) with a
1 GB persistent disk for the SQLite hot tier. The S3 cold tier is optional
and can be enabled after the service is live.

## TL;DR

```bash
# 1. Push main (CI must be green — see .github/workflows/ci.yml)
git push origin main

# 2. In Render dashboard: New → Blueprint → select this repo
#    render.yaml is auto-detected.

# 3. Set secrets in the Render dashboard (NOT in render.yaml):
#      INGEST_TOKEN_<CONSUMER> = <tier>:<openssl rand -hex 32>   ← one row per consumer
#      DASHBOARD_TOKEN         = $(openssl rand -hex 32)
#    See "Per-consumer ingest tokens" below for the naming convention.

# 4. First deploy kicks off automatically. Cold build ≈ 4–6 min;
#    subsequent builds hit Docker layer cache ≈ 90 s.

# 5. Smoke test
curl https://logger-crab.onrender.com/health
# → {"ok": true, "hot": {...}, "cold": {"ok": true, "backend": "s3", ...}}
```

## What render.yaml provisions

| Field              | Value                  | Why                                                                                                       |
| ------------------ | ---------------------- | --------------------------------------------------------------------------------------------------------- |
| `runtime: docker`  | `./Dockerfile`         | Multi-stage Rust build. Slim Debian runtime. No `docker-compose`, no shell boot script — just the binary. |
| `plan: starter`    | $7/mo                  | Enough for ~10k events/day. Upgrade to Standard if ingest sustains > 5 rps.                               |
| `disk: 1 GB`       | mounted at `/var/data` | SQLite DB lives at `/var/data/logs.db`. Survives deploys.                                                 |
| `healthCheckPath`  | `/health`              | Render restarts the service if `/health` returns non-2xx for > 30s.                                       |
| `autoDeploy: true` | push-to-main           | Green CI → deploy. No manual trigger.                                                                     |
| `branch: main`     |                        | Match CI trigger. Change if you deploy from a long-lived release branch.                                  |
| `region: oregon`   |                        | Match the Versable app stack. Override if latency matters for ingest.                                     |

## Secrets (set in Render dashboard)

Never commit these. `sync: false` in `render.yaml` means a blueprint
re-apply will **not** overwrite dashboard values.

| Env var                       | Source                                                                       |
| ----------------------------- | ---------------------------------------------------------------------------- |
| `INGEST_TOKEN_<CONSUMER>`     | `<tier>:<openssl rand -hex 32>` — one row per emitter (see below)            |
| `DASHBOARD_TOKEN`             | `openssl rand -hex 32`                                                       |
| `AWS_ACCESS_KEY_ID`           | IAM user with `s3:PutObject` + `s3:HeadBucket` on bucket/\*                  |
| `AWS_SECRET_ACCESS_KEY`       | idem                                                                         |
| `CORS_ORIGINS`                | Comma-separated allowlist of browser origins (omit for "any" — dev only)     |
| `SLACK_WEBHOOK_URL`           | Slack Incoming Webhook for V1.5 alerts                                       |

### Per-consumer ingest tokens

The auth model is **one named token per emitter** rather than a single shared
secret. Each `INGEST_TOKEN_<NAME>=<tier>:<token>` row registers one consumer:

```
INGEST_TOKEN_PROD_APP_SERVER       = full:<openssl rand -hex 32>
INGEST_TOKEN_PROD_APP_BROWSER      = public:<openssl rand -hex 32>
INGEST_TOKEN_STAGING_APP_SERVER    = full:<openssl rand -hex 32>
INGEST_TOKEN_STAGING_APP_BROWSER   = public:<openssl rand -hex 32>
INGEST_TOKEN_DEV_AAKARSH_SERVER    = full:<openssl rand -hex 32>
INGEST_TOKEN_DEV_AAKARSH_BROWSER   = public:<openssl rand -hex 32>
```

- `<NAME>` is `UPPER_SNAKE_CASE`; consumer name in the dashboard is the
  same string lowercased with `_` → `-` (`PROD_APP_SERVER` → `prod-app-server`).
- `<tier>` is exactly `full` or `public`. Full tokens are server-side only;
  public tokens are safe to ship in browser bundles.
- Adding a new emitter = adding one new row. Rotating = editing one row's value.
- The settings modal (gear icon on the dashboard) shows all loaded consumers
  and any malformed entries.

`S3_LOGS_BUCKET=versable-logs` and `AWS_REGION=us-east-1` are preset in
`render.yaml`; only the IAM keys are secrets. `ENABLE_ALERTS=false` by
default — flip to `true` after dialing in Slack thresholds.

Rotate tokens by generating a new value, setting it in Render, then
rolling out emitters (shippers will reconnect on next batch — no
in-flight loss because shipper retries a failed POST).

## Enabling the S3 cold tier

1. Create the bucket: `aws s3 mb s3://versable-logs --region us-east-1`
2. Attach an IAM user with the policy below; generate a fresh access-key pair.
3. Set `AWS_ACCESS_KEY_ID`, `AWS_SECRET_ACCESS_KEY`, `S3_LOGS_BUCKET`,
   `AWS_REGION` in the Render dashboard.
4. Set `COLD_STORE=s3` in the dashboard. Restart picks it up.

On boot, logger-crab calls `head_bucket` against the configured bucket. Result
shows in:

- Boot log: `S3 cold store reachable bucket=versable-logs region=us-east-1`
  (success), or an `error` line with the AWS SDK error message (failure).
- Dashboard footer's "cold" column: backend, bucket, last rotation, last
  probe, and any current error.
- `/health` JSON: `cold.ok`, `cold.bucket`, `cold.last_rotation`,
  `cold.last_error`, `cold.events_archived_total`.

Failure mode is **soft**: if S3 is unreachable, the service continues running
with the hot tier (writes to S3 will fail until the underlying issue is fixed).
The `/health` endpoint reports `ok=false` so external monitors can alert.

Minimal IAM policy:

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

`HeadBucket` is required for the boot-time + periodic reachability probes.

### Object layout

S3 keys follow `{env}/{service}/{YYYY}/{MM}/{DD}/{HH}.ndjson.gz`. Each file is
gzipped NDJSON, one event per line. Layout is load-bearing: **don't change it
casually** — bucket-level lifecycle policies (e.g. transition to Glacier after
30 days) key off the `YYYY/MM/DD/HH` prefix. See [`STORAGE.md`](./STORAGE.md)
for the full reference.

## Disk sizing

Starter includes up to 1 GB. With the default 24h hot retention and
~10k events/day (each ~1.5 KB in SQLite rows + FTS), expected usage is
well under 50 MB. Bump `sizeGB` only if you extend `HOT_RETENTION_HOURS`
past a week.

## Rollback

Render keeps the previous image. In the Render dashboard: Deploys → pick
the last green one → "Rollback". State on disk (SQLite DB) is preserved
across rollbacks, so no data loss.

## Log shipper config after deploy

Point shippers at the service URL:

```bash
# frontend on Vercel (Production env)
LOGGER_CRAB_URL=https://logger-crab.onrender.com
LOGGER_CRAB_TOKEN_FULL=<full-tier token for prod-app-server>
NEXT_PUBLIC_LOGGER_CRAB_URL=https://logger-crab.onrender.com
NEXT_PUBLIC_LOGGER_CRAB_TOKEN_PUBLIC=<public-tier token for prod-app-browser>

# backend on Render (FastAPI / Credit Worker / cron)
LOGGER_CRAB_URL=http://logger-crab:10000   # ← internal Render network, faster
LOGGER_CRAB_TOKEN=<full-tier token for the relevant service>

# Note: Render Web Services bind to port 10000 internally regardless of
# render.yaml. Only Render-hosted services can reach the internal URL;
# Vercel and browsers must use the public HTTPS URL.
```

## Cost ceiling

- Web Service Starter: $7/mo
- 1 GB persistent disk: included in Starter
- S3 cold tier (us-east-1): < $0.25/mo at 10k events/day (most of the
  cost is PUT requests, not storage)

Total: ~$7–8/mo. If Render Starter is insufficient, the next rung
(Standard, $25/mo) includes autoscaling and more RAM — don't over-provision
until ingest rate demands it.
