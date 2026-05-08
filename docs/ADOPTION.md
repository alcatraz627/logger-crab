# Adoption — integrating a service with logger-crab

This doc is for **service owners** adopting logger-crab from the emitter side.
It covers the thin-client libraries (`logger-crab.ts`, `logger_crab.py`),
`X-Request-ID` threading across HTTP / Redis / cron boundaries, and Sentry
scope tagging so stack traces and log lines share a correlation key.

> For _operator_-side docs (hosting, disk, env vars, rotation cron), see
> [`DEPLOY.md`](./DEPLOY.md). For the event schema, see
> [`schema-evolution.md`](./schema-evolution.md).

---

## TL;DR

Adopting logger-crab in a new service is **two files + one env var set**:

1. Drop in the thin client (`logger-crab.ts` for Node/browser, `logger_crab.py`
   for Python).
2. Mint or forward `X-Request-ID` at every service boundary.
3. Set four env vars (`LOGGER_CRAB_URL`, `LOGGER_CRAB_TOKEN`,
   `LOGGER_CRAB_SERVICE`, `LOGGER_CRAB_ENV`).

The client is **fire-and-forget**: events are queued, batched (25 events / 2s),
and flushed over `POST /ingest` with `keepalive: true`. If the log service is
down, events are dropped silently — telemetry never blocks the app.

---

## Architecture recap

```
┌────────────────────────────────────────────────────────────────────┐
│  Emitter Services (each imports a thin client library)             │
│                                                                    │
│   ┌────────────────┐    ┌──────────────────┐   ┌─────────────┐     │
│   │ Next.js (web)  │    │ FastAPI (api)    │   │ Credit      │     │
│   │ logger-crab.ts │    │ logger_crab.py   │   │ Worker (ts) │     │
│   └───────┬────────┘    └──────┬───────────┘   └──────┬──────┘     │
│           │                    │                      │            │
│           └──────────┬─────────┴──────────────────────┘            │
│                     ▼   POST /ingest                               │
│                     │   Authorization: Bearer $LOGGER_CRAB_TOKEN   │
│                     │   X-Request-ID: <same rid through stack>     │
│ ┌───────────────────┴────────────────────────────────────────────┐ │
│ │  logger-crab (axum + sqlite + s3)                              │ │
│ │  /ingest  → SQLite hot  →  S3 NDJSON cold (>48h)               │ │
│ │  /logs    ← dashboard + API                                    │ │
│ └────────────────────────────────────────────────────────────────┘ │
└────────────────────────────────────────────────────────────────────┘
```

---

## Prerequisites

### 1. Request a `LOGGER_CRAB_TOKEN`

