# logger-crab — Research Report

Research compiled for designing a Rust-based centralized logging service ("logger-crab") with SQLite hot tier + S3 NDJSON cold tier on Render Starter, targeting ~10k events/day across Next.js, FastAPI, a Node worker, and cron jobs. Goal: correlate `request_id` across every service.

---

## OSS / Self-Hostable

### Grafana Loki

**Schema.** Loki is built around the _stream_ — a unique combination of labels — and _chunks_ of log lines per stream. Labels (low-cardinality key/value tags like `app=fastapi`) are first-class and indexed; the log line itself is opaque text. Loki resists high label cardinality on purpose: too many label combinations explode the index. Newer versions added "structured metadata" — non-indexed key/values stored alongside the line, recovering some structured-log ergonomics without blowing up the index.

**Naming/categorization.** No built-in dotted namespace. Categorization is whatever you put in your labels (`service`, `env`, `level`).

**Correlation.** Loki itself does not propagate request IDs; the convention is to log them in the line and use LogQL regex to extract. Grafana panels can derive trace links from log fields.

**Storage tiers.** Hot data lives in ingester memory and recent chunks; cold lives in object storage (S3/GCS). Index (TSDB or BoltDB) is shipped to the same object store. There is one tier in object storage, but compactor + retention rules enforce TTLs.

**Pluggable backends.** Pluggable object store (S3/GCS/Azure/filesystem) and pluggable index (TSDB/BoltDB). Not pluggable at the query/format layer.

**Dashboards/queries.** LogQL — PromQL-like, with `{label=value}` filter then `|= "substring"` then optional metric extraction. Time series and faceted slicing fall out for free.

**Shipper.** Promtail/Alloy and Vector batch lines per stream and push via gRPC/HTTP with WAL-backed retry.

**Novel.** The label-vs-payload dichotomy is the whole architecture. Cardinality discipline is a _feature_ — it is what makes the index small and queries fast on commodity hardware.

---

### ClickHouse for logs (Cloudflare, Uber, Discord-adjacent, ClickHouse LogHouse)

**Schema.** ClickHouse log tables typically have a small set of columnar "hot" fields (`timestamp`, `service`, `level`, `request_id`, `trace_id`) plus a `Map(String, String)` (or `Map(LowCardinality(String), String)`) for everything else. Cloudflare went further and stored fields as parallel typed arrays (`names[]`, `values_string[]`, `values_int[]`) so a single noisy app cannot corrupt the schema. ClickHouse LogHouse uses materialized views from a Null landing table into SharedMergeTree to allow schema evolution without touching pipelines.

**Categorization.** Driven by columns. Service, env, source come from columns; everything else lives in a Map and is queried with `mapValues`/`map['key']`.

**Correlation.** `trace_id`/`request_id` are first-class indexed columns; ORDER BY usually puts them after the partition key so range scans by trace are cheap.

**Storage tiers.** A single store, but TTL clauses move parts to S3-backed storage policies after N days. ClickHouse LogHouse uses ZSTD level 1 over S3 — small enough for cold, fast enough for warm.

**Pluggable backends.** ClickHouse storage policies abstract local NVMe vs S3-compatible object storage. Compute is single-engine.

**Dashboards/queries.** SQL with array/map functions, `argMax`, `topK`, `quantilesExact`, sampling clauses. Cloudflare's _ABR_ (Adaptive Bit Rate) sampling is striking: store identical data at 100%, 10%, 1%, 0.1% in parallel tables and pick the right one per query window.

**Shipper.** Mostly OTel collector or Vector → Kafka → batch insert. ClickHouse's own LogHouse skipped Kafka because direct batch inserts handle the volume.

**Novel.** "Wide events with a `Map` escape hatch" is the dominant pattern. Bloom filters on map keys/values give cheap lookup without indexing every key. ABR is genuinely counter-intuitive.

---

### Vector

**Schema.** Internal event model is a polymorphic `log | metric | trace`. Logs are `BTreeMap<String, Value>` with a few well-known paths (`.timestamp`, `.message`, `.host`).

