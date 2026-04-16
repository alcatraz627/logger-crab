use axum::extract::{Query, State};
use axum::response::Html;
use chrono::{DateTime, Utc};
use maud::{html, Markup, PreEscaped, DOCTYPE};
use serde::Deserialize;

use super::{AppState, BootInfo};
use crate::error::AppError;
use crate::models::{HotHealth, LogEvent, QueryParams};

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
    let markup =
        render(&q, &page.events, page.next_cursor.as_deref(), health.as_ref(), &state.boot);
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

fn render(
    q: &DashboardQuery,
    events: &[LogEvent],
    next_cursor: Option<&str>,
    health: Option<&HotHealth>,
    boot: &BootInfo,
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
                title { "logger-crab · dashboard" }
                link rel="icon" type="image/svg+xml"
                    href="data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 16 16'%3E%3Ctext y='14' font-size='14'%3E%F0%9F%A6%80%3C/text%3E%3C/svg%3E";
                link rel="preconnect" href="https://fonts.googleapis.com";
                link rel="preconnect" href="https://fonts.gstatic.com" crossorigin;
                link rel="stylesheet"
                    href="https://fonts.googleapis.com/css2?family=Inter:wght@400;500;600;700&family=JetBrains+Mono:wght@400;500&display=swap";
                style { (PreEscaped(CSS)) }
            }
            body {
                nav.lc-nav {
                    h1 { "🦀 logger-crab" }
                    a.active href="/" { "dashboard" }
                    a href="/api" { "API" }
                    a href="/docs" { "docs" }
                    a href="/health" { "health" }
                    div.health-chip {
                        @if health_ok {
                            span.dot.ok { } "hot ok"
                        } @else {
                            span.dot.err { } "hot down"
                        }
                    }
                    button.toggle id="theme-toggle" title="toggle light/dark" { "☾ / ☀" }
                }

                form.filters method="get" action="/" {
                    div.filter-group {
                        label { "request_id" }
                        input type="text" name="request_id"
                            value=[q.request_id.as_deref()] placeholder="req_abc_01";
                    }
                    div.filter-group {
                        label { "service" }
                        input type="text" name="service"
                            value=[q.service.as_deref()] placeholder="versable-api";
                    }
                    div.filter-group {
                        label { "env" }
                        input type="text" name="env"
                            value=[q.env.as_deref()] placeholder="prod";
                    }
                    div.filter-group {
                        label { "event prefix" }
                        input type="text" name="event_prefix"
                            value=[q.event_prefix.as_deref()] placeholder="pipeline.";
                    }
                    div.filter-group {
                        label { "level" }
                        select name="level" {
                            option value="" { "any" }
                            @for lvl in ["trace", "debug", "info", "warn", "error", "fatal"] {
                                option value=(lvl) selected[q.level.as_deref() == Some(lvl)] { (lvl) }
                            }
                        }
                    }
                    div.filter-group.grow {
                        label { "full-text search" }
                        input type="text" name="q"
                            value=[q.q.as_deref()] placeholder="message or payload…";
                    }
                    div.filter-group.actions {
                        label { "\u{00a0}" }
                        div.btn-row {
                            select name="limit" {
                                @for n in [50u32, 100, 250, 500] {
                                    option value=(n) selected[q.limit.unwrap_or(100) == n] { (n) "/page" }
                                }
                            }
                            button type="submit" { "apply" }
                            a.reset href="/" { "reset" }
                        }
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
                    @if let Some(cursor) = next_cursor {
                        a.pager href=(build_next_url(q, cursor)) { "older →" }
                    } @else {
                        span.pager.disabled { "no older" }
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

                (render_footer(boot, health))

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

fn render_footer(boot: &BootInfo, health: Option<&HotHealth>) -> Markup {
    html! {
        footer.lc-footer {
            div.ft-col {
                div.ft-title { "build" }
                div.ft-row { span.ft-k { "git" } span.ft-v.mono { (boot.git_sha) } }
                div.ft-row { span.ft-k { "built" } span.ft-v { (fmt_build_time(boot.build_time_unix)) } }
                div.ft-row { span.ft-k { "uptime" } span.ft-v { (fmt_uptime(boot.uptime_seconds())) } }
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
                div.ft-row {
                    span.ft-k { "started" }
                    span.ft-v { (boot.started_at.format("%Y-%m-%d %H:%M UTC").to_string()) }
                }
            }
            div.ft-col {
                div.ft-title { "config" }
                div.ft-row { span.ft-k { "hot" } span.ft-v.mono { (boot.hot_store) } }
                div.ft-row { span.ft-k { "cold" } span.ft-v.mono { (boot.cold_store) } }
                div.ft-row {
                    span.ft-k { "ingest auth" }
                    @if boot.has_ingest_token {
                        span.ft-v.ok { "set" }
                    } @else {
                        span.ft-v.warn { "unset" }
                    }
                }
                div.ft-row {
                    span.ft-k { "dash auth" }
                    @if boot.has_dashboard_token {
                        span.ft-v.ok { "set" }
                    } @else {
                        span.ft-v.warn { "unset" }
                    }
                }
                @if let Some(bucket) = &boot.s3_bucket {
                    div.ft-row { span.ft-k { "bucket" } span.ft-v.mono { (bucket) } }
                }
                div.ft-row { span.ft-k { "region" } span.ft-v.mono { (boot.aws_region) } }
            }
            div.ft-col {
                div.ft-title { "health" }
                @if let Some(h) = health {
                    div.ft-row {
                        span.ft-k { "hot" }
                        @if h.ok { span.ft-v.ok { "● online" } }
                        @else { span.ft-v.err { "● offline" } }
                    }
                    div.ft-row { span.ft-k { "rows" } span.ft-v.mono { (h.rows) } }
                    @if let Some(ts) = h.oldest_ts {
                        div.ft-row {
                            span.ft-k { "oldest" }
                            span.ft-v { (fmt_relative(ts)) }
                        }
                    }
                } @else {
                    div.ft-row { span.ft-k { "hot" } span.ft-v.err { "● offline" } }
                }
                div.ft-row {
                    span.ft-k { "db" }
                    span.ft-v.mono title=(boot.database_url_masked) {
                        (truncate(&boot.database_url_masked, 24))
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
