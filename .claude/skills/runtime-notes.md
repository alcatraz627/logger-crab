# Runtime notes — logger-crab

Prepend new sessions at the top. Each entry captures insights reusable by
future sessions (not a changelog — the commit log + WAL cover that).

## session: rebrand + dashboard polish [refac-nav-a0] — 2026-04-17

**Purpose:** Rebrand dashboard to "Versable logger-crab" with Versable favicon, fix git=unknown footer, replace native selects with rich pill/button-group filters, and lift footer truncation.

**Insights:**

1. **Render injects `RENDER_GIT_COMMIT` as a build ARG — but only if you declare it.** Docker build contexts strip `.git`, so `git rev-parse` in `build.rs` always returns "unknown" on Render. Fix needs both pieces: (a) `ARG RENDER_GIT_COMMIT="" / ENV RENDER_GIT_COMMIT=$RENDER_GIT_COMMIT` in the builder stage of the Dockerfile, and (b) `build.rs` reads the env var first, falls back to git command. Without (a), step (b) sees nothing — Render only auto-injects build ARGs that are explicitly declared in the Dockerfile.
2. **`PreEscaped` is mandatory for inline SVG in maud.** The `html!` macro escapes `<` and `>` by default, so an icon helper must wrap its raw SVG string in `PreEscaped(format!(r#"<svg ...>...</svg>"#))`. Pattern: write a single `svg_icon(path_d)` helper that builds the wrapper attrs and PreEscapes — then each individual icon function (`icon_check`, `icon_x`, etc.) is a one-liner returning `svg_icon(r#"<polyline ...>"#)`.
3. **Footer truncation is anti-feature when value can wrap.** The original `.ft-v { max-width: 60%; overflow: hidden; text-overflow: ellipsis; white-space: nowrap }` truncated `db = sqlite:///var/data/logs.db` to `sqlite:///var/d…`. Fix: drop the truncation rules, switch to `word-break: break-all`, and let the value wrap naturally. The `title` attribute with full value becomes redundant once the actual value is visible.
4. **Sticky-bottom footer pattern that actually works:** `body { min-height: 100vh; display: flex; flex-direction: column }` + `main { flex: 1 0 auto }` + `footer { flex-shrink: 0; margin-top: auto }`. Without `flex-shrink: 0`, the footer collapses on short pages; without `flex: 1 0 auto` on main, the footer floats up against the table.
5. **Custom rich select pattern (no JS).** Instead of a native `<select>`, render a row of `<a>` tags with `href` pointing to a URL with the param toggled on/removed. Active state computed server-side from `q.level`. Each option gets `aria-checked` and a `role="radiogroup"` parent for screen readers. URL stays the source of truth — no JS state to reconcile (same pattern as `filter_url_override` / `filter_url_remove` chips).
6. **Chrome MCP browser locks on `~/.cache/chrome-devtools-mcp/chrome-profile`.** When a previous session left the persistent Chrome profile in use, every `new_page`/`list_pages` call returns "browser is already running" — even with `isolatedContext`. Fallback: skip the screenshot, fetch HTML via `curl`, grep for the new selectors and key bits (link href, badge text). Faster anyway when you only need to confirm structure, not pixels.

---

## session: dashboard 3-iter overhaul + CI + deploy-prep [dash-iter-9c] — 2026-04-17

**Purpose:** Screenshot the dashboard, iterate 3× on visual/UX/a11y, bootstrap CI, prep Render deploy artifacts.

**Insights:**

1. **`-D warnings` in CI surfaces latent lints.** The repo had never run clippy with `-D warnings` before, so the first CI run hit `unnecessary_sort_by` and `to_string_in_format_args` in `crates/log-server/src/store/memory.rs`. Rule of thumb: any time you add `-D warnings` to a repo, expect a one-commit sweep afterward. Use `cargo clippy --fix --allow-dirty` to auto-apply, then re-check manually — it doesn't touch `Display`-arg lints.
2. **Playwright MCP sandbox restricts writes to CWD.** Full-page screenshots saved via `browser_take_screenshot` land in `.playwright-mcp/` relative to the server's CWD. Copy them out to `~/.claude/assets/screenshots/logger-crab/` with `cp` after capture — the direct path-argument to save outside CWD is denied.
3. **Port 8080 collides on macOS.** AirPlay Receiver grabs 8080 intermittently. Default dev port for logger-crab should be 8099 whenever iterating locally; the Dockerfile's `PORT=8080` is fine in production where it's isolated.
4. **Gum handles boxes, not arrows.** When rendering architecture diagrams in terminal via `gum style --border double`, the boxes align perfectly but inter-box arrows (`│` / `▼`) must be hand-crafted and positioned with spaces. `gum join --horizontal` only lays boxes side-by-side — it does not route arrows.
5. **Render blueprint `env: docker` is the older field; `runtime: docker` is newer.** Both work. The existing render.yaml uses `env: docker` — don't flip it to `runtime:` just for consistency, it's a no-op churn. Render auto-translates older field names.
6. **Dockerfile `/var/data` + `USER crab` ownership.** When the Dockerfile drops privileges to a non-root user AND Render mounts a persistent disk, pre-create the mount parent (`mkdir -p /var/data && chown crab:crab /var/data`) _before_ `USER crab`. Render mounts with the container user's UID, so pre-chown ensures the SQLite file can be created on first boot.
7. **`color-mix(in srgb, var(--x) N%, var(--y))`** is the cleanest way to derive chip hover/border tints from an accent var — no need for a separate `--accent-hover` var per palette. Works everywhere except IE (irrelevant).
8. **Active-filter chips with × remove links** are much clearer than a single "clear all" button. Use `filter_url_override(q, key, value)` / `filter_url_remove(q, key)` helpers that clone the current `DashboardQuery` struct and emit a fresh querystring — the URL is the source of truth, no JS state to reconcile.
9. **Double sort-arrow bug pattern.** If you use a text `↕` glyph in the header and a `::before` pseudo for active state, both render unless you explicitly zero `font-size` on the `.arr` span when active. One-line fix: `th.sortable.sort-asc .arr, th.sortable.sort-desc .arr { font-size: 0 }`.

---
