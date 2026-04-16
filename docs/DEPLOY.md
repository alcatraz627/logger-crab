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
#      INGEST_TOKEN      = $(openssl rand -hex 32)
#      DASHBOARD_TOKEN   = $(openssl rand -hex 32)

# 4. First deploy kicks off automatically. Cold build ≈ 4–6 min;
#    subsequent builds hit Docker layer cache ≈ 90 s.

# 5. Smoke test
curl https://logger-crab.onrender.com/health
# → {"ok": true, "hot_ok": true, "cold_ok": true}
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

| Env var                 | Source                                    |
| ----------------------- | ----------------------------------------- |
| `INGEST_TOKEN`          | `openssl rand -hex 32`                    |
| `DASHBOARD_TOKEN`       | `openssl rand -hex 32`                    |
| `AWS_ACCESS_KEY_ID`     | IAM user with `s3:PutObject` on bucket/\* |
| `AWS_SECRET_ACCESS_KEY` | idem                                      |
| `SLACK_WEBHOOK_URL`     | Slack Incoming Webhook for V1.5 alerts    |

`S3_LOGS_BUCKET=versable-logs` and `AWS_REGION=us-east-1` are preset in
`render.yaml`; only the IAM keys are secrets. `ENABLE_ALERTS=false` by
default — flip to `true` after dialing in Slack thresholds.

Rotate tokens by generating a new value, setting it in Render, then
rolling out emitters (shippers will reconnect on next batch — no
in-flight loss because shipper retries a failed POST).

## Enabling S3 cold tier after first deploy

1. Create the bucket: `aws s3 mb s3://versable-logs --region us-east-1`
2. Attach an IAM user with policy below to a fresh access-key pair.
3. Paste the 4 AWS env vars in Render dashboard.
4. Flip `COLD_STORE=s3` (edit env var in dashboard — no redeploy needed,
   rotation worker picks it up on next tick).

Minimal IAM policy:

```json
{
  "Version": "2012-10-17",
  "Statement": [
    {
      "Effect": "Allow",
      "Action": ["s3:PutObject", "s3:ListBucket", "s3:GetObject"],
      "Resource": ["arn:aws:s3:::versable-logs", "arn:aws:s3:::versable-logs/*"]
    }
  ]
}
```

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
# frontend (.env.production)
LOGGER_CRAB_URL=https://logger-crab.onrender.com
LOGGER_CRAB_TOKEN=<INGEST_TOKEN value>

# backend (Render service env)
LOGGER_CRAB_URL=https://logger-crab.onrender.com
LOGGER_CRAB_TOKEN=<INGEST_TOKEN value>
```

## Cost ceiling

- Web Service Starter: $7/mo
- 1 GB persistent disk: included in Starter
- S3 cold tier (us-east-1): < $0.25/mo at 10k events/day (most of the
  cost is PUT requests, not storage)

Total: ~$7–8/mo. If Render Starter is insufficient, the next rung
(Standard, $25/mo) includes autoscaling and more RAM — don't over-provision
until ingest rate demands it.
