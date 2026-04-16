use axum::extract::{Query, State};
use axum::response::Html;
use maud::{DOCTYPE, Markup, PreEscaped, html};
use serde::Deserialize;

use super::AppState;
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
}

pub async fn get_dashboard(
    State(state): State<AppState>,
    Query(q): Query<DashboardQuery>,
) -> Result<Html<String>, AppError> {
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
        limit: q.limit.unwrap_or(100).min(500),
        cursor: None,
    };
    let page = state.hot.query(&params).await?;
    let health = state.hot.health().await.ok();
    let markup = render(&q, &page.events, health.as_ref());
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

const CSS: &str = r#"
:root {
  --bg: #0e1116; --surface: #161b22; --text: #e6edf3; --dim: #7d8590;
  --border: #30363d; --accent: #58a6ff; --warn: #d29922; --err: #f85149;
  --ok: #3fb950;
}
body.light {
  --bg: #ffffff; --surface: #f6f8fa; --text: #1f2328; --dim: #656d76;
  --border: #d0d7de; --accent: #0969da; --warn: #9a6700; --err: #cf222e;
  --ok: #1a7f37;
}
* { box-sizing: border-box; }
body { margin: 0; font-family: ui-monospace, SFMono-Regular, Menlo, monospace;
       background: var(--bg); color: var(--text); font-size: 13px; }
header { padding: 12px 20px; border-bottom: 1px solid var(--border);
         display: flex; align-items: center; gap: 16px; background: var(--surface); }
header h1 { margin: 0; font-size: 16px; font-weight: 600; }
header .dim { color: var(--dim); font-size: 12px; }
.badge { padding: 2px 8px; border-radius: 4px; font-size: 11px;
         border: 1px solid var(--border); }
.badge.ok { color: var(--ok); border-color: var(--ok); }
.badge.err { color: var(--err); border-color: var(--err); }
.toggle { margin-left: auto; background: transparent; color: var(--text);
          border: 1px solid var(--border); padding: 4px 10px; border-radius: 4px;
          cursor: pointer; font-family: inherit; font-size: 12px; }
form.filters { padding: 12px 20px; display: flex; flex-wrap: wrap; gap: 8px;
               border-bottom: 1px solid var(--border); background: var(--surface); }
form.filters input, form.filters select {
  background: var(--bg); color: var(--text); border: 1px solid var(--border);
  padding: 4px 8px; border-radius: 4px; font-family: inherit; font-size: 12px;
}
form.filters button { background: var(--accent); color: white; border: 0;
                      padding: 4px 14px; border-radius: 4px; cursor: pointer;
                      font-family: inherit; font-size: 12px; }
form.filters a { color: var(--dim); align-self: center; font-size: 11px;
                 text-decoration: none; }
.refresh-hint { color: var(--dim); font-size: 11px; margin-left: 8px; align-self: center; }
table { width: 100%; border-collapse: collapse; }
th { text-align: left; padding: 6px 10px; font-weight: 600; color: var(--dim);
     border-bottom: 1px solid var(--border); position: sticky; top: 0;
     background: var(--surface); }
td { padding: 6px 10px; border-bottom: 1px solid var(--border);
     vertical-align: top; }
tr.evrow:hover { background: var(--surface); }
.lvl { display: inline-block; padding: 1px 6px; border-radius: 3px;
       font-size: 10px; font-weight: 600; text-transform: uppercase; }
.lvl-trace { color: var(--dim); }
.lvl-debug { color: var(--accent); }
.lvl-info  { color: var(--ok); }
.lvl-warn  { color: var(--warn); }
.lvl-error { color: var(--err); }
.lvl-fatal { color: white; background: var(--err); padding: 1px 8px; }
.ts { color: var(--dim); white-space: nowrap; }
.rid { color: var(--accent); cursor: pointer; }
.rid:hover { text-decoration: underline; }
.evt { font-weight: 500; }
.msg { color: var(--dim); max-width: 360px; overflow: hidden;
       text-overflow: ellipsis; white-space: nowrap; }
