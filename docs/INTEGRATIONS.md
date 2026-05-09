# Integration recipes — per-emitter setup

Step-by-step for adopting logger-crab from each runtime in the Versable stack.
For the producer-contract reference (what an event looks like, threading rules,
Sentry tagging), see [`ADOPTION.md`](./ADOPTION.md).

## Common prerequisites

1. logger-crab deployed and healthy (`/health` returns `{"ok": true}`)
2. A consumer-named ingest token issued (`INGEST_TOKEN_<NAME>=<tier>:<token>`)
3. Network reachability:
   - Vercel → public URL `https://logger-crab.onrender.com`
   - Render → internal URL `http://logger-crab:10000`

## Next.js (Vercel) — TypeScript

Files:

- `src/utils/logger/sinks/crab-sink.ts` — implementation per spec in the file
- `src/utils/logger/index.ts` — register the sink

Env vars (Vercel project settings):

```
LOGGER_CRAB_URL=https://logger-crab.onrender.com
NEXT_PUBLIC_LOGGER_CRAB_URL=https://logger-crab.onrender.com
LOGGER_CRAB_TOKEN=<full-tier token>                # server-side only
NEXT_PUBLIC_LOGGER_CRAB_TOKEN_PUBLIC=<public token> # client bundle
LOGGER_CRAB_SERVICE=versable-app
LOGGER_CRAB_ENV=prod
LOGGER_CRAB_ENABLED=true
NEXT_PUBLIC_LOGGER_CRAB_ENABLED=true
```

Spec for the sink itself: see the embedded JSDoc in
`enhancement-product/frontend/src/utils/logger/sinks/crab-sink.ts`.

## FastAPI (Render) — Python

Files:

- `lib/logger_crab.py` — thin client (httpx + ContextVar)
- `api/middleware/request_id.py` — mint/propagate request_id

Env vars (Render service env):

```
LOGGER_CRAB_URL=http://logger-crab:10000   # ← internal Render network
LOGGER_CRAB_TOKEN=<full-tier token>
LOGGER_CRAB_SERVICE=versable-api
LOGGER_CRAB_ENV=prod
LOGGER_CRAB_ENABLED=true
```

Internal URL is significantly faster (no public DNS, no TLS handshake). See
[`DEPLOY.md`](./DEPLOY.md) for the full Render-internal URL pattern.

## Worker (Render) — Node or Python

Same env vars as the runtime's stack (TS sink for Node, Python sink for Python).
Workers should read `request_id` from the Redis job payload and pass to
emitter calls — see [`identity-hierarchy.md`](./identity-hierarchy.md) for
how `request_id` / `user_id` / `session_id` relate.

## Cron jobs (Render)

Use the same client lib as the worker, but emit a `cron.<job>.start` and
`cron.<job>.done` (or `.error`) per run so the dashboard shows a clean
start/end pair per cron invocation.

## Reference

- Producer contract: [`ADOPTION.md`](./ADOPTION.md)
- Identity model: [`identity-hierarchy.md`](./identity-hierarchy.md)
- Event naming conventions: [`EVENT_TAXONOMY.md`](./EVENT_TAXONOMY.md)
- Wire format / schema: [`SCHEMA.md`](./SCHEMA.md)
