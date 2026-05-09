use axum::extract::{Query, State};
use axum::http::HeaderMap;
use axum::response::{Html, IntoResponse, Response};
use chrono::{DateTime, Utc};
use maud::{html, Markup, PreEscaped, DOCTYPE};
use serde::Deserialize;

use super::auth::TokenRecord;
use super::dashboard_url::{
    build_next_url_with_page, download_url, filter_url_override,
    filter_url_override_u32, filter_url_remove, parse_dashboard_dt,
    reset_to_first_page,
};
use super::nav::{
    icon_box, icon_branch, icon_check, icon_globe, icon_hash, icon_search, icon_x, render_nav,
    svg_icon, Active, BRAND_NAME, GITHUB_URL,
};
use super::{AppState, BootInfo};
use crate::error::AppError;
use crate::models::{ColdHealth, HotHealth, LogEvent, QueryParams};

const LEVELS: &[&str] = &["trace", "debug", "info", "warn", "error", "fatal"];
const PAGE_SIZES: &[u32] = &[5, 10, 25, 50, 100, 250, 500];

#[derive(Deserialize, Default)]
pub struct DashboardQuery {
    pub request_id: Option<String>,
    pub service: Option<String>,
    pub env: Option<String>,
    pub event_prefix: Option<String>,
    pub level: Option<String>,
    pub q: Option<String>,
    pub limit: Option<u32>,
    pub cursor: Option<String>,
    /// 1-indexed page counter. Incremented when "older →" is clicked,
    /// reset to 1 when "← newest" or any filter changes. Cursor pagination
    /// alone can't tell us "page 2 vs page 7"; this URL state does.
    pub page: Option<u32>,
    /// ISO-8601 datetime range. Both inclusive of their boundary. Browser
    /// `<input type="datetime-local">` posts these as `2026-05-09T03:30`
    /// (no seconds, no TZ), so the dashboard handler treats them as UTC
    /// for query purposes. See `parse_dashboard_dt` for parsing details.
    pub since: Option<String>,
    pub until: Option<String>,
    /// Paste-and-go login: visiting `/?token=XXX` validates the token,
    /// sets the dashboard cookie, and redirects back to `/` so the URL
    /// no longer carries the secret.
    pub token: Option<String>,
}

pub async fn get_dashboard(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<DashboardQuery>,
) -> Result<Response, AppError> {
    let expected = state.dashboard_token.as_deref().map(|s| s.as_str());

    // Paste-and-go: GET /?token=XXX → set cookie, 302 to / (so the secret
    // doesn't sit in the address bar or browser history).
    if let Some(token) = q.token.as_deref() {
        if let Some(exp) = expected {
            if token == exp {
                return Ok(super::dashboard_login::login_redirect_with_cookie(token, &headers));
            }
        }
        // Token in query but invalid (or no token configured) → fall through
        // to the login page, which will render with an error notice.
        return Ok(super::dashboard_login::render_login_page(true).into_response());
    }

    // No ?token= param — check existing auth (cookie or Bearer).
    if !super::auth::check_dashboard_auth(&headers, expected) {
        return Ok(super::dashboard_login::render_login_page(false).into_response());
    }

    let page_size = q.limit.unwrap_or(100).clamp(10, 500);
    let params = QueryParams {
        request_id: not_empty(&q.request_id),
        user_id: None,
        session_id: None,
        service: not_empty(&q.service),
        env: not_empty(&q.env),
        event_prefix: not_empty(&q.event_prefix),
        min_severity: not_empty(&q.level).as_deref().map(level_to_min_severity),
        since: parse_dashboard_dt(q.since.as_deref()),
        until: parse_dashboard_dt(q.until.as_deref()),
        fts: not_empty(&q.q),
        limit: page_size,
        cursor: not_empty(&q.cursor),
    };
    let health = state.hot.health().await.ok();
    let cold_health = state.cold.health().await.ok();

    // Cold-tier auto-routing: if the user picked a `since` older than
    // hot's oldest event, the data they want is in the cold tier. We query
    // cold instead of hot. Only triggers when a real S3 backend is configured
    // (cold_health.backend == "s3") to avoid wasted no-op calls.
    let cold_oldest_ok = cold_health
        .as_ref()
        .map(|c| c.backend == "s3" && c.ok)
        .unwrap_or(false);
    let hot_oldest = health.as_ref().and_then(|h| h.oldest_ts);
    let queried_cold = match (params.since, hot_oldest, cold_oldest_ok) {
        (Some(since), Some(oldest), true) if since < oldest => true,
        (Some(_), None, true) => true, // hot empty, cold has S3 backend
        _ => false,
    };

    // Three modes:
    //   1. Straddle: `since` is older than hot.oldest_ts AND `until` is newer
    //      (or absent). Run two queries — cold for [since, hot.oldest), hot
    //      for [hot.oldest, until] — and concatenate. Naturally dedup-free
    //      because the boundary is a point in time and rotation only writes
    //      to cold then deletes from hot (no overlap by design).
    //   2. Cold-only: `since` is older AND `until` < hot.oldest_ts (or hot
    //      is genuinely empty). Cold tier is the right answer.
    //   3. Hot-only: default, what we always did.
    let queried_straddle = match (params.since, params.until, hot_oldest, cold_oldest_ok) {
        (Some(since), until, Some(oldest), true) => {
            since < oldest && until.map(|u| u >= oldest).unwrap_or(true)
        }
        _ => false,
    };

    let (page, queried_cold) = if queried_straddle {
        let oldest = hot_oldest.expect("guarded by queried_straddle match");

        let mut cold_params = params.clone();
        cold_params.until = Some(oldest);
        cold_params.cursor = None; // straddle ignores cursor for V1
        let cold_page = state.cold.read_range(&cold_params).await?;

        let mut hot_params = params.clone();
        hot_params.since = Some(oldest);
        let hot_page = state.hot.query(&hot_params).await?;

        let mut merged = hot_page.events;
        merged.extend(cold_page.events);
        merged.sort_by_key(|e| std::cmp::Reverse(e.ts));
        merged.truncate(params.limit as usize);

        // Surface BOTH next-cursors? Straddle pagination is V2; for now we
        // return the hot tier's cursor (newer half) since most users will
        // walk back from there.
        (
            crate::models::QueryPage {
                events: merged,
                next_cursor: hot_page.next_cursor,
            },
            true,
        )
    } else if queried_cold {
        let cold_page = state.cold.read_range(&params).await?;
        (cold_page, true)
    } else {
        (state.hot.query(&params).await?, false)
    };

    // Real filtered count (only when filters are active — without filters
    // it would equal hot.rows, which we already have).
    let filter_active_for_count = params.request_id.is_some()
        || params.user_id.is_some()
        || params.session_id.is_some()
        || params.service.is_some()
        || params.env.is_some()
        || params.event_prefix.is_some()
        || params.min_severity.is_some()
        || params.fts.is_some();
    let filtered_count = if filter_active_for_count {
        state.hot.count(&params).await.ok()
    } else {
        None
    };

    // Distinct values for the filter datalists, served from a 60s cache
    // to avoid hitting the store on every dashboard render.
    let (datalist_services, datalist_envs, datalist_event_prefixes) =
        cached_distinct_values(&state).await;
    let markup = render(
        &q,
        &page.events,
        page.next_cursor.as_deref(),
        health.as_ref(),
        cold_health.as_ref(),
        &state.boot,
        &state.ingest_tokens,
        &state.config_warnings,
        filtered_count,
        &datalist_services,
        &datalist_envs,
        &datalist_event_prefixes,
        queried_cold,
    );
    Ok(Html(markup.into_string()).into_response())
}