**Architecture.** Sources → Transforms (VRL — Vector Remap Language) → Sinks, as a DAG. Transforms can fan out, route, sample, filter, parse.

**Pluggable backends.** _This_ is the design lesson. Vector has 50+ sinks (Loki, Elasticsearch, ClickHouse, S3, Kafka, files, HTTP, Sentry, Datadog…). Each sink owns its batching, encoding, retry, and ack semantics.

**Shipper.** Disk or memory buffers per sink; backpressure either blocks upstream or drops newest. End-to-end ack is opt-in. Retries use exponential backoff with jitter.

**Novel.** VRL — a small typed expression language for mutating events at line speed — beats embedded Lua/Python on perf and safety. The "buffer per sink with explicit drop policy" model is the right answer for a small service.

---

### Fluent Bit / Fluentd

**Schema.** Records are `(timestamp, tag, map)`. Tag is the routing key.

**Categorization.** Tag-based, dot-separated (`app.fastapi.access`). Filters and outputs match tag patterns (`app.*`, `app.fastapi.*`).

**Pluggable backends.** Many output plugins; the chain is Input → Parser → Filter → Buffer → Router → Output. Filters can rewrite tags (route via mutation).

**Shipper.** Persistent buffer (memory or filesystem). Bounded queues with drop or block.

**Novel.** Tag-as-routing-key is a tiny, powerful primitive. Parsers (regex, JSON, ltsv, logfmt) decoupled from inputs.

---

### Seq + Serilog message templates

**Schema.** An event has `@t` (timestamp), `@l` (level), `@mt` (message template), `@m` (rendered message), `@x` (exception), `@i` (event id, hash of template), plus arbitrary user properties. The _template_ itself is a first-class indexed property.

**Categorization.** By `@i` — the template hash. All "User {UserId} logged in" events share an event ID regardless of which user, so you can chart "this kind of event" without grepping rendered strings.

**Novel.** _Message templates_ are the most underrated idea in logging. `log.info("User {UserId} logged in from {Ip}", userId, ip)` produces both a human-readable string AND a structured event with `UserId` and `Ip` as named properties — no double-write, no separate metadata arg, no string-format-then-parse. The `@i` template hash gives you free event-type aggregation.

---

### Graylog / GELF

**Schema.** Required: `version` (1.1), `host`, `short_message`, `timestamp`. Optional: `full_message`, `level` (syslog 0–7), `_*` (any user fields, underscore-prefixed). Chunked UDP for large payloads.

**Novel.** The underscore prefix for custom fields is a clean namespace separation between "well-known" and "user-defined" without nesting. Compact but extensible.

---

### OpenTelemetry Logs spec

**Schema (12 fields).** `Timestamp`, `ObservedTimestamp`, `TraceId`, `SpanId`, `TraceFlags`, `SeverityText`, `SeverityNumber` (1–24, 4 per level), `Body` (string OR structured), `Resource` (process/service identity, _static per source_), `InstrumentationScope` (library/module emitting), `Attributes` (per-event), `EventName` (event class / typed event).

**Novel — Resource/Scope/Record split.** Three layers of context. Resource is the _machine/service_ (`service.name`, `host.name`, `deployment.environment`) and is sent ONCE per batch, not per record — huge bandwidth win. Scope is the _library or module_ (`tracing-subscriber`, `myapp::auth`). Record is _this event_. This three-layer model is the cleanest schema in the survey and maps perfectly onto a multi-service correlator.

**Severity numbers.** 1–4 TRACE, 5–8 DEBUG, 9–12 INFO, 13–16 WARN, 17–20 ERROR, 21–24 FATAL. Four slots per level lets you express "warn but worse" without inventing a new level.

---

## Hosted / Commercial (design ideas only)

### Honeycomb

**Schema.** A single wide event per unit of work — every interesting field as a top-level attribute. Cardinality is unconstrained on purpose.

**Correlation.** Trace ID + span ID + parent span ID, propagated head-based via W3C `traceparent`. Refinery (their tail sampler) buffers all spans for a trace and then decides keep/drop based on the whole trace (errors, slow, rule match).

