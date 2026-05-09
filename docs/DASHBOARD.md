# Dashboard guide

The dashboard at `/` is logger-crab's primary debugging surface. This is
how to use it well.

## First visit

Paste `https://logger-crab.onrender.com/?token=<DASHBOARD_TOKEN>` in your
URL bar. Server validates, sets a 30-day cookie, redirects to `/` (so the
secret leaves the URL bar). Subsequent visits are auto-authenticated.

## Filters

| Filter              | What it does                                                           |
| ------------------- | ---------------------------------------------------------------------- |
| `request_id`        | Show every event sharing one correlation ID — the headline feature     |
| `service`           | Restrict to one emitter (`versable-app`, `versable-api`, etc.)         |
| `env`               | Restrict to one env (`prod` / `staging` / `dev`)                       |
| `event prefix`      | `pipeline.` matches `pipeline.start` and `pipeline.error`              |
| `full-text search`  | FTS5 search across `message` + `payload`                               |
| `min level`         | Severity floor (any / trace / debug / info / warn / error / fatal)     |

Filter inputs autocomplete from values present in the hot store (cached 60s).

## Click-to-filter from the table

Every cell with a colored value in a row is a link that sets that as a filter:

- Click a service chip → `?service=<name>`
- Click an env pill → `?env=<value>`
- Click an event namespace (the `pipeline` in `pipeline.start`) → `?event_prefix=pipeline.`
- Click a request_id → `?request_id=<rid>`
- Click a severity (FATAL/ERROR/etc.) → `?level=<severity>`

Stack filters by clicking multiple. Active filters appear as removable chips
above the table; click the `×` to drop one.

## Keyboard shortcuts

Press `?` for the floating cheatsheet. Quick reference:

| Key            | Action                       |
| -------------- | ---------------------------- |
| `/`            | focus the search box         |
| `j` / `↓`      | next row                     |
| `k` / `↑`      | previous row                 |
| `Enter`        | expand current row's payload |
| `r`            | refresh (preserves filters)  |
| `Esc`          | blur input / close modal     |
| `?`            | this cheatsheet              |

## Pagination

Cursor-based — "older →" walks one page-size window back through the result
set. The "page N" indicator in the pager reflects where you are.
"← newest" resets to page 1 (drops cursor + page).

## Cold-tier auto-routing

The dashboard picks the right tier for your time range automatically:

| Filter shape                                          | What runs                              |
| ----------------------------------------------------- | -------------------------------------- |
| No `since` (or `since` >= hot.oldest_ts)              | hot only                               |
| `since` < hot.oldest_ts AND `until` >= hot.oldest_ts  | **straddle merge** — cold + hot, concat |
| `since` < hot.oldest_ts AND `until` < hot.oldest_ts   | cold only                              |
| Cold tier offline (`cold.ok == false`)                | hot only (with stale data noted in footer) |

A blue banner appears whenever cold is involved:

> ❄ Showing archived events from the cold tier (S3). Queries are capped
> at 5000 events; narrow `since` / `until` for finer slices.

### Cold + cold-straddle limits

- **5000-event per-page cap** — narrow the time range or service filter
- **"older →" works on cold and on straddle** (uses cold's cursor for cold-only,
  hot's cursor for straddle — page-back walks through the newer half first)
- **Slower than hot** — each cold query does an S3 LIST + multiple GETs.
  Expect 1-3 seconds for typical queries; ~10s for wide ranges
- **No cross-tier cursor**: paging from straddle into pure-cold territory
  requires manually adjusting `since`/`until` to slot you into cold-only mode

## Settings modal

Gear icon (top-right). Lists every consumer registered via
`INGEST_TOKEN_<NAME>=<tier>:<token>` and any **config warnings** — malformed
env vars, deprecated names, etc. Tokens themselves are never displayed; only
the consumer name and source env var key.

## Footer (build / hosting / hot / cold)

Always-visible system state:

- **build** — git sha, build age, uptime, started-at
- **hosting** — env, port, region, ingest auth state, dashboard auth state
- **hot** — store backend, db path, status, row count, oldest-event age
- **cold** — backend, bucket, archived count, last rotation, last probe, last error (if any) + `action` hint

When the cold tier reports `last_issue.kind: "WrongRegion"` or similar,
the `action` field on the issue tells you exactly what to fix.

## Download

The Download button in the toolbar exports the currently-filtered events as
NDJSON (capped at 2000). Filename includes the most-specific filter for
context: `logger-crab-<service-or-rid>-<timestamp>.ndjson`.

## Refresh

The refresh icon next to the gear is visible only when there's state worth
preserving (any filter or non-default cursor). On the default "latest 100"
view it's hidden because navigating to `/` already gives you the latest.

## Theme

Top-right toggle (☾ / ☀). Auto-detects OS theme on first visit; explicit
choice persists in localStorage. Changes to OS theme update live unless
you've set an explicit preference.

## Mobile (≤720px)

- Filter form stacks vertically
- Table becomes a card layout (one card per event, fields prefixed with
  uppercase column labels)
- Nav collapses to icons + theme/settings toggles only

## See also

- [`ADOPTION.md`](./ADOPTION.md) — what to emit (producer side)
- [`STORAGE.md`](./STORAGE.md) — hot/cold tier details
- [`DEPLOY.md`](./DEPLOY.md) — env vars, auth setup
- [`EVENT_TAXONOMY.md`](./EVENT_TAXONOMY.md) — naming conventions
