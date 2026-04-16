# TODO

## CI expansion (beyond current sanity workflow)

The current `.github/workflows/ci.yml` runs `cargo fmt --check`, `cargo clippy -D warnings`, and `cargo test` on every push/PR to `main`. That catches format drift, lint regressions, and unit-test breakage but nothing more. Next steps, in priority order:

- [ ] **Integration tests for HTTP routes** — spin up `axum::serve` on an ephemeral port, hit `/ingest`, `/logs`, `/health`, `/`, `/api`, `/docs`, `/openapi.yaml`; assert status + content-type + basic body shape. Use `reqwest` or `axum::body::to_bytes` + `tower::ServiceExt`.
- [ ] **Hot-store round-trip test** — ingest a small batch via `MemoryHotStore`, query with each filter (request_id/service/env/min_severity/since/until/fts), assert expected counts. Mirror the manual validation done during the UI overhaul.
- [ ] **Cold-store smoke test** — use the `noop` cold store by default; optionally add a feature-gated job that runs against `minio` service container for S3 NDJSON rotation.
- [ ] **SQLite HotStore CI job** — matrix over `HOT_STORE=memory` and `HOT_STORE=sqlite` to prevent divergence between backends. Use `sqlite::memory:` for speed.
- [ ] **OpenAPI lint** — validate `openapi.yaml` with `redocly lint` or `swagger-cli` step before deploy.
- [ ] **Security scan** — `cargo audit` + `cargo deny` as a separate (non-blocking initially) job.
- [ ] **Build artifact** — `cargo build --release` and upload the `log-server` binary as a workflow artifact for quick smoke-deploy.
- [ ] **Render preview deploy** — add a conditional step on `main` pushes that calls the Render API to trigger deploy.
- [ ] **Playwright visual-regression** — reuse the three-iteration screenshot harness to catch unintended CSS regressions.

## Dashboard / product

- [ ] Persist client-side sort direction in localStorage so it survives reload.
- [ ] Add `aria-live="polite"` region to announce row-count changes after filter apply.
- [ ] Inline sparkline of event rate (per-minute, last 60m) above the toolbar.
- [ ] Export filtered page as NDJSON via a `⬇ export` toolbar button.
- [ ] Service/event chip filters should stack visually as "breadcrumbs" rather than replace the header.

## Storage / ops

- [ ] Rotation worker: move events older than `HOT_RETENTION_HOURS` from hot → cold on a tokio interval.
- [ ] `/logs?tail=true` SSE stream endpoint for live-tailing.
- [ ] Structured audit log for dashboard queries (who queried what).