**Sampling.** Dynamic, per-key. Tracks incoming volume per key, adjusts sample rate to hit a target throughput. The actual sample rate is _recorded on the event_ so query-time multiplication reconstructs accurate counts.

**Novel.** "Wide events, not narrow logs." BubbleUp: pick a slow region of a heatmap and Honeycomb diffs every attribute against the baseline to surface what's different. Sample rate as a recorded field is the unlock — without it, dynamic sampling lies to your dashboards.

---

### BetterStack (Logtail)

**Schema.** Wide events — explicitly markets "wide events vs time series." Accepts OTel, Vector, raw HTTP. ClickHouse-backed under the hood.

**Novel.** Live-tail with VRL transforms applied at query time. Predictable per-GB pricing with retention tiers.

---

### Datadog Logs

**Schema.** Reserved attributes (`timestamp`, `status`, `host`, `service`, `message`, `dd.trace_id`, `dd.span_id`, `ddsource`) plus arbitrary attributes. _Facets_ are user-promoted attributes that become indexed/aggregatable.

**Pipelines.** Ordered chain of processors (grok, JSON parser, status remapper, attribute remapper, lookup tables). Nested pipelines for team/service-level scoping.

**Novel.** _Index vs Archive split._ Indexed (queryable, expensive, short retention) and Archive (S3 NDJSON, cheap, rehydratable). Exclusion filters drop high-volume noise from index but keep archive. This is exactly logger-crab's hot/cold model.

---

### Sentry breadcrumbs

**Schema.** `type`, `category`, `message`, `data`, `level`, `timestamp`. Ring buffer (default 100) attached to every error event.

**Novel.** Breadcrumbs are _not stored as logs_ — they are a circular in-memory buffer that gets shipped as part of the error payload. So instead of the logger sending everything, the _error_ drags its context with it. For low-budget hot-path correlation this is brilliant: zero cost unless something goes wrong.

---

### Axiom

**Schema.** Schema-on-read dataset. Every field discovered at ingest, vacuumed when fields fall out of retention. Column store, APL query language (KQL-like).

**Novel.** Dataset-per-domain (logs, traces, metrics, security) is recommended over one big dataset to avoid field count explosion. Field count is the main cost driver.

---

## Library / SDK design

### Pino (Node)

`logger.child({ requestId })` returns a sub-logger that prepends bindings to every line. Child creation is cheap (~26µs). Transports are out-of-process workers (`pino.transport`) so the hot path only writes JSON to a worker thread.

**Lessons.** Child loggers + bound context. Out-of-process transport so the app never blocks on network IO.

### structlog (Python)

`merge_contextvars` processor reads from `contextvars.ContextVar` (asyncio-safe, thread-safe). At request entry: `clear_contextvars(); bind_contextvars(request_id=..., user_id=...)`. Every subsequent log call within the request scope auto-includes those fields.

**Lessons.** _Per-request bound context via context-local storage_ is the killer feature. Devs write `log.info("created file")` and `request_id`, `user_id`, `team_id` come along for free.

### tracing (Rust)

Spans (with begin/end) and events (point in time). Fields are typed. `#[instrument]` attribute auto-creates a span around a function and records arguments as fields. Layered subscriber model — `tracing-subscriber` composes filter, format, and sink layers. `tracing` integrates with OTel via `tracing-opentelemetry`.

**Lessons.** The Layer model is the right abstraction for mixed sinks (stdout pretty + logger-crab JSON + OTel). Fields-on-spans propagate to all events inside the span — same idea as bound context, but scoped to lexical/async region.

---

## Synthesis — Feature Backlog for logger-crab

Verdict legend: GREEN = ship in V1; YELLOW = V1.5/V2 once you understand the load; RED = skip or defer indefinitely for this scale.

### V1 (foundation)

1. **OTel-shaped event schema** — top-level `timestamp`, `severity_number`, `severity_text`, `service`, `env`, `request_id`, `trace_id`, `span_id`, `event_name`, `body`, `attributes` (JSON map). Avoid reinventing field names. _Source: OpenTelemetry._ **GREEN** — costs nothing, gives you free interop with OTel SDKs later.