fn not_empty(s: &Option<String>) -> Option<String> {
    s.as_ref().filter(|x| !x.trim().is_empty()).cloned()
}

fn level_to_min_severity(s: &str) -> u8 {
    match s.to_ascii_lowercase().as_str() {
        "trace" => 1,
        "debug" => 5,
        "info" => 9,
        "warn" | "warning" => 13,
        "error" => 17,
        "fatal" => 21,
        _ => 1,
    }
}

/// Deterministic 8-color chip index for service/event-namespace coloring.
fn hash_color(s: &str) -> u32 {
    let mut h: u32 = 5381;
    for b in s.bytes() {
        h = h.wrapping_mul(33).wrapping_add(b as u32);
    }
    h % 8
}

fn event_namespace(event: &str) -> (&str, &str) {
    match event.split_once('.') {
        Some((ns, rest)) => (ns, rest),
        None => (event, ""),
    }
}

fn fmt_relative(ts: DateTime<Utc>) -> String {
    let now = Utc::now();
    let delta = (now - ts).num_seconds();
    if delta < 5 {
        return "just now".into();
    }
    if delta < 60 {
        return format!("{delta}s ago");
    }
    let m = delta / 60;
    if m < 60 {
        return format!("{m}m ago");
    }
    let h = m / 60;
    if h < 24 {
        return format!("{h}h ago");
    }
    let d = h / 24;
    format!("{d}d ago")
}

fn fmt_uptime(seconds: i64) -> String {
    let d = seconds / 86400;
    let h = (seconds % 86400) / 3600;
    let m = (seconds % 3600) / 60;
    let s = seconds % 60;
    if d > 0 {
        return format!("{d}d {h}h {m}m");
    }
    if h > 0 {
        return format!("{h}h {m}m");
    }
    if m > 0 {
        return format!("{m}m {s}s");
    }
    format!("{s}s")
}

fn fmt_build_time(unix: u64) -> String {
    if unix == 0 {
        return "unknown".into();
    }
    DateTime::<Utc>::from_timestamp(unix as i64, 0)
        .map(|t| t.format("%Y-%m-%d %H:%M UTC").to_string())
        .unwrap_or_else(|| "unknown".into())
}

const CSS: &str = include_str!("dashboard.css");
const JS: &str = include_str!("dashboard.js");

// render_settings_modal moved to `dashboard_modal.rs`.