Rotate the shared ingest secret or (preferred) issue one secret per service so
you can revoke per-service without disrupting others. The token matches
`INGEST_TOKEN` on the log-server side — see [`DEPLOY.md`](./DEPLOY.md#secrets).

### 2. Set env vars everywhere the service runs

| Variable                      | Example                             | Where       | Public? |
| ----------------------------- | ----------------------------------- | ----------- | ------- |
| `LOGGER_CRAB_URL`             | `https://logger-crab.onrender.com`  | all envs    | yes     |
| `NEXT_PUBLIC_LOGGER_CRAB_URL` | same as above                       | Vercel only | yes     |
| `LOGGER_CRAB_TOKEN`           | `$(openssl rand -hex 32)`           | all envs    | **no**  |
| `LOGGER_CRAB_SERVICE`         | `versable-app` / `versable-api` / … | per-service | yes     |
| `LOGGER_CRAB_ENV`             | `prod` / `staging` / `dev`          | per-env     | yes     |

> **Never put the token behind `NEXT_PUBLIC_*`.** Browsers would then be able
> to impersonate services. If you need browser-side emission, proxy through a
> server route (see [Browser emission](#browser-emission)).

---

## Service playbooks

Each subsection below is self-contained — skip to the runtime you're adopting.

### Next.js (App Router, on Vercel)

**Files:**

- `src/utils/logger-crab.ts` — the client (~100 lines)
- `src/middleware.ts` — mints/forwards `X-Request-ID`

**Client library** (`src/utils/logger-crab.ts`):

```typescript
type Severity = "trace" | "debug" | "info" | "warn" | "error" | "fatal";

const SEV: Record<Severity, number> = {
  trace: 1,
  debug: 5,
  info: 9,
  warn: 13,
  error: 17,
  fatal: 21,
};
const URL = process.env.NEXT_PUBLIC_LOGGER_CRAB_URL ?? "";
const SERVICE = process.env.LOGGER_CRAB_SERVICE ?? "versable-app";
const ENV = process.env.LOGGER_CRAB_ENV ?? process.env.NODE_ENV ?? "dev";

const BATCH = 25;
const FLUSH_MS = 2000;
let queue: unknown[] = [];
let timer: ReturnType<typeof setTimeout> | null = null;

export interface LogEvent {
  event: string;
  severity?: Severity;
  message?: string;
  request_id?: string;
  user_id?: string;
  session_id?: string;
  payload?: Record<string, unknown>;
}

export function log(e: LogEvent, token?: string): void {
  if (!URL) return;
  queue.push({
    event: e.event,
    severity_number: SEV[e.severity ?? "info"],
    severity_text: e.severity ?? "info",
    ts: new Date().toISOString(),
    message: e.message,
    service: SERVICE,
    env: ENV,
    request_id: e.request_id ?? "",
    user_id: e.user_id,
    session_id: e.session_id,
    payload: e.payload ?? {},
  });
  if (queue.length >= BATCH) void flush(token);
  else if (!timer) timer = setTimeout(() => flush(token), FLUSH_MS);
}

export async function flush(token?: string): Promise<void> {
  if (timer) {
    clearTimeout(timer);
    timer = null;
  }
  if (!queue.length || !URL) return;
  const batch = queue.splice(0, queue.length);
  try {
    await fetch(`${URL}/ingest`, {
      method: "POST",
      keepalive: true,
      headers: {
        "Content-Type": "application/json",
        ...(token ? { Authorization: `Bearer ${token}` } : {}),
      },
      body: JSON.stringify({
        resource: { service: SERVICE, env: ENV },
        scope: { name: "logger-crab.ts", version: "1" },
        events: batch,
      }),
    });
  } catch {
    /* telemetry must never break the app */
  }
}

if (typeof window !== "undefined") {
  window.addEventListener("beforeunload", () => {
    void flush();
  });
}
```

**Middleware** (`src/middleware.ts`):

```typescript
import { NextRequest, NextResponse } from "next/server";

export function middleware(req: NextRequest) {
  const rid = req.headers.get("x-request-id") ?? crypto.randomUUID();
  const headers = new Headers(req.headers);
  headers.set("x-request-id", rid);
  const res = NextResponse.next({ request: { headers } });
  res.headers.set("x-request-id", rid);
  return res;
}

export const config = {
  matcher: ["/((?!_next/static|_next/image|favicon.ico|assets).*)"],
};
```

**Usage in route handlers** (server-side — token in env is fine):

```typescript
import { log } from "@/utils/logger-crab";

export async function POST(req: NextRequest) {
  const rid = req.headers.get("x-request-id") ?? "";
  try {
    const result = await doWork();
    log(
      {
        event: "http.request",
        message: `POST /api/jobs 200`,
        request_id: rid,
        payload: { method: "POST", status: 200 },
      },
      process.env.LOGGER_CRAB_TOKEN,
    );
    return NextResponse.json(result);
  } catch (e) {
    log(
      {
        event: "http.error",
        severity: "error",
        message: String(e),
        request_id: rid,
      },
      process.env.LOGGER_CRAB_TOKEN,
    );
    throw e;
  }
}
```

#### Browser emission

Three options, in order of preference:

1. **Server-proxy route** — browser POSTs to `/api/log`, which forwards to
   logger-crab with the server-side token. Recommended.
2. **Unauthenticated ingest** — set `INGEST_TOKEN` to empty on the log-server,
   let browsers hit `/ingest` directly. Dev-only.
3. **Don't emit from the browser** — emit only from route handlers after the
   fetch returns. Works when every user action round-trips through your
   backend anyway.

---

### FastAPI (Python, on Render)

**Files:**

- `lib/logger_crab.py` — the client
- Middleware added to `api/api.py`

**Client library** (`lib/logger_crab.py`):

```python
from __future__ import annotations
import asyncio, logging, os
from datetime import datetime, timezone
from typing import Any

import httpx

SEV = {"trace": 1, "debug": 5, "info": 9, "warn": 13, "error": 17, "fatal": 21}
URL = os.environ.get("LOGGER_CRAB_URL", "").rstrip("/")
TOKEN = os.environ.get("LOGGER_CRAB_TOKEN", "")
SERVICE = os.environ.get("LOGGER_CRAB_SERVICE", "versable-api")
ENV = os.environ.get("LOGGER_CRAB_ENV", "dev")

BATCH = 25
FLUSH_S = 2.0

_q: list[dict[str, Any]] = []
_timer: asyncio.Task | None = None
_lock = asyncio.Lock()
_log = logging.getLogger(__name__)


def _iso() -> str:
    return datetime.now(timezone.utc).isoformat().replace("+00:00", "Z")


def log(event: str, severity: str = "info", message: str | None = None,
        request_id: str = "", user_id: str | None = None,
        session_id: str | None = None,
        payload: dict[str, Any] | None = None) -> None:
    if not URL:
        return
    _q.append({
        "event": event,
        "severity_number": SEV.get(severity, 9),
        "severity_text": severity,
        "ts": _iso(),
        "message": message,
        "service": SERVICE,
        "env": ENV,
        "request_id": request_id,
        "user_id": user_id,
        "session_id": session_id,
        "payload": payload or {},
    })
    if len(_q) >= BATCH:
        asyncio.create_task(_flush())
    else:
        _schedule()


def _schedule() -> None:
    global _timer
    if _timer and not _timer.done():
        return
    try:
        loop = asyncio.get_running_loop()
    except RuntimeError:
        return
    _timer = loop.create_task(_delayed_flush())


async def _delayed_flush() -> None:
    await asyncio.sleep(FLUSH_S)
    await _flush()


async def _flush() -> None:
    async with _lock:
        if not _q:
            return
        batch, _q[:] = _q[:], []
    body = {
        "resource": {"service": SERVICE, "env": ENV},
        "scope": {"name": "logger_crab.py", "version": "1"},
        "events": batch,
    }
    try:
        async with httpx.AsyncClient(timeout=5.0) as c:
            await c.post(f"{URL}/ingest", json=body,
                         headers={"Authorization": f"Bearer {TOKEN}"})
    except Exception as e:
        _log.debug("logger-crab flush failed: %s", e)


def flush_sync() -> None:
    """Sync flush for cron jobs / scripts with no running event loop."""
    if not _q:
        return
    import json, urllib.request
    batch = _q[:]
    _q.clear()
    body = json.dumps({
        "resource": {"service": SERVICE, "env": ENV},
        "scope": {"name": "logger_crab.py", "version": "1"},
        "events": batch,
    }).encode()
    req = urllib.request.Request(
        f"{URL}/ingest", data=body, method="POST",
        headers={"Content-Type": "application/json",
                 "Authorization": f"Bearer {TOKEN}"})
    try:
        urllib.request.urlopen(req, timeout=5).read()
    except Exception as e:
        _log.debug("logger-crab flush_sync failed: %s", e)
```

**Middleware** (add to `api/api.py` right after `app = FastAPI(...)`):

```python
import time, uuid
from starlette.middleware.base import BaseHTTPMiddleware
from lib.logger_crab import log as log_event


class RequestIdAndLogMiddleware(BaseHTTPMiddleware):
    async def dispatch(self, request, call_next):
        rid = request.headers.get("x-request-id") or str(uuid.uuid4())
        request.state.request_id = rid
        start = time.time()
        status = 500
        try:
            response = await call_next(request)
            status = response.status_code
            response.headers["x-request-id"] = rid
            return response
        finally:
            log_event(
                event="http.request",
                severity="info" if status < 400
                         else ("warn" if status < 500 else "error"),
                message=f"{request.method} {request.url.path} {status}",
                request_id=rid,
                payload={
                    "method": request.method,
                    "path": request.url.path,
                    "status": status,
                    "duration_ms": int((time.time() - start) * 1000),
                },
            )


app.add_middleware(RequestIdAndLogMiddleware)
```

Route-level emission is optional — the middleware gives you one event per
request for free. Add `log_event(...)` calls inside handlers only for
domain-level events (job enqueued, user verified, etc.).

---

### Credit Worker (Node, on Render)

The worker has no HTTP surface but enters the request via Redis. It reuses the
same TS client and picks up `request_id` from the job payload.

```typescript
// credit-worker/run.ts
import { log, flush } from "../src/utils/logger-crab";

const TOKEN = process.env.LOGGER_CRAB_TOKEN;
let running = true;

while (running) {
  const result = await redis.blpop(QUEUE_NAME, 0);
  if (!result) continue;
  const [, raw] = result;
  const payload = JSON.parse(raw);
  const rid = payload.request_id ?? ""; // threaded in

  log(
    {
      event: "pipeline.start",
      request_id: rid,
      payload: { job_id: payload.id },
    },
    TOKEN,
  );
  try {
    await runPipeline(payload);
    log(
      {
        event: "pipeline.done",
        request_id: rid,
        payload: { job_id: payload.id },
      },
      TOKEN,
    );
  } catch (e) {
    log(
      {
        event: "pipeline.error",
        severity: "error",
        message: String(e),
        request_id: rid,
        payload: { job_id: payload.id },
      },
      TOKEN,
    );
  }
}

process.on("SIGTERM", async () => {
  running = false;
  await flush(TOKEN); // drain before exit
  process.exit(0);
});
```

> **Why the explicit SIGTERM flush:** the browser client has `beforeunload` but
> Node workers exit abruptly. Without draining, the last in-flight batch is
> lost. Always call `flush()` in shutdown handlers.

---

### Cron jobs & one-shot scripts

For Python cron (Render cron job or local `python script.py`):

```python
# scripts/nightly_rollup.py
import os, uuid
from lib.logger_crab import log, flush_sync

if __name__ == "__main__":
    rid = os.environ.get("CRON_REQUEST_ID") or str(uuid.uuid4())
    log(event="cron.rollup.start", request_id=rid, payload={"job": "rollup.daily"})
    try:
        rows = run_rollup()
        log(event="cron.rollup.done", request_id=rid,
            payload={"rows": rows, "job": "rollup.daily"})
    except Exception as e:
        log(event="cron.rollup.error", severity="error",
            request_id=rid, message=str(e))
        raise
    finally:
        flush_sync()   # MUST call — no event loop to drain timer-based flush
```

For Node one-shots: `await flush(TOKEN)` before `process.exit()` — same reason.

---

## Request-ID threading

The request_id is the single most valuable piece of this system. Once minted
at the edge, it should flow unmodified through every hop:

```
Browser                               ← optional: crypto.randomUUID()
   │
   │  fetch (no x-request-id yet)
   ▼
Next.js middleware                    ← mints crypto.randomUUID if absent
   │
   │  x-request-id: abc-123
   ▼
Next.js route handler                 ← reads req.headers
   │
   │  fetch('https://api', { headers: { 'x-request-id': rid } })
   ▼
FastAPI middleware                    ← reads header, request.state.request_id
   │
   │  payload["request_id"] = rid
   ▼
Redis (JSON payload)                  ← copies into job body
   │
   │  BLPOP → JSON.parse → payload.request_id
   ▼
Credit worker                         ← emits with same rid
```

**At every hop that makes an outbound call, you must forward the header:**

```typescript
// Next.js server-side fetch to FastAPI
const rid = req.headers.get("x-request-id");
await fetch(`${BACKEND_URL}/api/jobs`, {
  headers: { "x-request-id": rid, ... },
  ...
});
```

```python
# FastAPI enqueueing to Redis
job["request_id"] = request.state.request_id
await redis.lpush(QUEUE_NAME, json.dumps(job))
```

Once this is wired end-to-end, the payoff is:

```
https://logger-crab.onrender.com/?request_id=abc-123
```

→ returns every event from `ui.page.view` through `pipeline.done`, in
chronological order, across all three services. That's the feature.

---

## Sentry scope tagging

Tag `request_id` into Sentry scope so stack traces link to the log timeline:

```typescript
// Next.js
import * as Sentry from "@sentry/nextjs";
Sentry.withScope((scope) => {
  scope.setTag("request_id", rid);
  // your handler code
});
```

```python
# FastAPI middleware — after rid is assigned
import sentry_sdk
sentry_sdk.set_tag("request_id", rid)
```

Now: click a Sentry issue → copy `request_id` tag → paste into logger-crab
filter → see the full lead-up to the error.

---

## Rollout order

Adopt in this order — each step produces visible dashboard data on its own,
and earlier steps don't block later ones.

| #   | Step                                | Files touched                     | Risk   |
| --- | ----------------------------------- | --------------------------------- | ------ |
| 1   | Env vars on all hosts               | Vercel, Render (×N)               | none   |
| 2   | Next.js client + middleware         | `logger-crab.ts`, `middleware.ts` | low    |
| 3   | FastAPI client + middleware         | `logger_crab.py`, `api/api.py`    | low    |
| 4   | Redis producer threads `request_id` | enqueue callsites                 | medium |
| 5   | Credit worker emission              | `credit-worker/run.ts`            | low    |
| 6   | Cron jobs + Sentry scope tags       | cron scripts, Sentry configs      | low    |

**After step 2** — you see `http.request` and `ui.page.view` events in the
dashboard.
**After step 3** — you see backend HTTP events with status codes.
**After step 4** — the same `request_id` now threads UI → API → Redis.
**After step 5** — the full lifecycle (UI click → pipeline complete) is
queryable by one `request_id`.

---

## Verification

After each rollout step, confirm from the dashboard:

1. **Visit** `https://logger-crab.onrender.com/`.
2. **Filter** by `service=<your-service>` and `env=<your-env>`.
3. **Expect** new events within ~2 seconds (one batch window).
4. **Copy** a fresh `request_id` from the table and paste into the query box.
5. **Expect** every event from that request across all adopted services.

Smoke test from a terminal:

```bash
curl -sX POST "$LOGGER_CRAB_URL/ingest" \
  -H "Authorization: Bearer $LOGGER_CRAB_TOKEN" \
  -H "Content-Type: application/json" \
  -d '{
    "resource": { "service": "smoke-test", "env": "dev" },
    "events": [{
      "event": "smoke.test",
      "severity_number": 9,
      "severity_text": "info",
      "ts": "'"$(date -u +%Y-%m-%dT%H:%M:%SZ)"'",
      "request_id": "smoke-'"$(date +%s)"'",
      "message": "adoption smoke test"
    }]
  }'
```

Expected: `{"accepted": 1, "rejected": []}`.

---

## Troubleshooting

### "I don't see any data in the dashboard"

- Did `LOGGER_CRAB_URL` get set? Empty URL silently no-ops.
- Did your service actually emit? Add a `log("boot.smoke")` at startup.
- Is the token right? Hit `/ingest` with curl (above) and check for `401`.
- Is the service name spelled the same as your filter? Compare exactly.

### "Events arrive but `request_id` is empty"

- The middleware is only one hop. Check that each outbound `fetch` /
  `httpx.post` / `redis.lpush` **copies** the header/field.
- In Next.js, `req.headers.get("x-request-id")` only works inside route
  handlers and server components — client components don't have access. Use a
  context or cookie to thread it to the browser.

### "Worker logs are missing the last few events"

- Add `await flush(TOKEN)` in the SIGTERM handler.
- For Python cron: use `flush_sync()` at the end of the script.
- The timer-based flush (2s interval) is best-effort; processes that exit
  faster than that will drop the tail batch.

### "Browser emission is blocked by CORS"

- The log-server allows `*` by default, but some hosts strip the header.
- Proxy through a server route instead (see [Browser emission](#browser-emission)).

### "`Authorization` header leaks to the browser"

- Only pass `token` to `log()` in server-side code (route handlers,
  middleware, server actions). Client components should never import the
  token — destructure `process.env.LOGGER_CRAB_TOKEN` only where
  `"use server"` or a route handler guarantees server execution.

---

## Event naming conventions

Keep event names **dotted, lowercase, and stable**. Good names read like
`<subsystem>.<verb>[.<modifier>]`:

| Example                 | When                                 |
| ----------------------- | ------------------------------------ |
| `http.request`          | One per inbound HTTP request         |
| `http.error`            | Request failed with 5xx or exception |
| `ui.page.view`          | Client-side route change             |
| `ui.upload.start`       | User initiated upload                |
| `ui.upload.error`       | Upload failed                        |
| `redis.enqueue`         | Job pushed to queue                  |
| `pipeline.start`        | Worker picked up job                 |
| `pipeline.done`         | Worker finished job successfully     |
| `pipeline.error`        | Worker failed                        |
| `pipeline.retry`        | Worker requeueing                    |
| `cron.<job>.start/done` | Cron boundary                        |
| `auth.login.ok/fail`    | Authentication outcomes              |
| `db.query.slow`         | Slow-query logger                    |
| `openai.call.error`     | External provider failure            |

Once an event name ships, **don't rename it** — existing queries and dashboards
depend on stable names. Add a new name for new semantics.

---

## FAQ

**Q: Do I need to call `flush()` manually?**
No for long-lived processes (web server, worker). Yes for anything that exits
(cron, one-shot script, SIGTERM handler).

**Q: What happens if logger-crab is down?**
Events are dropped silently. The client swallows all exceptions. You will see
nothing in the dashboard but your app keeps running.

**Q: Can I emit very high-volume events (10k/sec)?**
Not with V1. SQLite + single-node ingest caps at a few hundred events/sec
comfortably. For high-volume paths, sample client-side (e.g., log 1 in 100
`http.request` events) or switch to OTLP/Loki/ClickHouse.

**Q: Does the client buffer to disk?**
No. If the process crashes with queued events, they're lost. Good enough for
V1; if durability matters, wrap `log()` with your own disk-backed queue.

**Q: Can I add custom fields?**
Yes — everything goes into `payload` as arbitrary JSON. Top-level fields
(`request_id`, `user_id`, `session_id`, `service`, `env`) are indexed and
filterable in the dashboard; `payload` is searchable via FTS but not indexed.

**Q: How do I test emission locally?**
Run logger-crab locally (`cargo run -p log-server`) with `PORT=8099`,
`HOT_STORE=sqlite`, `DATABASE_URL=sqlite://./dev.db`, then point your
service's `LOGGER_CRAB_URL` at `http://127.0.0.1:8099`.

---

## See also

- [`DEPLOY.md`](./DEPLOY.md) — hosting, disk, env var reference
- [`identity-hierarchy.md`](./identity-hierarchy.md) — how `user_id`, `session_id`, `request_id` relate
- [`schema-evolution.md`](./schema-evolution.md) — adding fields to `LogEvent`
- [`architecture-v1.md`](./architecture-v1.md) — internal design (hot/cold tiers, rotation)
