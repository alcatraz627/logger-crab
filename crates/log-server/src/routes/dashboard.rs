use axum::extract::{Query, State};
use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
use axum::response::{Html, IntoResponse, Redirect, Response};
use chrono::{DateTime, Utc};
use maud::{html, Markup, PreEscaped, DOCTYPE};
use serde::Deserialize;

use super::auth::{AuthRole, TokenRecord};
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
                return Ok(login_redirect_with_cookie(token, &headers));
            }
        }
        // Token in query but invalid (or no token configured) → fall through
        // to the login page, which will render with an error notice.
        return Ok(render_login_page(true).into_response());
    }

    // No ?token= param — check existing auth (cookie or Bearer).
    if !super::auth::check_dashboard_auth(&headers, expected) {
        return Ok(render_login_page(false).into_response());
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
    let page = state.hot.query(&params).await?;
    let health = state.hot.health().await.ok();
    let cold_health = state.cold.health().await.ok();

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

fn render_settings_modal(consumers: &[TokenRecord], warnings: &[String]) -> Markup {
    let full_count = consumers.iter().filter(|c| matches!(c.tier, AuthRole::Full)).count();
    let public_count = consumers.iter().filter(|c| matches!(c.tier, AuthRole::Public)).count();

    html! {
        dialog id="settings-modal" class="settings-dialog" {
            div.settings-shell {
                header.settings-header {
                    div.settings-title { "Settings" }
                    button.settings-close type="button" id="settings-close" title="close (Esc)"
                        aria-label="close settings" { "✕" }
                }

                section.settings-section {
                    div.settings-section-head {
                        h3 { "Registered consumers" }
                        span.settings-chip {
                            (consumers.len()) " total · "
                            span.settings-chip-full { (full_count) " full" }
                            " · "
                            span.settings-chip-public { (public_count) " public" }
                        }
                    }
                    @if consumers.is_empty() {
                        div.settings-empty {
                            "No consumers configured. Set "
                            code { "INGEST_TOKEN_<NAME>=<tier>:<token>" }
                            " env vars to register one."
                        }
                    } @else {
                        table.settings-table {
                            thead {
                                tr {
                                    th { "consumer" }
                                    th { "tier" }
                                    th { "source env var" }
                                }
                            }
                            tbody {
                                @for c in consumers {
                                    tr {
                                        td.settings-name { (c.name) }
                                        td {
                                            @match c.tier {
                                                AuthRole::Full => span.settings-tier.tier-full { "full" },
                                                AuthRole::Public => span.settings-tier.tier-public { "public" },
                                            }
                                        }
                                        td.settings-source { code { (c.source_env_var) } }
                                    }
                                }
                            }
                        }
                    }
                }

                section.settings-section {
                    div.settings-section-head {
                        h3 { "Config warnings" }
                        @if warnings.is_empty() {
                            span.settings-chip.settings-chip-ok { "● clean" }
                        } @else {
                            span.settings-chip.settings-chip-warn {
                                (warnings.len()) " issue" (if warnings.len() == 1 { "" } else { "s" })
                            }
                        }
                    }
                    @if warnings.is_empty() {
                        div.settings-empty.settings-empty-ok {
                            "All " code { "INGEST_TOKEN_*" } " env vars parsed successfully and no deprecated vars detected."
                        }
                    } @else {
                        ul.settings-warnings {
                            @for w in warnings {
                                li { (w) }
                            }
                        }
                    }
                }

                footer.settings-footer {
                    span.settings-foot-note {
                        "Tokens themselves are never displayed. Edit values in your hosting provider's env-var UI; restart logger-crab to apply."
                    }
                }
            }
        }
    }
}

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

                (render_settings_modal(consumers, config_warnings))
                (render_kbd_toast())

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
mod url_tests {
    use super::*;

    fn q_empty() -> DashboardQuery { DashboardQuery::default() }

    fn q_with(svc: Option<&str>, env: Option<&str>, cursor: Option<&str>, page: Option<u32>) -> DashboardQuery {
        DashboardQuery {
            service: svc.map(String::from),
            env: env.map(String::from),
            cursor: cursor.map(String::from),
            page,
            ..Default::default()
        }
    }

    #[test]
    fn current_pairs_skips_empty_strings() {
        let q = q_with(Some(""), Some("dev"), None, None);
        let pairs = current_pairs(&q);
        // Only env should survive; empty service is dropped.
        assert_eq!(pairs.len(), 1);
        assert_eq!(pairs[0], ("env", "dev".into()));
    }

    #[test]
    fn current_pairs_preserves_filters_and_page() {
        let q = q_with(Some("api"), Some("prod"), Some("c1"), Some(3));
        let pairs = current_pairs(&q);
        assert!(pairs.contains(&("service", "api".into())));
        assert!(pairs.contains(&("env", "prod".into())));
        assert!(pairs.contains(&("page", "3".into())));
        // Cursor is intentionally NOT in current_pairs — pagination state is
        // managed separately by build_next_url_with_page / reset_to_first_page.
        // Including it would mean filter changes accidentally carry the cursor.
        assert!(!pairs.iter().any(|(k, _)| *k == "cursor"));
    }

    #[test]
    fn current_pairs_omits_page_when_one() {
        let q = q_with(Some("api"), None, None, Some(1));
        let pairs = current_pairs(&q);
        assert!(!pairs.iter().any(|(k, _)| *k == "page"), "page=1 should not appear in URL");
    }

    #[test]
    fn filter_url_override_replaces_existing_value() {
        let q = q_with(Some("old-svc"), Some("dev"), None, None);
        let url = filter_url_override(&q, "service", "new-svc");
        assert!(url.contains("service=new-svc"));
        assert!(!url.contains("old-svc"));
        assert!(url.contains("env=dev"), "env should be preserved");
    }

    #[test]
    fn filter_url_remove_drops_just_one_key() {
        let q = q_with(Some("api"), Some("prod"), Some("c1"), None);
        let url = filter_url_remove(&q, "cursor");
        assert!(url.contains("service=api"));
        assert!(url.contains("env=prod"));
        assert!(!url.contains("cursor"));
    }

    #[test]
    fn reset_to_first_page_drops_cursor_and_page() {
        let q = q_with(Some("api"), None, Some("c1"), Some(7));
        let url = reset_to_first_page(&q);
        assert!(url.contains("service=api"), "filter survives");
        assert!(!url.contains("cursor"));
        assert!(!url.contains("page"));
    }

    #[test]
    fn build_next_url_with_page_increments() {
        let q = q_with(Some("api"), None, Some("old-cursor"), Some(2));
        let url = build_next_url_with_page(&q, "new-cursor");
        assert!(url.contains("cursor=new-cursor"));
        assert!(url.contains("page=3"));
        assert!(!url.contains("old-cursor"));
        assert!(!url.contains("page=2"));
    }

    #[test]
    fn build_next_url_with_page_starts_at_2_from_default() {
        let q = q_empty();
        let url = build_next_url_with_page(&q, "c1");
        assert!(url.contains("page=2"));
    }

    #[test]
    fn pairs_to_url_returns_root_when_empty() {
        assert_eq!(pairs_to_url(&[]), "/");
    }

    #[test]
    fn pairs_to_url_urlencodes_values() {
        let pairs = vec![("q", "hello world".to_string())];
        let url = pairs_to_url(&pairs);
        assert!(url.contains("hello%20world"), "spaces should be percent-encoded");
    }

    #[test]
    fn rid_short_returns_full_string() {
        // Behavior changed: was truncating, now returns full id.
        let id = "abcdefghijklmnop";
        assert_eq!(rid_short(id), id);
    }

    #[test]
    fn truncate_chars_respects_unicode() {
        // Should not split a multi-byte char in half.
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
        // Underscore-prefixed keys (server-stamped) are excluded.
        assert!(!preview.contains("_internal"));
    }

    #[test]
    fn parse_dashboard_dt_handles_browser_format() {
        // Browser datetime-local sends without seconds + without TZ
        let parsed = parse_dashboard_dt(Some("2026-05-09T03:30")).unwrap();
        assert_eq!(parsed.format("%Y-%m-%d %H:%M:%S").to_string(), "2026-05-09 03:30:00");
    }

    #[test]
    fn parse_dashboard_dt_handles_seconds_form() {
        let parsed = parse_dashboard_dt(Some("2026-05-09T03:30:45")).unwrap();
        assert_eq!(parsed.format("%H:%M:%S").to_string(), "03:30:45");
    }

    #[test]
    fn parse_dashboard_dt_returns_none_on_garbage() {
        assert!(parse_dashboard_dt(Some("not a date")).is_none());
        assert!(parse_dashboard_dt(Some("")).is_none());
        assert!(parse_dashboard_dt(None).is_none());
        assert!(parse_dashboard_dt(Some("   ")).is_none());
    }

    #[test]
    fn current_pairs_includes_date_range() {
        let mut q = DashboardQuery::default();
        q.since = Some("2026-05-01T00:00".into());
        q.until = Some("2026-05-09T00:00".into());
        let pairs = current_pairs(&q);
        assert!(pairs.contains(&("since", "2026-05-01T00:00".into())));
        assert!(pairs.contains(&("until", "2026-05-09T00:00".into())));
    }

    #[test]
    fn render_login_page_contains_login_form() {
        // Smoke test for the login page maud — just check the response body
        // includes the form action + the brand. Catches gross structural changes.
        use axum::body::to_bytes;
        let resp = render_login_page(false).into_response();
        let bytes = futures::executor::block_on(to_bytes(resp.into_body(), 65536)).unwrap();
        let body = std::str::from_utf8(&bytes).unwrap();
        assert!(body.contains("Versable logger-crab"));
        assert!(body.contains(r#"action="/""#));
        assert!(body.contains(r#"name="token""#));
        assert!(body.contains("Sign in"));
        // Login page should NOT show the dashboard filters (would mean leaked data)
        assert!(!body.contains("name=\"service\""));
        assert!(!body.contains("name=\"env\""));
    }

    #[test]
    fn render_login_page_with_invalid_token_shows_error() {
        use axum::body::to_bytes;
        let resp = render_login_page(true).into_response();
        let bytes = futures::executor::block_on(to_bytes(resp.into_body(), 65536)).unwrap();
        let body = std::str::from_utf8(&bytes).unwrap();
        assert!(body.contains("Invalid token"));
    }

    #[test]
    fn is_https_detects_x_forwarded_proto() {
        use axum::http::{HeaderMap, HeaderValue};
        let mut h = HeaderMap::new();
        assert!(!is_https(&h));
        h.insert("x-forwarded-proto", HeaderValue::from_static("https"));
        assert!(is_https(&h));
        h.insert("x-forwarded-proto", HeaderValue::from_static("HTTPS"));
        assert!(is_https(&h), "case-insensitive");
        h.insert("x-forwarded-proto", HeaderValue::from_static("http"));
        assert!(!is_https(&h));
    }
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

/// "older →" URL: bump the page counter, swap the cursor, drop nothing else.
/// Replaces the old `build_next_url` helper which didn't track page state.
#[allow(dead_code)]
fn build_next_url(q: &DashboardQuery, cursor: &str) -> String {
    build_next_url_with_page(q, cursor)
}

fn build_next_url_with_page(q: &DashboardQuery, cursor: &str) -> String {
    let next_page = q.page.unwrap_or(1) + 1;
    let mut pairs: Vec<(&'static str, String)> = current_pairs(q)
        .into_iter()
        .filter(|(k, _)| *k != "cursor" && *k != "page")
        .collect();
    pairs.push(("cursor", cursor.to_string()));
    pairs.push(("page", next_page.to_string()));
    pairs_to_url(&pairs)
}

fn filter_url_override(q: &DashboardQuery, key: &str, value: &str) -> String {
    let mut pairs: Vec<(&'static str, String)> =
        current_pairs(q).into_iter().filter(|(k, _)| *k != key).collect();
    let static_key: &'static str = match key {
        "request_id" => "request_id",
        "service" => "service",
        "env" => "env",
        "event_prefix" => "event_prefix",
        "level" => "level",
        "q" => "q",
        _ => return pairs_to_url(&pairs),
    };
    pairs.push((static_key, value.to_string()));
    pairs_to_url(&pairs)
}

fn current_pairs(q: &DashboardQuery) -> Vec<(&'static str, String)> {
    let mut pairs: Vec<(&'static str, String)> = Vec::new();
    // Empty strings — produced when the GET form submits with blank inputs —
    // are dropped here so they don't pollute the URL and the active-filter
    // chip strip with bogus "key=" entries.
    let push = |pairs: &mut Vec<(&'static str, String)>, k: &'static str, v: &Option<String>| {
        if let Some(s) = v {
            if !s.is_empty() {
                pairs.push((k, s.clone()));
            }
        }
    };
    push(&mut pairs, "request_id", &q.request_id);
    push(&mut pairs, "service", &q.service);
    push(&mut pairs, "env", &q.env);
    push(&mut pairs, "event_prefix", &q.event_prefix);
    push(&mut pairs, "level", &q.level);
    push(&mut pairs, "q", &q.q);
    push(&mut pairs, "since", &q.since);
    push(&mut pairs, "until", &q.until);
    if let Some(v) = q.limit {
        pairs.push(("limit", v.to_string()));
    }
    // page is meaningful only with cursor; preserved here so the
    // active-filter strip / download URL keep the right context.
    if let Some(p) = q.page {
        if p > 1 {
            pairs.push(("page", p.to_string()));
        }
    }
    pairs
}

/// Parse the browser's datetime-local string into a UTC `DateTime`.
/// Browsers post `2026-05-09T03:30` (no seconds, no offset) — we treat as UTC.
/// Returns None for empty/missing/malformed input.
fn parse_dashboard_dt(s: Option<&str>) -> Option<chrono::DateTime<chrono::Utc>> {
    let s = s?.trim();
    if s.is_empty() {
        return None;
    }
    // Try with and without seconds.
    for fmt in ["%Y-%m-%dT%H:%M:%S", "%Y-%m-%dT%H:%M"] {
        if let Ok(naive) = chrono::NaiveDateTime::parse_from_str(s, fmt) {
            return Some(naive.and_utc());
        }
    }
    None
}

fn pairs_to_url(pairs: &[(&str, String)]) -> String {
    if pairs.is_empty() {
        return "/".to_string();
    }
    let query: String =
        pairs.iter().map(|(k, v)| format!("{k}={}", urlenc(v))).collect::<Vec<_>>().join("&");
    format!("/?{query}")
}

fn urlenc(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char);
            }
            _ => out.push_str(&format!("%{:02X}", b)),
        }
    }
    out
}

fn filter_url_remove(q: &DashboardQuery, key: &str) -> String {
    let pairs: Vec<(&'static str, String)> =
        current_pairs(q).into_iter().filter(|(k, _)| *k != key).collect();
    pairs_to_url(&pairs)
}

/// "← newest" URL: drop both cursor + page so the user lands on page 1.
fn reset_to_first_page(q: &DashboardQuery) -> String {
    let pairs: Vec<(&'static str, String)> = current_pairs(q)
        .into_iter()
        .filter(|(k, _)| *k != "cursor" && *k != "page")
        .collect();
    pairs_to_url(&pairs)
}

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

/// Builds the 302 redirect that lands the user on `/` with the dashboard
/// cookie set. `Max-Age=2592000` = 30 days; `HttpOnly` blocks JS access;
/// `SameSite=Strict` blocks cross-site auto-send. `Secure` is added when
/// the request reached us over HTTPS — Render and most production reverse
/// proxies set `X-Forwarded-Proto: https`, so we honor that. Local
/// HTTP-only dev runs without `Secure` so the cookie is still set.
fn login_redirect_with_cookie(token: &str, headers: &HeaderMap) -> Response {
    let secure_flag = if is_https(headers) { "; Secure" } else { "" };
    let cookie = format!(
        "{}={}; Path=/; HttpOnly; SameSite=Strict; Max-Age=2592000{}",
        super::auth::DASHBOARD_COOKIE,
        token,
        secure_flag,
    );
    let mut response = Redirect::to("/").into_response();
    if let Ok(value) = HeaderValue::from_str(&cookie) {
        response.headers_mut().insert(header::SET_COOKIE, value);
    }
    response
}

/// Detect whether the original request was HTTPS. Trusts `X-Forwarded-Proto`
/// which Render (and most reverse proxies) set after TLS termination.
fn is_https(headers: &HeaderMap) -> bool {
    headers
        .get("x-forwarded-proto")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.eq_ignore_ascii_case("https"))
        .unwrap_or(false)
}

/// Custom login page — single password input, no username field. Replaces
/// the browser-native HTTP Basic prompt. Submits via GET to `/?token=...`
/// which the dashboard handler turns into a cookie + redirect.
fn render_login_page(token_was_invalid: bool) -> impl IntoResponse {
    let markup = html! {
        (DOCTYPE)
        html {
            head {
                meta charset="utf-8";
                meta name="viewport" content="width=device-width, initial-scale=1";
                title { (BRAND_NAME) " · login" }
                link rel="icon" type="image/svg+xml" href="/favicon.svg";
                link rel="preconnect" href="https://fonts.googleapis.com";
                link rel="preconnect" href="https://fonts.gstatic.com" crossorigin;
                link rel="stylesheet"
                    href="https://fonts.googleapis.com/css2?family=Inter:wght@400;500;600;700&family=JetBrains+Mono:wght@400;500&display=swap";
                style { (PreEscaped(LOGIN_CSS)) }
            }
            body.login-body {
                main.login-card {
                    div.login-brand {
                        img.login-logo src="/assets/crab-logo.svg" alt="logger-crab" width="48" height="48";
                        h1.login-title { (BRAND_NAME) }
                    }
                    @if token_was_invalid {
                        div.login-error { "Invalid token. Try again." }
                    }
                    form method="get" action="/" {
                        label for="token" { "Dashboard token" }
                        input type="password" id="token" name="token"
                            autocomplete="current-password"
                            autofocus
                            placeholder="paste your DASHBOARD_TOKEN value";
                        button type="submit" { "Sign in" }
                    }
                    p.login-hint {
                        "API consumers can use "
                        code { "Authorization: Bearer <token>" }
                        " instead — see "
                        a href="/docs" { "/docs" } "."
                    }
                }
                script { (PreEscaped(LOGIN_THEME_JS)) }
            }
        }
    };
    let mut response = (StatusCode::OK, Html(markup.into_string())).into_response();
    // Status is 200 (not 401) because we're rendering a real page; the
    // browser will display the form rather than its native error page.
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response
}

const LOGIN_CSS: &str = r#"
:root {
  --bg: #0d1117; --surface: #161b22; --surface2: #21262d; --text: #e6edf3;
  --dim: #7d8590; --muted: #484f58; --border: #30363d;
  --accent: #58a6ff; --err: #f85149;
}
body.light {
  --bg: #ffffff; --surface: #f6f8fa; --surface2: #eaeef2; --text: #1f2328;
  --dim: #656d76; --muted: #8c959f; --border: #d0d7de;
  --accent: #0969da; --err: #cf222e;
}
* { box-sizing: border-box; }
html, body { margin: 0; padding: 0; }
body.login-body {
  background: var(--bg);
  color: var(--text);
  font-family: "Inter", ui-sans-serif, system-ui, sans-serif;
  min-height: 100vh;
  display: flex;
  align-items: center;
  justify-content: center;
  padding: 24px;
}
.login-card {
  width: 100%;
  max-width: 380px;
  background: var(--surface);
  border: 1px solid var(--border);
  border-radius: 10px;
  padding: 32px;
  box-shadow: 0 16px 40px rgba(0,0,0,0.35);
}
.login-brand {
  display: flex;
  align-items: center;
  gap: 12px;
  margin-bottom: 28px;
}
.login-logo {
  display: block;
  filter: drop-shadow(0 0 6px color-mix(in srgb, var(--accent) 30%, transparent));
}
.login-title {
  margin: 0;
  font-size: 16px;
  font-weight: 600;
  letter-spacing: -0.01em;
}
.login-error {
  background: color-mix(in srgb, var(--err) 12%, transparent);
  border: 1px solid color-mix(in srgb, var(--err) 40%, var(--border));
  color: var(--err);
  padding: 10px 12px;
  border-radius: 6px;
  font-size: 13px;
  margin-bottom: 16px;
}
form { display: flex; flex-direction: column; gap: 8px; }
label {
  font-size: 11.5px;
  text-transform: uppercase;
  letter-spacing: 0.06em;
  color: var(--dim);
  font-weight: 500;
}
input[type="password"] {
  background: var(--bg);
  color: var(--text);
  border: 1px solid var(--border);
  border-radius: 6px;
  padding: 10px 12px;
  font-size: 14px;
  font-family: "JetBrains Mono", ui-monospace, monospace;
  transition: border-color 0.12s ease, box-shadow 0.12s ease;
}
input[type="password"]::placeholder { color: var(--muted); }
input[type="password"]:hover {
  border-color: color-mix(in srgb, var(--accent) 40%, var(--border));
}
input[type="password"]:focus {
  outline: none;
  border-color: var(--accent);
  box-shadow: 0 0 0 3px color-mix(in srgb, var(--accent) 24%, transparent);
}
button[type="submit"] {
  margin-top: 14px;
  padding: 10px 14px;
  border: 1px solid color-mix(in srgb, var(--accent) 60%, var(--border));
  background: var(--accent);
  color: white;
  border-radius: 6px;
  font-size: 13px;
  font-weight: 600;
  cursor: pointer;
  transition: filter 0.12s ease, transform 0.12s ease;
}
button[type="submit"]:hover { filter: brightness(1.08); }
button[type="submit"]:active { transform: translateY(1px); }
.login-hint {
  margin: 22px 0 0 0;
  font-size: 12px;
  color: var(--dim);
  line-height: 1.5;
}
.login-hint code {
  font-family: "JetBrains Mono", ui-monospace, monospace;
  font-size: 11px;
  background: var(--bg);
  border: 1px solid var(--border);
  padding: 1px 6px;
  border-radius: 4px;
  color: var(--text);
}
.login-hint a { color: var(--accent); text-decoration: none; }
.login-hint a:hover { text-decoration: underline; }
"#;

const LOGIN_THEME_JS: &str = r#"
(function() {
  var saved = localStorage.getItem('logger-crab-theme');
  if (saved === 'light') document.body.classList.add('light');
})();
"#;

/// Floating "Press ? for shortcuts" toast triggered by the `?` key.
/// Hidden by default; gets `.visible` class for ~4.5s when invoked.
fn render_kbd_toast() -> Markup {
    html! {
        div id="kbd-help-toast" class="kbd-toast" role="status" aria-hidden="true" {
            div.kbd-toast-title { "Keyboard shortcuts" }
            ul.kbd-toast-list {
                li { kbd { "j" } " / " kbd { "↓" } span.kbd-desc { "next row" } }
                li { kbd { "k" } " / " kbd { "↑" } span.kbd-desc { "previous row" } }
                li { kbd { "Enter" } span.kbd-desc { "expand current row" } }
                li { kbd { "/" } span.kbd-desc { "focus search" } }
                li { kbd { "r" } span.kbd-desc { "refresh" } }
                li { kbd { "Esc" } span.kbd-desc { "blur input / close modal" } }
                li { kbd { "?" } span.kbd-desc { "this help" } }
            }
        }
    }
}

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

/// Build a `/logs/download.ndjson?...` URL preserving the current filter
/// state so the download contains exactly what the user is looking at.
fn download_url(q: &DashboardQuery) -> String {
    let mut pairs = current_pairs(q);
    // Strip cursor so the download is the FILTERED set from the start, not
    // a single-page slice. Caller can still page through the dashboard normally.
    pairs.retain(|(k, _)| *k != "cursor");
    let qs = pairs_to_url(&pairs);
    if qs == "/" {
        "/logs/download.ndjson".to_string()
    } else {
        format!("/logs/download.ndjson{}", qs.trim_start_matches('/'))
    }
}

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

fn filter_url_override_u32(q: &DashboardQuery, key: &str, value: u32) -> String {
    let mut pairs: Vec<(&'static str, String)> =
        current_pairs(q).into_iter().filter(|(k, _)| *k != key).collect();
    if key == "limit" {
        pairs.push(("limit", value.to_string()));
    }
    pairs_to_url(&pairs)
}
