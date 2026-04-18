#!/usr/bin/env python3
"""Seed dummy events into a running log-server via POST /ingest.

Mirrors crates/log-server/src/seed.rs. Every event carries `payload.dummy = true`
so it can be filtered/excluded cleanly in /logs queries.

Usage:
  INGEST_TOKEN=... python3 scripts/seed_prod.py
  INGEST_TOKEN=... LOGGER_BASE_URL=https://logger-crab.onrender.com python3 scripts/seed_prod.py
  INGEST_TOKEN=... LOGGER_BASE_URL=http://127.0.0.1:8099 python3 scripts/seed_prod.py

Env:
  INGEST_TOKEN        required, must match the server's INGEST_TOKEN
  LOGGER_BASE_URL     optional, default https://logger-crab.onrender.com
"""

import json
import os
import sys
import urllib.error
import urllib.request
from datetime import datetime, timedelta, timezone

BASE_URL = os.environ.get("LOGGER_BASE_URL", "https://logger-crab.onrender.com").rstrip("/")
TOKEN = os.environ.get("INGEST_TOKEN", "").strip()


def iso(ts: datetime) -> str:
    return ts.astimezone(timezone.utc).isoformat().replace("+00:00", "Z")


def severity_label(n: int) -> str:
    if 1 <= n <= 4:
        return "trace"
    if 5 <= n <= 8:
        return "debug"
    if 9 <= n <= 12:
        return "info"
    if 13 <= n <= 16:
        return "warn"
    if 17 <= n <= 20:
        return "error"
    if 21 <= n <= 24:
        return "fatal"
    return "info"


def make_event(base: datetime, offset_ms: int, request_id: str, service: str,
               env: str, event: str, sev: int, message: str, payload: dict) -> dict:
    return {
        "request_id": request_id,
        "event": event,
        "severity_number": sev,
        "severity_text": severity_label(sev),
        "ts": iso(base + timedelta(milliseconds=offset_ms)),
        "message": message,
        "service": service,
        "env": env,
        "payload": {"dummy": True, **payload},
    }


def build_events() -> list[dict]:
    base = datetime.now(timezone.utc) - timedelta(minutes=30)
    return [
        make_event(base, 0, "req_alice_01", "versable-app", "prod", "ui.page.view", 9,
                   "User viewed /enhance/123", {"path": "/enhance/123", "user_id": "u_alice"}),
        make_event(base, 150, "req_alice_01", "versable-api", "prod", "http.request", 9,
                   "POST /api/jobs 201", {"method": "POST", "status": 201, "latency_ms": 128}),
        make_event(base, 400, "req_alice_01", "versable-api", "prod", "redis.enqueue", 5,
                   "enqueued job_id=j_9001", {"queue": "credits", "job_id": "j_9001"}),
        make_event(base, 820, "req_alice_01", "credit-worker", "prod", "pipeline.start", 9,
                   "picked up job_id=j_9001", {"job_id": "j_9001", "attempt": 1}),
        make_event(base, 3400, "req_alice_01", "credit-worker", "prod", "pipeline.done", 9,
                   "completed job_id=j_9001 in 3.4s",
                   {"job_id": "j_9001", "duration_ms": 3400, "rows": 42}),

        make_event(base, 5000, "req_bob_02", "versable-app", "prod", "ui.upload.start", 9,
                   "Bob started upload 12MB CSV", {"size_bytes": 12582912, "user_id": "u_bob"}),
        make_event(base, 6200, "req_bob_02", "versable-api", "prod", "http.request", 13,
                   "POST /api/upload 413 payload too large",
                   {"status": 413, "reason": "limit_exceeded"}),
        make_event(base, 6210, "req_bob_02", "versable-app", "prod", "ui.upload.error", 13,
                   "Upload failed: file too large", {"shown_to_user": True}),

        make_event(base, 8000, "req_crn_03", "cron-daily", "prod", "cron.rollup.start", 9,
                   "Starting nightly rollup", {"job": "rollup.daily"}),
        make_event(base, 9500, "req_crn_03", "cron-daily", "prod", "cron.rollup.warn", 13,
                   "Skipping 3 stale rows", {"skipped": 3, "reason": "stale"}),
        make_event(base, 11000, "req_crn_03", "cron-daily", "prod", "cron.rollup.done", 9,
                   "Rollup OK (1847 rows, 2.9s)", {"rows": 1847, "duration_ms": 2912}),

        make_event(base, 14000, "req_err_04", "credit-worker", "prod", "openai.call.error", 17,
                   "OpenAI API 429 rate limit",
                   {"provider": "openai", "status": 429, "retry_after_s": 30, "model": "gpt-5"}),
        make_event(base, 14010, "req_err_04", "credit-worker", "prod", "pipeline.retry", 13,
                   "Retrying after 30s", {"attempt": 2, "backoff_ms": 30000}),

        make_event(base, 16000, "req_dev_05", "versable-api", "dev", "db.query.slow", 13,
                   "Slow query: 2.1s",
                   {"duration_ms": 2108, "sql_fingerprint": "select_jobs_by_user"}),
        make_event(base, 16500, "req_dev_06", "versable-api", "dev", "auth.login.ok", 9,
                   "login successful", {"user_id": "u_dev", "method": "oauth"}),

        make_event(base, 18000, "req_fatal_07", "credit-worker", "prod", "worker.panic", 21,
                   "Worker panicked, exited",
                   {"panic_msg": "index out of bounds: len=0 idx=3", "will_restart": True}),
    ]


def post_ingest(events: list[dict]) -> dict:
    body = json.dumps({
        "resource": {"service": "seed-script", "env": "prod"},
        "scope": {"name": "seed_prod.py", "version": "1"},
        "events": events,
    }).encode("utf-8")

    req = urllib.request.Request(
        f"{BASE_URL}/ingest",
        data=body,
        method="POST",
        headers={
            "Content-Type": "application/json",
            "Authorization": f"Bearer {TOKEN}",
        },
    )
    try:
        with urllib.request.urlopen(req, timeout=30) as resp:
            return json.loads(resp.read().decode("utf-8"))
    except urllib.error.HTTPError as e:
        sys.stderr.write(f"HTTP {e.code}: {e.reason}\n{e.read().decode('utf-8', 'replace')}\n")
        raise


def main() -> int:
    if not TOKEN:
        sys.stderr.write("ERROR: INGEST_TOKEN is unset. Source your .env first:\n")
        sys.stderr.write("  set -a; source .env; set +a; python3 scripts/seed_prod.py\n")
        return 2

    events = build_events()
    print(f"→ POST {BASE_URL}/ingest  ({len(events)} events)")
    result = post_ingest(events)
    accepted = result.get("accepted", 0)
    rejected = result.get("rejected", [])
    print(f"✓ accepted={accepted}  rejected={len(rejected)}")
    if rejected:
        for r in rejected[:5]:
            print(f"  - idx {r.get('index')}: {r.get('reason')}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
