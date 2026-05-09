# Client library API reference (shippers)

Shippers are the thin client libraries each emitter imports to talk to
logger-crab. Two are in active use (TypeScript + Python). Both follow the
same shape so the docs translate 1:1.

## Common shape

A shipper exposes:

- `log(event, opts)` — fire-and-forget; queues + batches
- `flush()` — drain pending events; call on shutdown / `beforeunload`
- Auto-injection of `service` / `env` / `severity_number` from env vars
- Auto-batching (default 25 events / 2s flush window)
- Silent failure mode (telemetry never breaks the app)

## TypeScript — `@versable/logger-crab` (planned npm package)

Used in: Next.js (Vercel), Credit Worker (Node).

### Install

```bash
npm install @versable/logger-crab
```

### Setup

```typescript
import { logger } from "@versable/logger-crab";

logger.configure({
  url: process.env.LOGGER_CRAB_URL,
  token: process.env.LOGGER_CRAB_TOKEN,    // full-tier
  service: "versable-app",
  env: "prod",
});
```

For browser builds, use the public token + URL:

```typescript
logger.configure({
  url: process.env.NEXT_PUBLIC_LOGGER_CRAB_URL,
  token: process.env.NEXT_PUBLIC_LOGGER_CRAB_TOKEN_PUBLIC,
  service: "versable-app",
  env: process.env.NODE_ENV === "production" ? "prod" : "dev",
});
```

### Emit

```typescript
logger.emit({
  event: "pipeline.start",
  severity: "info",
  request_id: req.headers["x-request-id"],
  message: `picked up job ${job.id}`,
  payload: { job_id: job.id, attempt: 1 },
});
```

Or via severity-named helpers:

```typescript
logger.info("pipeline.start", { job_id });
logger.warn("db.query.slow", { duration_ms: 2100 });
logger.error("openai.call.error", { status: 429 });
```

### Shutdown

```typescript
window.addEventListener("beforeunload", () => logger.flush());
process.on("SIGTERM", async () => { await logger.flush(); process.exit(0); });
```

## Python — `logger_crab` (planned PyPI package)

Used in: FastAPI (`versable-api`), cron jobs.

### Install

```bash
pip install logger_crab
```

### Setup

```python
from logger_crab import emit, configure

configure(
    url=os.environ["LOGGER_CRAB_URL"],
    token=os.environ["LOGGER_CRAB_TOKEN"],
    service="versable-api",
    env=os.environ.get("LOGGER_CRAB_ENV", "dev"),
)
```

### Emit

```python
await emit(
    event="openai.call.error",
    severity="error",
    request_id=request.state.request_id,
    message="OpenAI 429",
    payload={"provider": "openai", "status": 429},
)
```

### Sync flush for cron / one-shot scripts

```python
from logger_crab import emit, flush_sync

emit(event="cron.rollup.done", payload={"rows": 1847})
flush_sync()  # required: cron has no event loop to drain
```

## Implementation status

Both shippers are **specified but not yet published**. The TypeScript sink
spec lives at `enhancement-product/frontend/src/utils/logger/sinks/crab-sink.ts`
(JSDoc contract). The Python sink contract is in [`ADOPTION.md`](./ADOPTION.md)
under "FastAPI playbook".

## Failure modes

All shippers swallow exceptions internally. Telemetry must never break the
app. Lost events are accepted as the V1 cost-of-doing-business — durability
guarantees would require disk-backed queues, which is out of scope.

## Configuration env vars (both shippers)

| Env var                  | Required | Notes                                          |
| ------------------------ | -------- | ---------------------------------------------- |
| `LOGGER_CRAB_URL`        | yes      | logger-crab base URL                           |
| `LOGGER_CRAB_TOKEN`      | yes      | bearer token (full or public tier)             |
| `LOGGER_CRAB_SERVICE`    | yes      | service identifier from EVENT_TAXONOMY.md      |
| `LOGGER_CRAB_ENV`        | yes      | prod / staging / dev                           |
| `LOGGER_CRAB_ENABLED`    | no       | `true` to activate (default off in JS variant) |
