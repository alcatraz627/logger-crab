use axum::extract::{Query, State};
use axum::response::Html;
use chrono::{DateTime, Utc};
use maud::{html, Markup, PreEscaped, DOCTYPE};
use serde::Deserialize;

use super::auth::{AuthRole, TokenRecord};
use super::nav::{
    icon_box, icon_branch, icon_check, icon_globe, icon_hash, icon_search, icon_x, render_nav,
    Active, BRAND_NAME, GITHUB_URL,
};
use super::{AppState, BootInfo};
use crate::error::AppError;
use crate::models::{ColdHealth, HotHealth, LogEvent, QueryParams};

const LEVELS: &[&str] = &["trace", "debug", "info", "warn", "error", "fatal"];
const PAGE_SIZES: &[u32] = &[50, 100, 250, 500];

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
}

pub async fn get_dashboard(
    State(state): State<AppState>,
    Query(q): Query<DashboardQuery>,
) -> Result<Html<String>, AppError> {
    let page_size = q.limit.unwrap_or(100).clamp(10, 500);
    let params = QueryParams {
        request_id: not_empty(&q.request_id),
        user_id: None,
        session_id: None,
        service: not_empty(&q.service),
        env: not_empty(&q.env),
        event_prefix: not_empty(&q.event_prefix),
        min_severity: not_empty(&q.level).as_deref().map(level_to_min_severity),
        since: None,
        until: None,
        fts: not_empty(&q.q),
        limit: page_size,
        cursor: not_empty(&q.cursor),
    };
    let page = state.hot.query(&params).await?;
    let health = state.hot.health().await.ok();
    let cold_health = state.cold.health().await.ok();
    let markup = render(
        &q,
        &page.events,
        page.next_cursor.as_deref(),
        health.as_ref(),
        cold_health.as_ref(),
        &state.boot,
        &state.ingest_tokens,
        &state.config_warnings,
    );
    Ok(Html(markup.into_string()))
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

fn severity_class(n: u8) -> &'static str {
    match n {
        1..=4 => "lvl-trace",
        5..=8 => "lvl-debug",
        9..=12 => "lvl-info",
        13..=16 => "lvl-warn",
        17..=20 => "lvl-error",
        _ => "lvl-fatal",
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

fn render(
    q: &DashboardQuery,
    events: &[LogEvent],
    next_cursor: Option<&str>,
    health: Option<&HotHealth>,
    cold: Option<&ColdHealth>,
    boot: &BootInfo,
    consumers: &[TokenRecord],
    config_warnings: &[String],
) -> Markup {
    let total = events.len();
    let health_ok = health.map(|h| h.ok).unwrap_or(false);
    let row_count = health.map(|h| h.rows).unwrap_or(0);
    let oldest = health.and_then(|h| h.oldest_ts);

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
            }
            body {
                (render_nav(Active::Dashboard, Some(health_ok)))

                form.filters method="get" action="/" {
                    div.filter-group {
                        label { (icon_hash()) "request_id" }
                        input type="text" name="request_id"
                            value=[q.request_id.as_deref()] placeholder="req_abc_01";
                    }
                    div.filter-group {
                        label { (icon_box()) "service" }
                        input type="text" name="service"
                            value=[q.service.as_deref()] placeholder="versable-api";
                    }
                    div.filter-group {
                        label { (icon_globe()) "env" }
                        input type="text" name="env"
                            value=[q.env.as_deref()] placeholder="prod";
                    }
                    div.filter-group {
                        label { (icon_branch()) "event prefix" }
                        input type="text" name="event_prefix"
                            value=[q.event_prefix.as_deref()] placeholder="pipeline.";
                    }
                    div.filter-group.grow {
                        label { (icon_search()) "full-text search" }
                        input type="text" name="q"
                            value=[q.q.as_deref()] placeholder="message or payload…";
                    }
                    div.filter-group.actions {
                        label { "\u{00a0}" }
                        div.btn-row {
                            button.btn-apply type="submit" title="apply filters (Enter)" {
                                (icon_check()) span { "Apply" }
                            }
                            a.btn-reset href="/" title="clear all filters" {
                                (icon_x()) span { "Reset" }
                            }
                        }
                    }

                    div.filter-row {
                        (level_pill_filter(q))
                        div.grow { }
                        (page_size_selector(q))
                    }
                }

                div.toolbar {
                    div.count {
                        span.num { (row_count) } " total · "
                        span.num { (total) } " shown"
                        @if let Some(ts) = oldest {
                            span.dim { " · oldest " (fmt_relative(ts)) }
                        }
                    }
                    (render_active_filters(q))
                    div.grow { }
                    span.hint title="Press / to focus search" { "press " kbd { "/" } " to search" }
                    div.pager-group {
                        @if q.cursor.is_some() {
                            a.pager.pager-newer href=(filter_url_remove(q, "cursor"))
                                title="jump to newest events" {
                                "← newest"
                            }
                        } @else {
                            span.pager.pager-newer.disabled title="already at newest" { "← newest" }
                        }
                        @if let Some(cursor) = next_cursor {
                            a.pager.pager-older href=(build_next_url(q, cursor))
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
                            div.empty-title { "No events match the current filters" }
                            div.empty-hint {
                                "Try resetting filters, or "
                                code { "POST /ingest" } " to seed events. "
                                "Boot with " code { "SEED_ON_BOOT=1" } " to auto-populate dummy data."
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
                                        td.ts {
                                            span.ts-rel title=(e.ts.format("%Y-%m-%d %H:%M:%S%.3f UTC").to_string()) {
                                                (fmt_relative(e.ts))
                                            }
                                            span.ts-abs { (e.ts.format("%H:%M:%S%.3f").to_string()) }
                                        }
                                        td {
                                            span class=(format!("lvl-pill {}", severity_class(e.severity_number))) {
                                                (e.severity_text.to_ascii_uppercase())
                                            }
                                        }
                                        td {
                                            @if let Some(svc) = &e.service {
                                                a class=(format!("chip chip-link svc-c{}", hash_color(svc)))
                                                    href=(filter_url_override(q, "service", svc))
                                                    title=(format!("filter by service: {svc}")) {
                                                    (svc)
                                                }
                                            } @else {
                                                span.dim { "—" }
                                            }
                                        }
                                        td {
                                            @if let Some(env) = &e.env {
                                                a class=(format!("env-pill env-link env-{}", env_class(env)))
                                                    href=(filter_url_override(q, "env", env))
                                                    title=(format!("filter by env: {env}")) {
                                                    (env.to_ascii_uppercase())
                                                }
                                            } @else {
                                                span.dim { "—" }
                                            }
                                        }
                                        td.evt {
                                            span class=(format!("evt-ns ns-c{}", hash_color(ns))) { (ns) }
                                            @if !leaf.is_empty() {
                                                span.evt-sep { "." }
                                                span.evt-leaf { (leaf) }
                                            }
                                        }
                                        td {
                                            a.rid href=(filter_url_override(q, "request_id", &e.request_id))
                                                title=(format!("filter by request_id: {}", e.request_id)) {
                                                (&e.request_id[..e.request_id.len().min(14)])
                                            }
                                        }
                                        td.msg {
                                            details {
                                                summary {
                                                    (e.message.as_deref().unwrap_or(""))
                                                }
                                                div.payload {
                                                    pre { code {
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

fn build_next_url(q: &DashboardQuery, cursor: &str) -> String {
    let mut pairs = current_pairs(q);
    pairs.push(("cursor", cursor.to_string()));
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
    if let Some(v) = &q.request_id {
        pairs.push(("request_id", v.clone()));
    }
    if let Some(v) = &q.service {
        pairs.push(("service", v.clone()));
    }
    if let Some(v) = &q.env {
        pairs.push(("env", v.clone()));
    }
    if let Some(v) = &q.event_prefix {
        pairs.push(("event_prefix", v.clone()));
    }
    if let Some(v) = &q.level {
        pairs.push(("level", v.clone()));
    }
    if let Some(v) = &q.q {
        pairs.push(("q", v.clone()));
    }
    if let Some(v) = q.limit {
        pairs.push(("limit", v.to_string()));
    }
    pairs
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

fn render_active_filters(q: &DashboardQuery) -> Markup {
    let active: Vec<(&'static str, String, &'static str)> = [
        ("request_id", q.request_id.clone(), "req"),
        ("service", q.service.clone(), "svc"),
        ("env", q.env.clone(), "env"),
        ("event_prefix", q.event_prefix.clone(), "evt"),
        ("level", q.level.clone(), "lvl"),
        ("q", q.q.clone(), "q"),
    ]
    .into_iter()
    .filter_map(|(k, v, label)| v.map(|v| (k, v, label)))
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
                        @else { span.ft-v.warn { "○ unconfigured" } }
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
                } @else {
                    div.ft-row { span.ft-k { "status" } span.ft-v.warn { "○ unavailable" } }
                }
                @if let Some(bucket) = &boot.s3_bucket {
                    div.ft-row { span.ft-k { "bucket" } span.ft-v.mono { (bucket) } }
                } @else {
                    div.ft-row { span.ft-k { "bucket" } span.ft-v.dim { "—" } }
                }
            }
        }
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