2. **Resource / Scope / Record three-layer split** — send `resource` (service identity) once per batch, not per event. Saves 60–80% of bandwidth at small scale where every event repeats `service`, `host`, `env`. _Source: OpenTelemetry._ **GREEN** — protocol-level decision; cheap now, painful to retrofit.

3. **Severity numbers (1–24, four per level)** — store the integer, render the text. Lets you express WARN/WARN+/WARN++ without new levels. _Source: OpenTelemetry._ **GREEN**.

4. **`request_id` as first-class indexed column in SQLite** — indexed `BLOB` (raw 16 bytes) or `TEXT`. The whole point of the product. _Source: ClickHouse, Datadog `dd.trace_id`._ **GREEN**.

5. **Bound-context shipper SDK (per language)** — `with_context(request_id=...)` sets a context-local; every `info/warn/error` inside auto-includes it. Pino child / structlog contextvars / tracing span fields. _Source: Pino, structlog, tracing._ **GREEN** — this is what makes correlation actually happen at the call site.

6. **Background batching shipper, never blocks the hot path** — accumulate events, flush on size or interval, retry with exponential backoff + jitter, drop-newest on full buffer with a counter. _Source: Vector, Pino transports._ **GREEN**.

7. **Hot SQLite + cold S3 NDJSON two-tier with explicit promotion** — query hot for recent (last 24–72h), background job rolls older to S3 NDJSON partitioned by `dt=YYYY-MM-DD/service=...`. _Source: Datadog Index/Archive split, ClickHouse storage policies._ **GREEN** — already in your design; just make sure the cold format is grep-able with `aws s3 cp - | jq`.

8. **Underscore-prefixed user attribute namespace** — store user attributes under `attributes.*` (or `_*` per GELF) so well-known fields never collide with user payload. _Source: GELF, OTel attributes._ **GREEN**.

9. **Drop-newest with a `dropped_count` field on the next event** — when the shipper buffer overflows, drop new and stamp the next-shipped event with how many were lost. Prevents silent loss. _Source: Vector buffers + observability convention._ **GREEN**.

10. **Sample rate recorded on the event** — even if you don't sample in V1, reserve `sample_rate: 1` as a top-level field. The day you turn on sampling, your charts won't lie. _Source: Honeycomb._ **GREEN** — costs 1 byte; saves a migration later.

### V1.5 (after you have a week of real data)

11. **Message templates with template hash (`@i`)** — `log.info("user {user_id} did {action}", ...)` stores both the rendered message and a stable template ID. Lets you chart "events of this kind" without grep. _Source: Seq/Serilog._ **GREEN** — biggest dev-ergonomics win in the survey; pairs naturally with Rust's `tracing` field model.

12. **Per-template event-type aggregation view** — auto-materialize "top N event templates by count, last 24h" so you can see "this template fired 50k times today" without a query. _Source: Seq._ **GREEN**.

13. **Faceted filter UI driven by attribute discovery** — periodically scan recent events, surface top keys as filter chips. _Source: Datadog facets, Axiom field discovery._ **YELLOW** — useful but only after you have a UI; build the API first.

14. **Sentry-style breadcrumb mode** — opt-in ring buffer per `request_id`, only flushed when an `ERROR`/`FATAL` event occurs in that context. Drops 95% of debug noise but keeps it when something breaks. _Source: Sentry._ **GREEN** — perfect fit for 10k events/day budget.

15. **Trace-aware tail sampling for long requests** — buffer events keyed by `request_id` for ~30s; on close, decide keep-all (slow or errored) vs sample (fast and clean). _Source: Honeycomb Refinery._ **YELLOW** — only worth it once you exceed budget; over-engineering at 10k/day.

16. **Pluggable sink trait** — define a Rust `Sink` trait so SQLite, S3, stdout, and (later) ClickHouse/Loki are interchangeable. Vector did this; you'll thank yourself when you outgrow Render. _Source: Vector._ **GREEN** — small upfront cost; large optionality.

17. **VRL-lite or Rhai/CEL transform layer** — let users write small expressions to redact PII, drop noisy events, or rename fields without redeploying. _Source: Vector VRL, BetterStack._ **YELLOW** — sexy but ~2 weeks; defer until users ask.