#[allow(clippy::too_many_arguments)]
fn render(
    q: &DashboardQuery,
    events: &[LogEvent],
    next_cursor: Option<&str>,
    health: Option<&HotHealth>,
    cold: Option<&ColdHealth>,
    boot: &BootInfo,
    consumers: &[TokenRecord],
    config_warnings: &[String],
    filtered_count: Option<u64>,
    services: &[String],
    envs: &[String],
    event_prefixes: &[String],
    queried_cold: bool,
) -> Markup {
    let total = events.len();
    let health_ok = health.map(|h| h.ok).unwrap_or(false);
    let row_count = health.map(|h| h.rows).unwrap_or(0);
    let oldest = health.and_then(|h| h.oldest_ts);

    let any_filter_active = q.request_id.as_deref().is_some_and(|s| !s.is_empty())
        || q.service.as_deref().is_some_and(|s| !s.is_empty())
        || q.env.as_deref().is_some_and(|s| !s.is_empty())
        || q.event_prefix.as_deref().is_some_and(|s| !s.is_empty())
        || q.level.as_deref().is_some_and(|s| !s.is_empty())
        || q.q.as_deref().is_some_and(|s| !s.is_empty())
        || q.since.as_deref().is_some_and(|s| !s.is_empty())
        || q.until.as_deref().is_some_and(|s| !s.is_empty());

    // The refresh button is visible only when there's state worth preserving
    // — a filter or a paginated cursor. On the default unfiltered first page,
    // navigating to `/` already gives the latest, so refresh is redundant.
    let show_refresh = any_filter_active || q.cursor.as_deref().is_some_and(|s| !s.is_empty());

    html! {
        (DOCTYPE)
        html {
            head {
                meta charset="utf-8";
                meta name="viewport" content="width=device-width, initial-scale=1";
                title { (BRAND_NAME) " · dashboard" }
                link rel="icon" type="image/svg+xml" href="/favicon.svg";
                link rel="preconnect" href="https://fonts.googleapis.com";
                link rel="preconnect" href="https://fonts.gstatic.com" crossorigin;
                link rel="stylesheet"
                    href="https://fonts.googleapis.com/css2?family=Inter:wght@400;500;600;700&family=JetBrains+Mono:wght@400;500&display=swap";
                style { (PreEscaped(CSS)) }
                // Prism for inline payload JSON highlighting. Scoped via
                // payload-only token color rules in the main CSS so it does
                // not bleed into other code blocks.
                link rel="stylesheet"
                    href="https://cdnjs.cloudflare.com/ajax/libs/prism/1.29.0/themes/prism.min.css";
            }
            body data-show-refresh=[if show_refresh { Some("1") } else { None }] {
                (render_nav(Active::Dashboard, Some(health_ok)))

                // Datalists power native browser autocomplete on the inputs
                // below. Distinct values come from the currently-loaded events
                // — narrow but useful (the values that exist in your view).
                datalist id="dl-services" {
                    @for s in services.iter() { option value=(s) {} }
                }
                datalist id="dl-envs" {
                    @for v in envs.iter() { option value=(v) {} }
                }
                datalist id="dl-event-prefixes" {
                    @for p in event_prefixes.iter() { option value=(p) {} }
                }

                form.filters method="get" action="/" {
                    div.filter-group {
                        label { (icon_hash()) "request_id" }
                        input type="text" name="request_id" autocomplete="off"
                            value=[q.request_id.as_deref()] placeholder="req_abc_01";
                    }
                    div.filter-group {
                        label { (icon_box()) "service" }
                        input type="text" name="service" list="dl-services" autocomplete="off"
                            value=[q.service.as_deref()] placeholder="versable-api";
                    }
                    div.filter-group {
                        label { (icon_globe()) "env" }
                        input type="text" name="env" list="dl-envs" autocomplete="off"
                            value=[q.env.as_deref()] placeholder="prod";
                    }
                    div.filter-group {
                        label { (icon_branch()) "event prefix" }
                        input type="text" name="event_prefix" list="dl-event-prefixes" autocomplete="off"
                            value=[q.event_prefix.as_deref()] placeholder="pipeline.";
                    }
                    div.filter-group.grow {
                        label { (icon_search()) "full-text search" }
                        input type="text" name="q" autocomplete="off"
                            value=[q.q.as_deref()] placeholder="message or payload…";
                    }
                    div.filter-group.actions {
                        label { "\u{00a0}" }
                        div.btn-row {
                            button.btn-apply type="submit" title="apply filters (Enter)" {
                                (icon_check()) span { "Apply" }
                            }
                            @if any_filter_active {
                                a.btn-reset href="/" title="clear all filters" {
                                    (icon_x()) span { "Reset" }
                                }
                            }
                        }
                    }

                    div.filter-row {
                        (level_pill_filter(q))
                        div.filter-group.date-range-group {
                            label { "since (UTC)" }
                            input type="datetime-local" name="since"
                                value=[q.since.as_deref().filter(|s| !s.is_empty())];
                        }
                        div.filter-group.date-range-group {
                            label { "until (UTC)" }
                            input type="datetime-local" name="until"
                                value=[q.until.as_deref().filter(|s| !s.is_empty())];
                        }
                        div.tz-toggle-group {
                            label { "display tz" }
                            div.tz-toggle role="radiogroup" aria-label="display timestamps in timezone" {
                                button.tz-toggle-opt.tz-utc type="button" data-tz="utc" aria-checked="true" { "UTC" }
                                button.tz-toggle-opt.tz-local type="button" data-tz="local" aria-checked="false" { "Local" }
                            }
                        }
                        div.grow { }
                        (page_size_selector(q))
                    }
                }

                @if queried_cold {
                    div.cold-tier-banner role="status" {
                        span.cold-tier-icon { "❄" }
                        span.cold-tier-msg {
                            "Showing archived events from the cold tier (S3). "
                            "Queries are capped at 5000 events; narrow "
                            code { "since" } " / " code { "until" } " for finer slices."
                        }
                    }
                }

                div.toolbar {
                    div.count {
                        @if any_filter_active {
                            @if let Some(matching_total) = filtered_count {
                                span.num { (total) } " on this page · "
                                span.num { (matching_total) } " matching · "
                                span.dim { (row_count) " in store" }
                            } @else {
                                span.num { (total) } " on this page · "
                                span.dim { (row_count) " in store" }
                            }
                        } @else {
                            span.num { (total) } " shown · "
                            span.dim { (row_count) " total" }
                        }
                        @if let Some(ts) = oldest {
                            span.dim { " · oldest " (fmt_relative(ts)) }
                        }
                    }
                    a.toolbar-download
                        href=(download_url(q))
                        title="download events matching the current filter as NDJSON" {
                        (icon_download()) span { "Download" }
                    }
                    (render_active_filters(q))
                    div.grow { }
                    span.hint title="Press / to focus search" { "press " kbd { "/" } " to search" }
                    div.pager-group {
                        @if q.cursor.as_deref().is_some_and(|s| !s.is_empty()) {
                            a.pager.pager-newer href=(reset_to_first_page(q))
                                title="jump to newest events" {
                                "← newest"
                            }
                        } @else {
                            span.pager.pager-newer.disabled title="already at newest" { "← newest" }
                        }
                        @let page_num = q.page.unwrap_or(1);
                        span.pager-page title=(format!("page {page_num}")) { "page " (page_num) }
                        @if let Some(cursor) = next_cursor {
                            a.pager.pager-older href=(build_next_url_with_page(q, cursor))
                                title="page back through older events" {
                                "older →"
                            }
                        } @else {
                            span.pager.pager-older.disabled title="no older events" { "older →" }
                        }
                    }
                }

                main.table-wrap {
                    @if events.is_empty() {
                        div.empty {
                            div.empty-icon { "∅" }
                            @if any_filter_active {
                                div.empty-title { "No events match the current filters" }
                                div.empty-hint {
                                    "Try removing one or more filters, or widening the search range."
                                }
                                a.empty-cta href="/" title="reset every filter" {
                                    (icon_x()) span { "Clear all filters" }
                                }
                            } @else {
                                div.empty-title { "No events yet" }
                                div.empty-hint {
                                    "POST events to " code { "/ingest" } ", or boot with "
                                    code { "SEED_ON_BOOT=1" } " to auto-populate dummy data."
                                }
                            }
                        }
                    } @else {
                        table id="events-table" {
                            thead {
                                tr {
                                    th.sortable data-sort="ts"   aria-sort="descending" scope="col" tabindex="0" { "time" span.arr { "↕" } }
                                    th.sortable data-sort="sev"  aria-sort="none" scope="col" tabindex="0" { "level" span.arr { "↕" } }
                                    th.sortable data-sort="svc"  aria-sort="none" scope="col" tabindex="0" { "service" span.arr { "↕" } }
                                    th.sortable data-sort="env"  aria-sort="none" scope="col" tabindex="0" { "env" span.arr { "↕" } }
                                    th.sortable data-sort="evt"  aria-sort="none" scope="col" tabindex="0" { "event" span.arr { "↕" } }
                                    th.sortable data-sort="rid"  aria-sort="none" scope="col" tabindex="0" { "request_id" span.arr { "↕" } }
                                    th scope="col" { "message / payload" }
                                }
                            }
                            tbody {
                                @for (i, e) in events.iter().enumerate() {
                                    @let (ns, leaf) = event_namespace(&e.event);
                                    tr.evrow
                                        data-ts=(e.ts.timestamp_millis())
                                        data-sev=(e.severity_number)
                                        data-svc=(e.service.as_deref().unwrap_or(""))
                                        data-env=(e.env.as_deref().unwrap_or(""))
                                        data-evt=(e.event)
                                        data-rid=(e.request_id)
                                        style=(format!("animation-delay:{}ms", (i.min(30) as u32) * 18))
                                    {
                                        td.ts data-label="time" {
                                            span.ts-rel data-iso=(e.ts.to_rfc3339())
                                                title=(e.ts.format("%Y-%m-%d %H:%M:%S UTC").to_string()) {
                                                (fmt_relative(e.ts))
                                            }
                                            span.ts-abs data-iso=(e.ts.to_rfc3339())
                                                data-utc-fmt=(e.ts.format("%b %-d, %H:%M:%S").to_string()) {
                                                (e.ts.format("%b %-d, %H:%M:%S").to_string())
                                            }
                                        }
                                        td data-label="level" {
                                            a class=(format!("row-lvl row-lvl-{}", e.severity_text.to_ascii_lowercase()))
                                                href=(filter_url_override(q, "level", &e.severity_text))
                                                title=(format!("filter by min severity ≥ {}", e.severity_text)) {
                                                (e.severity_text.to_ascii_uppercase())
                                            }
                                        }
                                        td data-label="service" {
                                            @if let Some(svc) = &e.service {
                                                a class=(format!("row-svc svc-c{}", hash_color(svc)))
                                                    href=(filter_url_override(q, "service", svc))
                                                    title=(format!("filter by service: {svc}")) {
                                                    (svc)
                                                }
                                            } @else {
                                                span.dim { "—" }
                                            }
                                        }
                                        td data-label="env" {
                                            @if let Some(env) = &e.env {
                                                a class=(format!("row-env env-{}", env_class(env)))
                                                    href=(filter_url_override(q, "env", env))
                                                    title=(format!("filter by env: {env}")) {
                                                    (env.to_ascii_uppercase())
                                                }
                                            } @else {
                                                span.dim { "—" }
                                            }
                                        }
                                        td.evt data-label="event" {
                                            a class=(format!("evt-ns evt-ns-link ns-c{}", hash_color(ns)))
                                                href=(filter_url_override(q, "event_prefix", &format!("{ns}.")))
                                                title=(format!("filter by event prefix: {ns}.")) {
                                                (ns)
                                            }
                                            @if !leaf.is_empty() {
                                                span.evt-sep { "." }
                                                span.evt-leaf { (leaf) }
                                            }
                                        }
                                        td.rid-cell data-label="request_id" {
                                            @if e.request_id.is_empty() {
                                                span.rid-empty title="no request_id" { "—" }
                                            } @else {
                                                span.rid-group {
                                                    a.rid href=(filter_url_override(q, "request_id", &e.request_id))
                                                        title=(format!("filter by request_id: {}", e.request_id)) {
                                                        (rid_short(&e.request_id))
                                                    }
                                                    button.rid-copy
                                                        type="button"
                                                        data-copy=(e.request_id)
                                                        title=(format!("copy {}", e.request_id))
                                                        aria-label="copy request_id" {
                                                        (icon_copy())
                                                    }
                                                }
                                            }
                                        }
                                        td.msg data-label="message" {
                                            details {
                                                summary {
                                                    @let preview = render_msg_preview(e);
                                                    @if preview.is_empty() {
                                                        span.msg-dim { "(no message)" }
                                                    } @else {
                                                        span.msg-text { (preview) }
                                                    }
                                                }
                                                div.payload {
                                                    div.payload-head {
                                                        button.payload-copy
                                                            type="button"
                                                            title="copy payload as JSON"
                                                            aria-label="copy payload" {
                                                            (icon_copy()) span { "Copy JSON" }
                                                        }
                                                    }
                                                    pre { code class="payload-json language-json" {
                                                        (serde_json::to_string_pretty(&e.payload)
                                                            .unwrap_or_else(|_| "—".into()))
                                                    } }
                                                    @if e.user_id.is_some() || e.session_id.is_some() || e.client_id.is_some() {
                                                        div.identity {
                                                            @if let Some(u) = &e.user_id {
                                                                span.id-chip { "user " code { (u) } }
                                                            }
                                                            @if let Some(s) = &e.session_id {
                                                                span.id-chip { "session " code { (s) } }
                                                            }
                                                            @if let Some(c) = &e.client_id {
                                                                span.id-chip { "client " code { (c) } }
                                                            }
                                                        }
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }

                (render_footer(boot, health, cold))

                (super::dashboard_modal::render_settings_modal(consumers, config_warnings))
                (super::dashboard_modal::render_kbd_toast())

                // Prism core + JSON grammar — used for in-row payload highlighting.
                script src="https://cdnjs.cloudflare.com/ajax/libs/prism/1.29.0/prism.min.js" {}
                script src="https://cdnjs.cloudflare.com/ajax/libs/prism/1.29.0/components/prism-json.min.js" {}
                script { (PreEscaped(JS)) }
            }
        }
    }
}

fn env_class(env: &str) -> &'static str {
    match env.to_ascii_lowercase().as_str() {
        "prod" | "production" => "prod",
        "staging" | "stage" => "stage",
        "dev" | "development" | "local" => "dev",
        _ => "other",
    }
}

#[cfg(test)]
mod render_tests {
    use super::*;

    #[test]
    fn rid_short_returns_full_string() {
        let id = "abcdefghijklmnop";
        assert_eq!(rid_short(id), id);
    }

    #[test]
    fn truncate_chars_respects_unicode() {
        let s = "café-extra";
        let result = truncate_chars(s, 5);
        assert!(result.chars().count() <= 5);
    }

    #[test]
    fn render_msg_preview_uses_message_when_present() {
        use crate::models::LogEvent;
        use chrono::Utc;
        let e = LogEvent {
            request_id: "r1".into(),
            event: "x".into(),
            severity_number: 9,
            severity_text: "info".into(),
            ts: Utc::now(),
            message: Some("the actual message".into()),
            service: None, env: None,
            user_id: None, session_id: None, client_id: None,
            payload: serde_json::json!({"k": "v"}),
        };
        assert_eq!(render_msg_preview(&e), "the actual message");
    }

    #[test]
    fn render_msg_preview_falls_back_to_payload_when_message_empty() {
        use crate::models::LogEvent;
        use chrono::Utc;
        let e = LogEvent {
            request_id: "r1".into(),
            event: "x".into(),
            severity_number: 9,
            severity_text: "info".into(),
            ts: Utc::now(),
            message: None,
            service: None, env: None,
            user_id: None, session_id: None, client_id: None,
            payload: serde_json::json!({"job_id": "abc", "_internal": "skip"}),
        };
        let preview = render_msg_preview(&e);
        assert!(preview.contains("job_id=abc"));
        assert!(!preview.contains("_internal"));
    }

    // render_login_page + is_https tests moved to `dashboard_login.rs`
}

/// Returns (services, envs, event_prefixes) — served from the AppState
/// cache when within TTL, refreshed from the hot store when stale.
async fn cached_distinct_values(state: &AppState) -> (Vec<String>, Vec<String>, Vec<String>) {
    let needs_refresh = {
        let cache = state.distinct_cache.lock().await;
        match cache.last_refresh {
            Some(ts) => {
                (Utc::now() - ts).num_seconds() > super::DISTINCT_CACHE_TTL_SECS
            }
            None => true,
        }
    };

    if needs_refresh {
        let services = state.hot.distinct_values("service", 200).await.unwrap_or_default();
        let envs = state.hot.distinct_values("env", 50).await.unwrap_or_default();
        let event_prefixes = state.hot.distinct_values("event_prefix", 200).await.unwrap_or_default();
        let mut cache = state.distinct_cache.lock().await;
        cache.services = services.clone();
        cache.envs = envs.clone();
        cache.event_prefixes = event_prefixes.clone();
        cache.last_refresh = Some(Utc::now());
        (services, envs, event_prefixes)
    } else {
        let cache = state.distinct_cache.lock().await;
        (
            cache.services.clone(),
            cache.envs.clone(),
            cache.event_prefixes.clone(),
        )
    }
}

// URL builders moved to `dashboard_url.rs`.

fn render_active_filters(q: &DashboardQuery) -> Markup {
    let active: Vec<(&'static str, String, &'static str)> = [
        ("request_id", q.request_id.clone(), "req"),
        ("service", q.service.clone(), "svc"),
        ("env", q.env.clone(), "env"),
        ("event_prefix", q.event_prefix.clone(), "evt"),
        ("level", q.level.clone(), "lvl"),
        ("q", q.q.clone(), "q"),
        ("since", q.since.clone(), "since"),
        ("until", q.until.clone(), "until"),
    ]
    .into_iter()
    .filter_map(|(k, v, label)| v.filter(|s| !s.is_empty()).map(|v| (k, v, label)))
    .collect();

    html! {
        @if !active.is_empty() {
            span.active-filters {
                @for (key, val, label) in &active {
                    a.active-chip href=(filter_url_remove(q, key))
                        title=(format!("remove filter: {label}={val}")) {
                        span.af-key { (label) ":" }
                        span.af-val { (truncate(val, 22)) }
                        span.af-x { "×" }
                    }
                }
            }
        }
    }
}

fn truncate(s: &str, n: usize) -> String {
    if s.len() <= n {
        s.to_string()
    } else {
        format!("{}…", &s[..n.saturating_sub(1)])
    }
}

fn render_footer(
    boot: &BootInfo,
    health: Option<&HotHealth>,
    cold: Option<&ColdHealth>,
) -> Markup {
    let git_sha_clean = boot.git_sha.trim_end_matches("-dirty");
    let git_is_dirty = boot.git_sha.ends_with("-dirty");
    let git_known = !git_sha_clean.is_empty() && git_sha_clean != "unknown";
    let commit_url = if git_known {
        format!("{}/commit/{}", GITHUB_URL, git_sha_clean)
    } else {
        String::new()
    };

    html! {
        footer.lc-footer {
            div.ft-col {
                div.ft-title { "build" }
                div.ft-row {
                    span.ft-k { "git" }
                    @if git_known {
                        a.ft-v.mono.ft-link href=(commit_url) target="_blank" rel="noopener"
                            title=(format!("view commit {git_sha_clean} on GitHub")) {
                            (git_sha_clean)
                            @if git_is_dirty { span.ft-dirty title="working tree was dirty at build" { "·dirty" } }
                        }
                    } @else {
                        span.ft-v.mono.warn title="set RENDER_GIT_COMMIT in builder env" { "unknown" }
                    }
                }
                div.ft-row {
                    span.ft-k { "built" }
                    span.ft-v title=(fmt_build_time(boot.build_time_unix)) {
                        (fmt_build_age(boot.build_time_unix))
                    }
                }
                div.ft-row { span.ft-k { "uptime" } span.ft-v { (fmt_uptime(boot.uptime_seconds())) } }
                div.ft-row {
                    span.ft-k { "started" }
                    span.ft-v { (boot.started_at.format("%Y-%m-%d %H:%M UTC").to_string()) }
                }
            }
            div.ft-col {
                div.ft-title { "hosting" }
                div.ft-row {
                    span.ft-k { "env" }
                    span class=(format!("ft-v env-pill env-{}", env_class(&boot.env_name))) {
                        (boot.env_name.to_ascii_uppercase())
                    }
                }
                div.ft-row { span.ft-k { "port" } span.ft-v.mono { (boot.port) } }
                div.ft-row { span.ft-k { "region" } span.ft-v.mono { (boot.aws_region) } }
                div.ft-row {
                    span.ft-k { "ingest auth" }
                    @if boot.has_ingest_token {
                        span.ft-v.ok { "● set" }
                    } @else {
                        span.ft-v.warn { "○ unset" }
                    }
                }
                div.ft-row {
                    span.ft-k { "dash auth" }
                    @if boot.has_dashboard_token {
                        span.ft-v.ok { "● set" }
                    } @else {
                        span.ft-v.warn { "○ unset" }
                    }
                }
            }
            div.ft-col {
                div.ft-title { "hot · sqlite" }
                div.ft-row { span.ft-k { "store" } span.ft-v.mono { (boot.hot_store) } }
                div.ft-row { span.ft-k { "db" } span.ft-v.mono { (boot.database_url_masked) } }
                @if let Some(h) = health {
                    div.ft-row {
                        span.ft-k { "status" }
                        @if h.ok { span.ft-v.ok { "● online" } }
                        @else { span.ft-v.err { "● offline" } }
                    }
                    div.ft-row { span.ft-k { "rows" } span.ft-v.mono { (h.rows) } }
                    @if let Some(ts) = h.oldest_ts {
                        div.ft-row {
                            span.ft-k { "oldest" }
                            span.ft-v title=(ts.format("%Y-%m-%d %H:%M:%S UTC").to_string()) {
                                (fmt_relative(ts))
                            }
                        }
                    }
                } @else {
                    div.ft-row { span.ft-k { "status" } span.ft-v.err { "● offline" } }
                }
            }
            div.ft-col {
                div.ft-title { "cold · " (boot.cold_store) }
                @if let Some(c) = cold {
                    div.ft-row {
                        span.ft-k { "status" }
                        @if c.ok { span.ft-v.ok { "● online" } }
                        @else { span.ft-v.err { "● offline" } }
                    }
                    div.ft-row {
                        span.ft-k { "backend" }
                        span.ft-v.mono { (c.backend) }
                    }
                    @if let Some(bucket) = &c.bucket {
                        div.ft-row { span.ft-k { "bucket" } span.ft-v.mono { (bucket) } }
                    } @else if let Some(bucket) = &boot.s3_bucket {
                        div.ft-row { span.ft-k { "bucket" } span.ft-v.mono { (bucket) } }
                    } @else {
                        div.ft-row { span.ft-k { "bucket" } span.ft-v.dim { "—" } }
                    }
                    @if c.events_archived_total > 0 {
                        div.ft-row {
                            span.ft-k { "archived" }
                            span.ft-v.mono { (c.events_archived_total) " events" }
                        }
                    }
                    @if let Some(ts) = c.last_rotation {
                        div.ft-row {
                            span.ft-k { "last rotation" }
                            span.ft-v title=(ts.format("%Y-%m-%d %H:%M:%S UTC").to_string()) {
                                (fmt_relative(ts))
                            }
                        }
                    } @else {
                        div.ft-row { span.ft-k { "last rotation" } span.ft-v.dim { "—" } }
                    }
                    @if let Some(ts) = c.last_health_check {
                        div.ft-row {
                            span.ft-k { "last probe" }
                            span.ft-v title=(ts.format("%Y-%m-%d %H:%M:%S UTC").to_string()) {
                                (fmt_relative(ts))
                            }
                        }
                    }
                    @if let Some(issue) = &c.last_issue {
                        div.ft-row {
                            span.ft-k { "issue" }
                            span.ft-v.err
                                title=(format!("{}: {}", issue.kind, issue.summary)) {
                                (truncate_err(&issue.kind, 32))
                            }
                        }
                        div.ft-row {
                            span.ft-k { "" }
                            span.ft-v title=(issue.summary.as_str()) {
                                (truncate_err(&issue.summary, 48))
                            }
                        }
                        @if let Some(action) = &issue.action {
                            div.ft-row {
                                span.ft-k { "action" }
                                span.ft-v.warn title=(action) {
                                    (truncate_err(action, 48))
                                }
                            }
                        }
                    }
                } @else {
                    div.ft-row { span.ft-k { "status" } span.ft-v.warn { "○ unavailable" } }
                }
            }
        }
    }
}

// login_redirect_with_cookie + is_https moved to `dashboard_login.rs`.

// render_login_page + LOGIN_CSS + LOGIN_THEME_JS moved to `dashboard_login.rs`.

// render_kbd_toast moved to `dashboard_modal.rs`.

fn icon_download() -> Markup {
    svg_icon(
        r#"<path d="M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4"/><polyline points="7 10 12 15 17 10"/><line x1="12" y1="15" x2="12" y2="3"/>"#,
    )
}

fn icon_copy() -> Markup {
    svg_icon(
        r#"<rect x="9" y="9" width="13" height="13" rx="2" ry="2"/><path d="M5 15H4a2 2 0 0 1-2-2V4a2 2 0 0 1 2-2h9a2 2 0 0 1 2 2v1"/>"#,
    )
}

// download_url moved to `dashboard_url.rs`.

/// Returns the full request_id. Used to be truncated to 8 chars + ellipsis,
/// but the user explicitly asked for the full id rendered (smaller font,
/// no chip-style bg/border). Kept as a function so future formatting
/// changes (e.g. opt-in truncation when the cell is narrow) have one place
/// to land.
fn rid_short(rid: &str) -> String {
    rid.to_string()
}

/// Inline preview of an event's message + payload for the collapsed details row.
/// Shows the message if present; otherwise the first useful payload key=value pair.
/// Truncated to ~80 chars so it fits the table cell on one line.
fn render_msg_preview(e: &LogEvent) -> String {
    if let Some(msg) = e.message.as_deref().filter(|m| !m.is_empty()) {
        return truncate_chars(msg, 80);
    }
    // No message — pull one or two fields from payload to give a hint.
    if let Some(obj) = e.payload.as_object() {
        let parts: Vec<String> = obj
            .iter()
            .filter(|(k, _)| !k.starts_with('_')) // skip server-stamped fields like _auth_consumer
            .take(2)
            .map(|(k, v)| match v {
                serde_json::Value::String(s) => format!("{k}={s}"),
                _ => format!("{k}={v}"),
            })
            .collect();
        if !parts.is_empty() {
            return truncate_chars(&parts.join(" · "), 80);
        }
    }
    String::new()
}

fn truncate_chars(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        s.to_string()
    } else {
        let head: String = s.chars().take(n.saturating_sub(1)).collect();
        format!("{head}…")
    }
}

fn truncate_err(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        s.to_string()
    } else {
        let mut out: String = s.chars().take(n.saturating_sub(1)).collect();
        out.push('…');
        out
    }
}

fn fmt_build_age(unix: u64) -> String {
    if unix == 0 {
        return "unknown".into();
    }
    let delta = (Utc::now().timestamp() - unix as i64).max(0);
    let m = delta / 60;
    if m < 60 {
        return format!("{m}m ago");
    }
    let h = m / 60;
    if h < 48 {
        return format!("{h}h ago");
    }
    let d = h / 24;
    format!("{d}d ago")
}

// ── Custom rich select replacements ──────────────────────────────────────────

/// Row of clickable level chips (replaces native `<select name="level">`).
/// Each chip uses the same `lvl-*` color tokens as the table cells, so the
/// active filter visually matches the rows it filters.
fn level_pill_filter(q: &DashboardQuery) -> Markup {
    let active = q.level.as_deref().unwrap_or("").to_ascii_lowercase();
    html! {
        div.filter-group {
            label { "min level" }
            div.lvl-pill-row role="radiogroup" aria-label="filter by minimum severity" {
                a.lvl-pill-opt.lvl-any
                    aria-checked=(if active.is_empty() { "true" } else { "false" })
                    href=(filter_url_remove(q, "level")) {
                    "any"
                }
                @for lv in LEVELS {
                    @let is_active = active == *lv;
                    a class=(format!(
                        "lvl-pill-opt lvl-pill {} {}",
                        match *lv {
                            "trace" => "lvl-trace", "debug" => "lvl-debug",
                            "info" => "lvl-info", "warn" => "lvl-warn",
                            "error" => "lvl-error", _ => "lvl-fatal",
                        },
                        if is_active { "is-active" } else { "" },
                    ))
                        aria-checked=(if is_active { "true" } else { "false" })
                        href=(filter_url_override(q, "level", lv))
                        title=(format!("min severity ≥ {lv}")) {
                        (lv.to_ascii_uppercase())
                    }
                }
            }
        }
    }
}

/// Button-group of page sizes (replaces native `<select name="limit">`).
fn page_size_selector(q: &DashboardQuery) -> Markup {
    let current = q.limit.unwrap_or(100);
    html! {
        div.filter-group {
            label { "page size" }
            div.page-size-row role="radiogroup" aria-label="rows per page" {
                @for n in PAGE_SIZES {
                    @let is_active = current == *n;
                    a class=(format!("page-size-opt {}", if is_active { "is-active" } else { "" }))
                        aria-checked=(if is_active { "true" } else { "false" })
                        href=(filter_url_override_u32(q, "limit", *n))
                        title=(format!("show {n} rows per page")) {
                        (n)
                    }
                }
            }
        }
    }
}

// filter_url_override_u32 moved to `dashboard_url.rs`.