details { margin: 0; }
details summary { cursor: pointer; list-style: none; }
details summary::-webkit-details-marker { display: none; }
details[open] td.msg { white-space: normal; max-width: none; }
.payload { background: var(--bg); border: 1px solid var(--border);
           border-radius: 4px; padding: 8px; margin-top: 6px;
           white-space: pre-wrap; word-break: break-all; font-size: 11px;
           color: var(--dim); }
.empty { padding: 40px; text-align: center; color: var(--dim); }
"#;

const TOGGLE_JS: &str = r#"
(function() {
  var saved = localStorage.getItem('logger-crab-theme');
  if (saved === 'light') document.body.classList.add('light');
  document.getElementById('theme-toggle').addEventListener('click', function() {
    document.body.classList.toggle('light');
    localStorage.setItem('logger-crab-theme',
      document.body.classList.contains('light') ? 'light' : 'dark');
  });
})();
"#;

fn render(q: &DashboardQuery, events: &[LogEvent], health: Option<&HotHealth>) -> Markup {
    let total = events.len();
    let health_ok = health.map(|h| h.ok).unwrap_or(false);
    let row_count = health.map(|h| h.rows).unwrap_or(0);

    html! {
        (DOCTYPE)
        html {
            head {
                meta charset="utf-8";
                meta name="viewport" content="width=device-width, initial-scale=1";
                meta http-equiv="refresh" content="15";
                title { "logger-crab · dashboard" }
                style { (PreEscaped(CSS)) }
            }
            body {
                header {
                    h1 { "🦀 logger-crab" }
                    @if health_ok {
                        span.badge.ok { "hot ok" }
                    } @else {
                        span.badge.err { "hot down" }
                    }
                    span.dim { (row_count) " total · showing " (total) }
                    span.refresh-hint { "auto-refresh 15s" }
                    button.toggle id="theme-toggle" { "☾ / ☀" }
                }

                form.filters method="get" action="/" {
                    input type="text" name="request_id" placeholder="request_id"
                        value=[q.request_id.as_deref()];
                    input type="text" name="service" placeholder="service"
                        value=[q.service.as_deref()];
                    input type="text" name="env" placeholder="env"
                        value=[q.env.as_deref()];
                    input type="text" name="event_prefix" placeholder="event prefix (e.g. pipeline.)"
                        value=[q.event_prefix.as_deref()];
                    select name="level" {
                        option value="" { "any level" }
                        @for lvl in ["trace", "debug", "info", "warn", "error", "fatal"] {
                            option value=(lvl) selected[q.level.as_deref() == Some(lvl)] { (lvl) }
                        }
                    }
                    input type="text" name="q" placeholder="full-text search"
                        value=[q.q.as_deref()];
                    button type="submit" { "filter" }
                    a href="/" { "reset" }
                }

                @if events.is_empty() {
                    div.empty {
                        "No events match. "
                        "Try POST /ingest with Bearer token to seed data, "
                        "or clear filters."
                    }
                } @else {
                    table {
                        thead {
                            tr {
                                th { "time" }
                                th { "lvl" }
                                th { "service" }
                                th { "event" }
                                th { "request_id" }
                                th { "message / payload" }
                            }
                        }
                        tbody {
                            @for e in events {
                                tr.evrow {
                                    td.ts {
                                        (e.ts.format("%H:%M:%S%.3f").to_string())
                                    }
                                    td {
                                        span class=(format!("lvl {}", severity_class(e.severity_number))) {
                                            (e.severity_text)
                                        }
                                    }
                                    td { (e.service.as_deref().unwrap_or("—")) }
                                    td.evt { (e.event) }
                                    td {
                                        a.rid href=(format!("/?request_id={}", e.request_id)) {
                                            (&e.request_id[..e.request_id.len().min(12)])
                                        }
                                    }
                                    td.msg {
                                        details {
                                            summary {
                                                (e.message.as_deref().unwrap_or(""))
                                            }
                                            div.payload {
                                                (serde_json::to_string_pretty(&e.payload)
                                                    .unwrap_or_else(|_| "—".into()))
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }

                script { (PreEscaped(TOGGLE_JS)) }
            }
        }
    }
}