18. **Map column with bloom filter for unknown attributes** — in SQLite this is a JSON blob with FTS5 over keys; in a future ClickHouse port it's `Map(LowCardinality(String), String)` with bloom_filter index. _Source: ClickHouse LogHouse._ **GREEN** — design the JSON shape now to mirror the future Map shape.

19. **Tag-based routing primitive** — events carry a `tag` (`app.fastapi.access`, `worker.credit.charge`); routing rules ship matching tags to different sinks/retentions. _Source: Fluent Bit._ **YELLOW** — only if you ever need multi-destination routing.

20. **Live tail endpoint (SSE)** — `GET /tail?service=fastapi&request_id=...` streams new events. Trivial in Rust with `tokio::sync::broadcast`. _Source: BetterStack live tail._ **GREEN** — high perceived value, low effort.

### V2 (scale or polish)

21. **Dynamic per-key sampling with recorded `sample_rate`** — when an event-template fires more than N/sec, automatically reduce its sample rate. Multiplies counts at query time. _Source: Honeycomb dynamic sampler._ **YELLOW** — only if 10k/day grows to 10M/day.

22. **ABR-style multi-resolution cold storage** — write 100% to NDJSON and 1% to a "fast scan" parquet alongside; queries auto-pick resolution. _Source: Cloudflare ABR._ **RED** — overkill at this scale.

23. **BubbleUp-style "what's different" diff** — pick a time/event slice in the UI and diff attribute distributions vs baseline. _Source: Honeycomb BubbleUp._ **YELLOW** — killer feature, but needs real volume to be useful.

24. **Schema-on-read with vacuum job** — auto-discover attribute fields, drop fields that fall out of retention. _Source: Axiom._ **YELLOW** — needed when attribute count gets unwieldy (1k+); you're nowhere near that.

25. **W3C `traceparent` header propagation helpers in every SDK** — auto-extract incoming `traceparent`, inject outgoing on `fetch`/`requests`/`reqwest`. _Source: OTel + Honeycomb._ **GREEN** — request-id propagation is your product; W3C is the free standard.

26. **Materialized "event-type" rollups (count, p50/p95 latency by template hash)** — small SQLite tables refreshed every minute; powers fast dashboards without aggregating raw events. _Source: Seq event-id rollups, ClickHouse materialized views._ **GREEN**.

27. **Exclusion filters at ingest** — drop matching events before they hit hot storage but optionally still write to cold S3. _Source: Datadog exclusion filters._ **GREEN** — the cheapest way to control volume.

28. **Per-tenant `X-Scope-OrgID` style multi-tenancy hook** — even if you only have one tenant now, partition the SQLite/S3 path by `tenant_id`. _Source: Loki._ **YELLOW** — only if there's any chance of multi-tenant.

29. **Out-of-process shipper option (sidecar)** — for the FastAPI worker, a small Rust sidecar that owns the HTTP push so the app process never blocks. _Source: Pino transports._ **YELLOW** — V2 perf optimization; in-process async is fine for 10k/day.

30. **Layered subscriber model in the Rust ingest core** — compose filter → enrich → route → sink as Tower-style layers, mirroring `tracing-subscriber`. _Source: Rust tracing._ **GREEN** — clean architecture from day one; trivial to add later layers.

---

## Top recommendations summary

For a 10k events/day budget on Render Starter, the highest-leverage moves are:

- Adopt the **OTel three-layer schema** (Resource/Scope/Record) verbatim
- Make **`request_id` + `trace_id` first-class indexed columns**
- Ship a **bound-context SDK** per language (the structlog/Pino model)
- Steal **Seq's message templates** for `@i` event-type IDs
- Steal **Sentry's breadcrumb pattern** for cheap error-context capture
- Steal **Datadog's Index vs Archive split** — already your design, formalize it
- Reserve a **`sample_rate` field** even before sampling exists
- Define a **pluggable Sink trait** so SQLite is just one implementation

Skip ABR, BubbleUp, schema-on-read vacuum, and dynamic sampling until volume justifies them.
