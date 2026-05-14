use axum::extract::State;
use axum::http::{HeaderMap, Uri};
use axum::response::{Html, IntoResponse, Response};
use maud::{html, Markup, PreEscaped, DOCTYPE};

use super::nav::{render_nav, Active, BRAND_NAME, NAV_CSS, TOGGLE_JS};
use super::AppState;

pub async fn get_docs(
    State(state): State<AppState>,
    uri: Uri,
    headers: HeaderMap,
) -> Response {
    if let Err(login) = super::gate_html(&headers, &state, &uri) {
        return login;
    }
    let markup = render(&state);
    Html(markup.into_string()).into_response()
}

const CSS: &str = r#"
* { box-sizing: border-box; }
body {
  margin: 0; padding: 0; background: var(--bg); color: var(--text);
  font-family: Inter, ui-sans-serif, system-ui, sans-serif;
  font-size: 14px; line-height: 1.6;
}
code, pre, .mono { font-family: "JetBrains Mono", ui-monospace, SFMono-Regular, Menlo, monospace; }
main { max-width: 860px; margin: 0 auto; padding: 32px 24px 80px; }
h1.title { font-size: 32px; margin: 0 0 8px; letter-spacing: -0.02em; font-weight: 700; }
.lede { color: var(--dim); font-size: 16px; margin: 0 0 32px; }
h2 {
  font-size: 20px; margin: 40px 0 12px; padding-bottom: 6px;
  border-bottom: 1px solid var(--border); letter-spacing: -0.01em;
}
h3 { font-size: 15px; margin: 24px 0 8px; color: var(--accent); }
p { margin: 0 0 12px; }
code {
  background: var(--surface2); padding: 1px 6px; border-radius: 4px;
  font-size: 12.5px; border: 1px solid var(--border);
}
pre {
  background: var(--surface); padding: 14px 16px; border-radius: 8px;
  border: 1px solid var(--border); overflow-x: auto; font-size: 12.5px;
  line-height: 1.5;
}
pre code { background: transparent; padding: 0; border: 0; }
table { width: 100%; border-collapse: collapse; margin: 12px 0; font-size: 13px; }
th, td { padding: 8px 12px; text-align: left; border-bottom: 1px solid var(--border); }
th { color: var(--dim); font-weight: 600; font-size: 12px; text-transform: uppercase;
     letter-spacing: 0.04em; background: var(--surface); }
tr:hover td { background: var(--surface); }
.endpoint-row td:first-child { font-family: "JetBrains Mono", ui-monospace, monospace; }
.method {
  display: inline-block; padding: 2px 8px; border-radius: 4px;
  font-size: 11px; font-weight: 700; text-transform: uppercase;
  margin-right: 8px; min-width: 48px; text-align: center;
}
.method.get { background: rgba(88, 166, 255, 0.15); color: var(--accent); }
.method.post { background: rgba(63, 185, 80, 0.15); color: var(--ok); }
.callout {
  background: var(--surface); border-left: 3px solid var(--accent);
  padding: 12px 16px; border-radius: 0 6px 6px 0; margin: 16px 0;
}
.callout.warn { border-left-color: var(--warn); }
.ascii {
  background: var(--surface); padding: 16px; border-radius: 8px;
  border: 1px solid var(--border); white-space: pre; overflow-x: auto;
  font-size: 12px; line-height: 1.4; color: var(--dim);
}
.link-card {
  display: block; background: var(--surface); border: 1px solid var(--border);
  padding: 14px 18px; border-radius: 8px; text-decoration: none;
  color: var(--text); transition: all 0.15s; margin-bottom: 10px;
}
.link-card:hover { border-color: var(--accent); transform: translateY(-1px); }
.link-card .title { font-weight: 600; margin-bottom: 2px; font-size: 14px; }
.link-card .sub { color: var(--dim); font-size: 12.5px; }

/* ─── Architecture diagram ──────────────────────────────────────────── */
.arch-diagram {
  display: flex;
  flex-direction: column;
  gap: 6px;
  margin: 18px 0;
  padding: 18px;
  background: var(--surface);
  border: 1px solid var(--border);
  border-radius: 10px;
}
.arch-row {
  display: grid;
  grid-template-columns: 90px 1fr;
  gap: 12px;
  align-items: center;
}
.arch-tier-label {
  font-size: 10.5px;
  font-weight: 600;
  text-transform: uppercase;
  letter-spacing: 0.06em;
  color: var(--dim);
  text-align: right;
}
.arch-boxes {
  display: flex;
  gap: 8px;
  flex-wrap: wrap;
}
.arch-box {
  flex: 1 1 0;
  min-width: 110px;
  padding: 10px 12px;
  border-radius: 8px;
  background: var(--bg);
  border: 1px solid var(--border);
}
.arch-box-emitter { border-color: color-mix(in srgb, var(--accent) 30%, var(--border)); }
.arch-box-server {
  background: color-mix(in srgb, var(--accent) 8%, var(--bg));
  border-color: color-mix(in srgb, var(--accent) 50%, var(--border));
}
.arch-box-hot { border-color: color-mix(in srgb, var(--ok) 40%, var(--border)); }
.arch-box-cold { border-color: color-mix(in srgb, var(--warn) 40%, var(--border)); }
.arch-box-title { font-weight: 600; font-size: 13px; margin-bottom: 2px; }
.arch-box-sub { font-size: 11.5px; color: var(--dim); }
.arch-arrow {
  display: flex;
  align-items: center;
  justify-content: center;
  gap: 10px;
  padding: 6px 0;
}
.arch-arrow-label { font-size: 11px; color: var(--dim); font-family: "JetBrains Mono", ui-monospace, monospace; }
.arch-arrow-glyph { font-size: 18px; color: var(--muted); line-height: 1; }

/* ─── Request-id flow diagram ──────────────────────────────────────── */
.flow-diagram {
  display: flex;
  flex-direction: column;
  gap: 0;
  margin: 18px 0;
  padding: 16px;
  background: var(--surface);
  border: 1px solid var(--border);
  border-radius: 10px;
}
.flow-step {
  display: grid;
  grid-template-columns: 28px 1fr;
  gap: 12px;
  padding: 10px 0;
  position: relative;
}
.flow-step:not(:last-of-type)::after {
  content: "";
  position: absolute;
  left: 13px;
  top: 36px;
  bottom: -8px;
  width: 2px;
  background: color-mix(in srgb, var(--accent) 40%, var(--border));
}
.flow-step-num {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 28px;
  height: 28px;
  border-radius: 50%;
  background: var(--accent);
  color: white;
  font-weight: 700;
  font-size: 12px;
  z-index: 1;
}
.flow-step-title { font-weight: 600; font-size: 13px; margin-bottom: 4px; }
.flow-step-detail {
  display: block;
  background: var(--bg);
  padding: 5px 10px;
  border-radius: 4px;
  font-size: 12px;
  color: var(--dim);
  border: 1px solid var(--border);
}
.flow-result {
  margin-top: 10px;
  padding: 10px 14px;
  background: color-mix(in srgb, var(--ok) 8%, var(--surface));
  border: 1px solid color-mix(in srgb, var(--ok) 35%, var(--border));
  border-radius: 6px;
  font-size: 12.5px;
  color: var(--text);
}

/* ─── Cycle (rotation) diagram ─────────────────────────────────────── */
.cycle-diagram {
  display: flex;
  align-items: stretch;
  justify-content: space-between;
  gap: 6px;
  margin: 18px 0;
  padding: 18px;
  background: var(--surface);
  border: 1px solid var(--border);
  border-radius: 10px;
  overflow-x: auto;
}
.cycle-step {
  flex: 1;
  min-width: 130px;
  padding: 12px;
  border-radius: 8px;
  background: var(--bg);
  border: 1px solid var(--border);
}
.cycle-step-label {
  font-size: 10.5px;
  font-weight: 700;
  color: var(--dim);
  letter-spacing: 0.08em;
  margin-bottom: 6px;
}
.cycle-step-name { font-weight: 600; font-size: 13px; margin-bottom: 4px; }
.cycle-step-detail { font-size: 11.5px; color: var(--dim); }
.cycle-step-detail code { background: var(--surface2); border-color: var(--border); }
.cycle-arrow {
  display: flex;
  align-items: center;
  font-size: 18px;
  color: var(--muted);
  flex-shrink: 0;
}
.cycle-step-1 { border-color: color-mix(in srgb, var(--accent) 30%, var(--border)); }
.cycle-step-2 { border-color: color-mix(in srgb, var(--accent2) 30%, var(--border)); }
.cycle-step-3 { border-color: color-mix(in srgb, var(--warn) 35%, var(--border)); }
.cycle-step-4 { border-color: color-mix(in srgb, var(--ok) 35%, var(--border)); }

/* ─── Severity scale ────────────────────────────────────────────────── */
.severity-scale {
  display: flex;
  flex-direction: column;
  gap: 4px;
  margin: 18px 0;
  padding: 14px;
  background: var(--surface);
  border: 1px solid var(--border);
  border-radius: 10px;
}
.sev-row {
  display: grid;
  grid-template-columns: 36px 60px 1fr;
  align-items: center;
  gap: 10px;
  padding: 8px 12px;
  border-radius: 6px;
  background: var(--bg);
}
.sev-num {
  font-family: "JetBrains Mono", ui-monospace, monospace;
  font-size: 11px;
  color: var(--dim);
  text-align: right;
}
.sev-name {
  font-family: "JetBrains Mono", ui-monospace, monospace;
  font-size: 11px;
  font-weight: 700;
  letter-spacing: 0.04em;
}
.sev-hint { font-size: 12.5px; color: var(--dim); }
.sev-row-trace .sev-name { color: var(--muted); }
.sev-row-debug .sev-name { color: var(--dim); }
.sev-row-info  .sev-name { color: var(--ok); }
.sev-row-warn  .sev-name { color: var(--warn); }
.sev-row-error .sev-name { color: var(--err); }
.sev-row-fatal .sev-name { color: var(--err); }
.sev-row-fatal {
  background: color-mix(in srgb, var(--err) 5%, var(--bg));
  border: 1px solid color-mix(in srgb, var(--err) 30%, var(--border));
}

/* ─── Auth comparison cards ───────────────────────────────────────── */
.auth-compare {
  display: grid;
  grid-template-columns: 1fr 1fr;
  gap: 14px;
  margin: 16px 0;
}
.auth-card {
  padding: 16px;
  background: var(--surface);
  border: 1px solid var(--border);
  border-radius: 10px;
}
.auth-card-title {
  font-size: 14px;
  font-weight: 700;
  margin-bottom: 4px;
  color: var(--accent);
}
.auth-card-sub { font-size: 12px; color: var(--dim); margin-bottom: 12px; }
.auth-card-list { margin: 0; padding-left: 18px; }
.auth-card-list li { font-size: 12.5px; line-height: 1.6; margin-bottom: 4px; }
@media (max-width: 640px) {
  .auth-compare { grid-template-columns: 1fr; }
  .arch-row { grid-template-columns: 1fr; }
  .arch-tier-label { text-align: left; }
}

/* ─── Callout overrides for the new tip variant ─────────────────── */
.callout-tip {
  display: flex;
  gap: 10px;
  align-items: flex-start;
  background: color-mix(in srgb, var(--accent) 6%, var(--surface));
  border-left-color: var(--accent);
  padding: 12px 16px;
  border-radius: 0 6px 6px 0;
  margin: 16px 0;
}
.callout-icon {
  color: var(--accent);
  font-weight: 700;
  font-size: 16px;
  line-height: 1;
  margin-top: 2px;
}
.callout-body { flex: 1; }
"#;

fn render(state: &AppState) -> Markup {
    let boot = &*state.boot;

    html! {
        (DOCTYPE)
        html {
            head {
                meta charset="utf-8";
                meta name="viewport" content="width=device-width, initial-scale=1";
                title { (BRAND_NAME) " · docs" }
                link rel="icon" type="image/svg+xml" href="/favicon.svg";
                link rel="preconnect" href="https://fonts.googleapis.com";
                link rel="preconnect" href="https://fonts.gstatic.com" crossorigin;
                link rel="stylesheet"
                    href="https://fonts.googleapis.com/css2?family=Inter:wght@400;500;600;700&family=JetBrains+Mono:wght@400;500&display=swap";
                // Prism syntax highlighting (theme switches with body class)
                link rel="stylesheet"
                    href="https://cdnjs.cloudflare.com/ajax/libs/prism/1.29.0/themes/prism-tomorrow.min.css";
                style { (PreEscaped(NAV_CSS)) }
                style { (PreEscaped(CSS)) }
                style { (PreEscaped(PRISM_OVERRIDES_CSS)) }
            }
            body {
                (render_nav(Active::Docs, None))

                main {
                    h1.title { "logger-crab docs" }
                    p.lede {
                        "A self-hosted centralized log ingest + query service. "
                        "Hot tier on SQLite (last 24–48h), cold tier on S3 NDJSON, "
                        "OpenTelemetry-flavored ingest envelope."
                    }

                    h2 id="overview" { "What is logger-crab?" }
                    p {
                        "Imagine your stack today: a Next.js frontend, a FastAPI backend, a Redis "
                        "worker, a few cron jobs. Each writes logs to its own place — Vercel logs, "
                        "Render logs, stdout, files. When something goes wrong with a single user's "
                        "request, you have to open four log viewers and grep manually for the right "
                        "events. That's the pain logger-crab solves."
                    }
                    p {
                        "Each emitter sends events to logger-crab tagged with a "
                        code { "request_id" } " (minted at the edge, threaded through every hop). "
                        "logger-crab indexes them in SQLite, displays them in a single dashboard, "
                        "and rotates older events to S3 NDJSON for archival. You filter by "
                        code { "request_id" } " and see the full journey of one user's request "
                        "across every service it touched."
                    }

                    h2 id="quickstart" { "Quickstart — your first event" }
                    div.callout.callout-tip {
                        div.callout-icon { "▸" }
                        div.callout-body {
                            "If logger-crab is already deployed (you're reading this on it), then sending "
                            "your first event is one curl command. Replace " code { "$TOKEN" }
                            " with one of your " code { "INGEST_TOKEN_<NAME>" } " values "
                            "(see " a href="#auth" { "Authentication" } ")."
                        }
                    }
                    pre { code class="language-bash" { (QUICKSTART_CURL) } }
                    p {
                        "Then visit the " a href="/" { "dashboard" } " — your event appears within a "
                        "second. Click its " code { "smoke.test" } " row to expand the payload. "
                        "Try filtering by " code { "service=demo" } " or pasting the request_id "
                        "into the filter — same event surfaces, no other noise."
                    }

                    h2 { "Architecture" }
                    p {
                        "Services emit logs through thin client libraries (TS + Python) "
                        "to the log-server over HTTP. The server writes to the "
                        a href="#hot-store" { "hot store" } " (SQLite WAL) for fast queries, and "
                        "periodically rotates older rows to the "
                        a href="#cold-store" { "cold store" } " (S3 NDJSON) for long-term archive."
                    }
                    div.arch-diagram {
                        div.arch-row {
                            div.arch-tier-label { "emitters" }
                            div.arch-boxes {
                                div.arch-box.arch-box-emitter { div.arch-box-title { "Next.js" } div.arch-box-sub { "TS sink" } }
                                div.arch-box.arch-box-emitter { div.arch-box-title { "FastAPI" } div.arch-box-sub { "Python sink" } }
                                div.arch-box.arch-box-emitter { div.arch-box-title { "Worker" } div.arch-box-sub { "Node / Python" } }
                                div.arch-box.arch-box-emitter { div.arch-box-title { "Cron" } div.arch-box-sub { "scheduled" } }
                            }
                        }
                        div.arch-arrow {
                            span.arch-arrow-label { "POST /ingest · " code { "Authorization: Bearer" } " · " code { "X-Request-ID" } }
                            span.arch-arrow-glyph { "↓" }
                        }
                        div.arch-row {
                            div.arch-tier-label { "log-server" }
                            div.arch-boxes {
                                div.arch-box.arch-box-server {
                                    div.arch-box-title { "axum · sqlx · maud" }
                                    div.arch-box-sub { "rust on render · port 10000" }
                                }
                            }
                        }
                        div.arch-arrow {
                            span.arch-arrow-label { "write hot · rotate cold" }
                            span.arch-arrow-glyph { "↓" }
                        }
                        div.arch-row {
                            div.arch-tier-label { "storage" }
                            div.arch-boxes {
                                div.arch-box.arch-box-hot {
                                    div.arch-box-title { "SQLite hot tier" }
                                    div.arch-box-sub { "WAL · 24-48h · indexed FTS" }
                                }
                                div.arch-box.arch-box-cold {
                                    div.arch-box-title { "S3 cold tier" }
                                    div.arch-box-sub { "NDJSON.gz · {env}/{svc}/YYYY/MM/DD/HH" }
                                }
                            }
                        }
                    }

                    h2 id="concepts" { "Core concepts" }
                    p {
                        "Five fields define an event. Get these right and the dashboard becomes "
                        "useful immediately; get them wrong and it's a soup of disconnected lines."
                    }
                    table {
                        thead {
                            tr { th { "Field" } th { "Required" } th { "What it means" } }
                        }
                        tbody {
                            tr { td { code { "event" } }
                                 td { "yes" }
                                 td {
                                     "Dotted lowercase name like " code { "pipeline.start" } " or "
                                     code { "auth.login.fail" } ". Stable identifier — never rename "
                                     "after shipping (queries depend on it)."
                                 } }
                            tr { td { code { "request_id" } }
                                 td { "no (recommended)" }
                                 td {
                                     "The correlation ID. One request → one id → many events sharing "
                                     "it. Threaded via " code { "X-Request-ID" } " header. "
                                     "Without it, the headline feature doesn't work."
                                 } }
                            tr { td { code { "service" } }
                                 td { "yes" }
                                 td {
                                     "Which app emitted this. " code { "versable-app" } ", "
                                     code { "versable-api" } ", " code { "credit-worker" } " — "
                                     "stable strings, document them in EVENT_TAXONOMY.md."
                                 } }
                            tr { td { code { "env" } }
                                 td { "yes" }
                                 td {
                                     code { "prod" } " / " code { "staging" } " / " code { "dev" } ". "
                                     "Drives the env pill color in the dashboard."
                                 } }
                            tr { td { code { "severity" } }
                                 td { "no (defaults info)" }
                                 td {
                                     code { "trace" } " · " code { "debug" } " · " code { "info" }
                                     " · " code { "warn" } " · " code { "error" } " · " code { "fatal" }
                                     ". Maps to OTel severity_number 1/5/9/13/17/21."
                                 } }
                            tr { td { code { "payload" } }
                                 td { "no" }
                                 td {
                                     "Free-form JSON object. Searched via FTS in the dashboard's "
                                     "full-text-search box. " code { "_auth_consumer" } " is "
                                     "server-stamped here on ingest — emitters cannot fake it."
                                 } }
                        }
                    }

                    h2 id="auth" { "Authentication" }
                    p { "logger-crab has two unrelated auth surfaces:" }

                    div.auth-compare {
                        div.auth-card {
                            div.auth-card-title { "Ingest" }
                            div.auth-card-sub { code { "/ingest" } " — POST events" }
                            ul.auth-card-list {
                                li { "Per-consumer named tokens" }
                                li { "Two tiers: " code { "full" } " (server) / " code { "public" } " (browser)" }
                                li { "Bearer header only (programmatic)" }
                                li { "Server stamps " code { "_auth_consumer" } " — unspoofable" }
                            }
                        }
                        div.auth-card {
                            div.auth-card-title { "Dashboard" }
                            div.auth-card-sub { code { "/" } " · " code { "/logs" } " · " code { "/health/full" } }
                            ul.auth-card-list {
                                li { "Single shared token (" code { "DASHBOARD_TOKEN" } ")" }
                                li { "Cookie auth (" code { "?token=" } " sets, lasts 30d)" }
                                li { "Bearer accepted for curl/scripts" }
                                li { code { "/health" } " is public; " code { "/health/full" } " gated" }
                            }
                        }
                    }

                    h3 { "Ingest auth (per-consumer)" }
                    p {
                        "Each emitter has its own token, registered in env as "
                        code { "INGEST_TOKEN_<NAME>=<tier>:<token>" } ". Two tiers:"
                    }
                    ul {
                        li {
                            code { "full" } " — server-side emitters (Next.js routes, FastAPI). "
                            "Token must never ship in a browser bundle. Keep in non-NEXT_PUBLIC env vars."
                        }
                        li {
                            code { "public" } " — browser-side emitters. Safe to ship in a JS bundle "
                            "(in " code { "NEXT_PUBLIC_LOGGER_CRAB_TOKEN_PUBLIC" } " on Vercel). "
                            "Authenticated but rate-limited (planned)."
                        }
                    }
                    p {
                        "On every successful ingest, logger-crab stamps the consumer's name "
                        "(" code { "INGEST_TOKEN_PROD_APP_SERVER" } " → "
                        code { "prod-app-server" } ") into "
                        code { "payload._auth_consumer" } ". This is the trustworthy attribution "
                        "field — the emitter cannot fake it, since it's bound to whichever bearer "
                        "token authenticated the request."
                    }
                    p {
                        "View registered consumers and any malformed env vars by clicking the "
                        b { "gear icon" } " on the dashboard."
                    }

                    h3 { "Dashboard auth (single shared token)" }
                    p {
                        "The dashboard at " code { "/" } " is gated by " code { "DASHBOARD_TOKEN" } ". "
                        "First visit: paste " code { "https://logger-crab.onrender.com/?token=YOUR_TOKEN" }
                        " in the URL bar. The server validates, sets an HttpOnly cookie, and "
                        "redirects to " code { "/" } ". Cookie persists 30 days."
                    }
                    p {
                        "API consumers (" code { "curl" } " / scripts) use Bearer instead: "
                        code { r#"curl -H "Authorization: Bearer $DASHBOARD_TOKEN" /logs"# } "."
                    }

                    h2 { "Endpoints" }
                    p { "See the interactive " a href="/api" { "API reference" }
                        " for full schemas + try-it-out." }
                    table {
                        thead {
                            tr { th { "Method" } th { "Path" } th { "Auth" } th { "Purpose" } }
                        }
                        tbody {
                            tr.endpoint-row {
                                td { span.method.get { "GET" } "/" }
                                td { "Dashboard" }
                                td { code { "DASHBOARD_TOKEN" } }
                                td { "HTML dashboard with filters" }
                            }
                            tr.endpoint-row {
                                td { span.method.post { "POST" } "/ingest" }
                                td { "Ingest batch" }
                                td { code { "INGEST_TOKEN" } }
                                td { "Accept OTel-flavored batch of records" }
                            }
                            tr.endpoint-row {
                                td { span.method.get { "GET" } "/logs" }
                                td { "Query JSON" }
                                td { code { "DASHBOARD_TOKEN" } }
                                td { "Filtered, cursor-paginated event page" }
                            }
                            tr.endpoint-row {
                                td { span.method.get { "GET" } "/health" }
                                td { "Health" }
                                td { "—" }
                                td { "Hot + cold store liveness" }
                            }
                            tr.endpoint-row {
                                td { span.method.get { "GET" } "/health/full" }
                                td { "Rich health" }
                                td { code { "DASHBOARD_TOKEN" } }
                                td { "Detailed cold/hot tier state including " code { "last_issue" } }
                            }
                            tr.endpoint-row {
                                td { span.method.get { "GET" } "/logs/download.ndjson" }
                                td { "Bulk export" }
                                td { code { "DASHBOARD_TOKEN" } }
                                td { "Filtered events as NDJSON download (capped at 2000)" }
                            }
                            tr.endpoint-row {
                                td { span.method.get { "GET" } "/api" }
                                td { "Swagger UI" }
                                td { "—" }
                                td { "Interactive OpenAPI 3.1 reference" }
                            }
                            tr.endpoint-row {
                                td { span.method.get { "GET" } "/openapi.yaml" }
                                td { "OpenAPI spec" }
                                td { "—" }
                                td { "Raw spec for tooling" }
                            }
                        }
                    }

                    h2 id="dashboard-tour" { "Dashboard tour" }
                    p {
                        "The dashboard is the place you'll spend the most time. A few things make "
                        "it faster:"
                    }
                    h3 { "Filters" }
                    ul {
                        li {
                            b { "Service / env / event prefix / FTS" } " — text inputs with "
                            "autocomplete suggesting every value present in the hot store."
                        }
                        li {
                            b { "Min level pill row" } " — clicking " code { "WARN" } " shows "
                            "warn/error/fatal; clicking " code { "any" } " removes the floor."
                        }
                        li {
                            b { "Click-to-filter from the table" } " — clicking a service chip, env "
                            "pill, event namespace, request_id, or severity in any row sets that "
                            "as a filter."
                        }
                        li {
                            b { "Active filter chips" } " above the table show what's set; click "
                            "the " code { "×" } " to remove just that filter without resetting."
                        }
                        li { b { "Reset" } " clears every filter. " b { "Apply" } " is also " code { "Enter" } "." }
                    }
                    h3 { "Keyboard shortcuts" }
                    p { "Press " code { "?" } " on the dashboard for a floating cheatsheet. Quick reference:" }
                    ul {
                        li { code { "/" } " — focus the search box" }
                        li { code { "j" } " / " code { "↓" } " — next row" }
                        li { code { "k" } " / " code { "↑" } " — previous row" }
                        li { code { "Enter" } " — expand current row's payload" }
                        li { code { "r" } " — refresh (preserves filters)" }
                        li { code { "Esc" } " — blur input / close modal" }
                    }
                    h3 { "Settings modal" }
                    p {
                        "Gear icon (top-right). Lists every consumer registered via "
                        code { "INGEST_TOKEN_<NAME>" } " and any " b { "config warnings" } " — "
                        "malformed env vars, deprecated names, etc. Tokens themselves are never "
                        "displayed; only the consumer name and source env var key."
                    }
                    h3 { "Footer" }
                    p {
                        "Three columns: build, hot tier, cold tier. The cold tier panel shows the "
                        b { "last_issue" } " when S3 access fails — kind, summary, and a remediation "
                        "hint. See the " a href="#troubleshooting" { "troubleshooting" } " section "
                        "for the full failure-mode catalog."
                    }

                    h2 { "Request-ID backbone" }
                    p {
                        "Every request gets a " code { "request_id" } " at the edge (UI or API gateway) "
                        "and threads it through every downstream hop via the "
                        code { "X-Request-ID" } " header. Logs emitted anywhere along the trace "
                        "carry the same id, so one filter groups the whole journey."
                    }
                    div.flow-diagram {
                        div.flow-step {
                            span.flow-step-num { "1" }
                            div.flow-step-body {
                                div.flow-step-title { "Browser → Next.js middleware" }
                                code.flow-step-detail { "rid = headers['x-request-id'] ?? crypto.randomUUID()" }
                            }
                        }
                        div.flow-step {
                            span.flow-step-num { "2" }
                            div.flow-step-body {
                                div.flow-step-title { "Next.js route → FastAPI" }
                                code.flow-step-detail { "fetch(api, { headers: { 'x-request-id': rid } })" }
                            }
                        }
                        div.flow-step {
                            span.flow-step-num { "3" }
                            div.flow-step-body {
                                div.flow-step-title { "FastAPI → Redis enqueue" }
                                code.flow-step-detail { "job.request_id = request.state.request_id" }
                            }
                        }
                        div.flow-step {
                            span.flow-step-num { "4" }
                            div.flow-step-body {
                                div.flow-step-title { "Worker pulls job" }
                                code.flow-step-detail { "rid = payload['request_id']  # same value, all 4 hops" }
                            }
                        }
                        div.flow-result {
                            "filter dashboard by " code { "request_id=<rid>" }
                            " → all 4 hops in chronological order, one screen"
                        }
                    }
                    p {
                        "The id is also attached to Sentry scope, so exceptions can be cross-referenced "
                        "against the full log trace."
                    }

                    h2 id="rotation" { "Rotation cycle" }
                    p {
                        "Every " code { "ROTATION_INTERVAL_SECS" } " (default 1h), the rotation task "
                        "walks events older than " code { "HOT_RETENTION_HOURS" } " (default 48h), "
                        "groups them by " code { "(env, service, hour-bucket)" } ", writes each "
                        "group as a single NDJSON.gz file to S3, then deletes the archived events "
                        "from hot. If any group fails to write, the whole cycle skips the delete — "
                        "next cycle retries."
                    }
                    div.cycle-diagram {
                        div.cycle-step.cycle-step-1 {
                            div.cycle-step-label { "01" }
                            div.cycle-step-name { "scan hot" }
                            div.cycle-step-detail { code { "ts < now - 48h" } }
                        }
                        div.cycle-arrow { "→" }
                        div.cycle-step.cycle-step-2 {
                            div.cycle-step-label { "02" }
                            div.cycle-step-name { "group by hour" }
                            div.cycle-step-detail { code { "(env, service, h)" } }
                        }
                        div.cycle-arrow { "→" }
                        div.cycle-step.cycle-step-3 {
                            div.cycle-step-label { "03" }
                            div.cycle-step-name { "write S3 batch" }
                            div.cycle-step-detail { code { "PUT *.ndjson.gz" } }
                        }
                        div.cycle-arrow { "→" }
                        div.cycle-step.cycle-step-4 {
                            div.cycle-step-label { "04" }
                            div.cycle-step-name { "delete hot" }
                            div.cycle-step-detail { code { "if all groups OK" } }
                        }
                    }

                    h2 id="severity" { "Severity scale" }
                    p {
                        "OpenTelemetry-style severity numbers, in increasing intensity. "
                        "The " a href="/" { "dashboard" } " color-codes rows by severity "
                        "(error/fatal get a coral left-stripe; warn gets amber)."
                    }
                    div.severity-scale {
                        @for (num, name, hint) in &[
                            (1u8, "trace", "ultra-verbose; follow-the-call-stack debug"),
                            (5, "debug", "developer-only state info"),
                            (9, "info", "default — business events worth recording"),
                            (13, "warn", "degraded state; not yet broken"),
                            (17, "error", "user-visible failure"),
                            (21, "fatal", "service crash / unrecoverable"),
                        ] {
                            div class=(format!("sev-row sev-row-{}", name)) {
                                span.sev-num { (num) }
                                span.sev-name { (name.to_ascii_uppercase()) }
                                span.sev-hint { (hint) }
                            }
                        }
                    }

                    h2 id="hot-store" { "Hot store" }
                    p {
                        "SQLite in WAL mode with " code { "synchronous=NORMAL" } " and a 5s busy-timeout. "
                        "An FTS5 virtual table shadows the " code { "events" } " table for full-text search, "
                        "maintained via AI/AD/AU triggers."
                    }
                    p {
                        "Retention: last 24–48h. A background rotator moves older rows to S3 NDJSON "
                        "and truncates the hot table."
                    }

                    h2 id="cold-store" { "Cold store" }
                    p {
                        "S3 NDJSON under " code { "logs/YYYY/MM/DD/HH/" } " partitioning. "
                        "Each file is gzip-compressed. Queries against the cold tier are (intentionally) "
                        "not exposed via " code { "/logs" } " — use Athena / DuckDB / S3 Select against "
                        "the bucket directly."
                    }

                    h2 { "Environment variables" }
                    p { "Currently active on this server:" }
                    table {
                        thead { tr { th { "Var" } th { "Value" } th { "Notes" } } }
                        tbody {
                            tr { td { code { "HOT_STORE" } }
                                 td { code { (boot.hot_store) } }
                                 td { code { "sqlite" } " recommended in prod" } }
                            tr { td { code { "COLD_STORE" } }
                                 td { code { (boot.cold_store) } }
                                 td { code { "s3" } " or " code { "noop" } } }
                            tr { td { code { "DATABASE_URL" } }
                                 td { code { (boot.database_url_masked) } }
                                 td { "Credentials masked for display" } }
                            tr { td { code { "AWS_REGION" } }
                                 td { code { (boot.aws_region) } }
                                 td { } }
                            tr { td { code { "S3_LOGS_BUCKET" } }
                                 td { code { (boot.s3_bucket.as_deref().unwrap_or("(unset)")) } }
                                 td { "Required when " code { "COLD_STORE=s3" } } }
                            tr { td { code { "APP_ENV" } }
                                 td { code { (boot.env_name) } }
                                 td { "Display only" } }
                        }
                    }

                    h2 id="client-quickstart" { "Client library quickstart" }
                    p {
                        "There is no published " code { "@versable/*" } " or PyPI package — each "
                        "consumer codebase carries its own thin sink that POSTs the same wire "
                        "format. The TypeScript pattern below is the working one in "
                        code { "enhancement-product/frontend/src/utils/logger/" } "; the Python "
                        "pattern is a reference for backend / worker / cron integration "
                        "(stdout-only today)."
                    }

                    div.callout.callout-tip {
                        div.callout-icon { "▸" }
                        div.callout-body {
                            "Wire format is identical across runtimes. If you keep the envelope "
                            "shape consistent (" code { "resource" } " + " code { "scope" } " + "
                            code { "events[]" } "), any HTTP client will work — there's no SDK "
                            "lock-in."
                        }
                    }

                    h3 { "What to register before any code runs" }
                    p {
                        "Six env vars per runtime. Token comes from the corresponding "
                        code { "INGEST_TOKEN_<NAME>" } " on the logger-crab server — copy the part "
                        b { "after" } " the " code { "full:" } " / " code { "public:" } " prefix."
                    }
                    p { b { "Frontend (Next.js, Vercel):" } }
                    pre { code class="language-bash" { (ENV_TS_QUICKSTART) } }
                    p { b { "Backend / worker / cron (Render):" } }
                    pre { code class="language-bash" { (ENV_PY_QUICKSTART) } }

                    h3 { "TypeScript — actually working integration" }
                    p {
                        "The frontend uses a pluggable-sink logger with a server-only crab sink. "
                        "Client events never POST to crab directly — they buffer in the browser, "
                        "ship to " code { "/api/log" } ", and the server re-emits through its own "
                        "logger so the same sink picks them up. This keeps the crab token out of "
                        "the browser bundle entirely."
                    }
                    p { b { "1. The sink (abridged):" } }
                    pre { code class="language-typescript" { (TS_SINK_SHAPE) } }
                    p { b { "2. The call surface — uniform across the app:" } }
                    pre { code class="language-typescript" { (TS_CALLSITE) } }
                    p { b { "3. Client → server bridge (so the browser never holds a token):" } }
                    pre { code class="language-typescript" { (TS_BROWSER_BRIDGE) } }

                    h3 { "Python — reference pattern (not yet wired in backend)" }
                    p {
                        "The backend currently configures loguru to stdout and Render captures the "
                        "lines. To ship to crab, add a loguru sink that batches + POSTs. Reference "
                        "shape below mirrors the TypeScript sink so the wire format stays identical "
                        "and dashboard queries work across runtimes."
                    }
                    pre { code class="language-python" { (PY_SINK_SHAPE) } }
                    p { b { "Call sites stay loguru-native:" } }
                    pre { code class="language-python" { (PY_CALLSITE) } }

                    h2 id="what-to-send" { "What to send where" }
                    p {
                        "Picking field values consistently is what makes the dashboard useful. "
                        "The table below maps emitter context to what's worth sending."
                    }
                    table {
                        thead {
                            tr {
                                th { "Emitter type" }
                                th { code { "request_id" } "?" }
                                th { "Typical " code { "service" } }
                                th { "Event-name examples" }
                            }
                        }
                        tbody {
                            tr {
                                td { "Web request handler (Next.js route, FastAPI endpoint)" }
                                td { b { "Yes — required." } " Mint at edge if missing." }
                                td { code { "versable-app" } " · " code { "versable-api" } }
                                td { code { "apiRoute.start" } " · " code { "auth.login.fail" } }
                            }
                            tr {
                                td { "Background worker picking up a queue job" }
                                td { "Yes " b { "only if" } " the originating request stored it in the job payload" }
                                td { code { "credit-worker" } " · " code { "backend-worker" } }
                                td { code { "pipeline.start" } " · " code { "job.retry" } }
                            }
                            tr {
                                td { "Cron job / scheduled task" }
                                td { b { "No." } " Use a synthetic " code { "tick_id" } " in payload if you need per-run grouping." }
                                td { code { "notify-cron" } }
                                td { code { "cron.tick.start" } " · " code { "cron.tick.end" } }
                            }
                            tr {
                                td { "Standalone script / CLI / migration" }
                                td { "No" }
                                td { code { "scripts" } " · " code { "migration" } }
                                td { code { "db.migrate.start" } }
                            }
                            tr {
                                td { "Boot / init / shutdown" }
                                td { "No" }
                                td { "whatever the runtime is" }
                                td { code { "service.boot" } " · " code { "service.shutdown" } }
                            }
                        }
                    }

                    h3 { "Field cheat-sheet (what each does)" }
                    ul {
                        li {
                            code { "event" } " — " b { "required, stable, dotted lowercase." }
                            " This is the primary group-by. Never rename after shipping; the "
                            "dashboard's event-prefix filter and any later queries depend on it. "
                            "Format: " code { "<noun>.<verb>" } " or " code { "<area>.<noun>.<verb>" } "."
                        }
                        li {
                            code { "severity" } " — " code { "info" } " by default. Use "
                            code { "warn" } " for degraded-but-working, " code { "error" } " for "
                            "user-visible failure, " code { "fatal" } " for crash."
                        }
                        li {
                            code { "request_id" } " — see the table above. When applicable, also "
                            "attach it to Sentry scope so exceptions and log traces link "
                            "bidirectionally."
                        }
                        li {
                            code { "service" } " — stable per emitter. Pick once and never change; "
                            "the env-pill color and " code { "/health" } " per-service stats depend "
                            "on a stable value."
                        }
                        li {
                            code { "env" } " — " code { "prod" } " / " code { "staging" } " / "
                            code { "dev" } ". Drives the env pill color in the dashboard."
                        }
                        li {
                            code { "payload" } " — free-form object. Searchable via FTS. "
                            "Don't put secrets in here; the frontend logger key-masks "
                            "(password/token/secret/auth/cookie) before any sink sees the event."
                        }
                        li {
                            code { "user_id" } " / " code { "session_id" } " / "
                            code { "team_id" } " — optional top-level correlation keys when the "
                            "request is user-attributable. Indexed for filter queries."
                        }
                    }

                    h2 id="patterns" { "Common patterns" }
                    h3 { "Threading the request_id" }
                    p {
                        "The headline feature only works if " code { "request_id" } " survives every "
                        "hop. Mint at the edge, copy via " code { "X-Request-ID" } " on every "
                        "outbound call, and store in Redis job payloads:"
                    }
                    pre { code class="language-typescript" { (REQUEST_ID_THREADING) } }
                    h3 { "Sentry cross-reference" }
                    p {
                        "Tag every Sentry scope with the same " code { "request_id" } " so "
                        "exceptions and log traces link bidirectionally:"
                    }
                    pre { code class="language-typescript" { (SENTRY_TAG_EXAMPLE) } }
                    h3 { "Server-stamped attribution" }
                    p {
                        "Every accepted event has " code { "payload._auth_consumer" }
                        " stamped with the registered consumer name from the token that authenticated "
                        "the request. Don't try to set this from the emitter — the server overwrites "
                        "any client-supplied value. Read it as the trustworthy provenance field."
                    }

                    h2 id="troubleshooting" { "Troubleshooting" }
                    h3 { "Events accepted but not appearing in dashboard" }
                    p {
                        "Likely the dashboard's filters are excluding them. Check the active filter "
                        "chips above the table; click " code { "Reset" } ". If it's still empty, the "
                        "hot store may be on " code { "memory" } " backend (resets on every restart). "
                        "Footer shows " code { "hot · sqlite" } " for persistent disk."
                    }
                    h3 { "401 on /ingest with a fresh token" }
                    p {
                        "Confirm the env var name is " code { "INGEST_TOKEN_<NAME>" } " (with the "
                        "underscore) and the value has a tier prefix: "
                        code { "full:<token>" } " or " code { "public:<token>" } ". The settings "
                        "modal will list which env vars failed to parse."
                    }
                    h3 { "Common S3 failures — quick triage" }
                    table {
                        thead { tr { th { "Kind" } th { "Status" } th { "Most likely cause" } th { "Fix" } } }
                        tbody {
                            tr {
                                td { code { "WrongRegion" } }
                                td { "301" }
                                td { "Bucket lives in a different AWS region" }
                                td { "Set " code { "AWS_REGION" } " to the actual region (issue's " code { "action" } " field tells you which)" }
                            }
                            tr {
                                td { code { "BucketNotFound" } }
                                td { "404" }
                                td { "Typo in bucket name; bucket in different account" }
                                td { "Verify " code { "S3_LOGS_BUCKET" } "; create with " code { "aws s3 mb" } }
                            }
                            tr {
                                td { code { "AuthFailure" } }
                                td { "403" }
                                td { code { "InvalidAccessKeyId" } " or " code { "SignatureDoesNotMatch" } }
                                td { "Verify " code { "AWS_ACCESS_KEY_ID" } " + " code { "AWS_SECRET_ACCESS_KEY" } }
                            }
                            tr {
                                td { code { "AccessDenied" } }
                                td { "403" }
                                td { "IAM user policy missing required actions" }
                                td { "Add " code { "s3:ListBucket" } " (bucket-level), " code { "s3:PutObject" } " + " code { "s3:GetObject" } " (object-level)" }
                            }
                            tr {
                                td { code { "NetworkFailure" } }
                                td { "—" }
                                td { "DNS / connectivity issue to *.amazonaws.com" }
                                td { "Check outbound network from logger-crab's host" }
                            }
                            tr {
                                td { code { "TimeoutError" } }
                                td { "—" }
                                td { "Slow network or AWS service degradation" }
                                td { "Check AWS Health Dashboard; transient retries" }
                            }
                        }
                    }

                    h3 { "Cold tier ok=false in /health/full" }
                    p {
                        "Look at " code { "cold.last_issue" } " in the JSON response. Common kinds:"
                    }
                    ul {
                        li {
                            code { "WrongRegion" } " — bucket lives in a different region than "
                            code { "AWS_REGION" } ". Issue's " code { "action" } " field tells you "
                            "the exact region to set."
                        }
                        li {
                            code { "BucketNotFound" } " — typo in " code { "S3_LOGS_BUCKET" } " or "
                            "the bucket lives in a different account."
                        }
                        li {
                            code { "AuthFailure" } " — " code { "AWS_ACCESS_KEY_ID" } " / "
                            code { "AWS_SECRET_ACCESS_KEY" } " mismatch."
                        }
                        li {
                            code { "AccessDenied" } " — IAM user policy missing required actions. "
                            "logger-crab needs " code { "s3:ListBucket" } " (bucket-level) and "
                            code { "s3:PutObject" } " + " code { "s3:GetObject" } " (object-level)."
                        }
                    }
                    h3 { "Verifying S3 from your laptop" }
                    p {
                        "Two scripts in the repo: "
                        code { "./scripts/check-s3.sh" } " (AWS CLI smoke) and "
                        code { "cargo run -p log-server --example check_s3" } " (uses the same SDK "
                        "the runtime uses, so it surfaces SDK-specific failures the CLI auto-fixes)."
                    }

                    h2 id="ops" { "Operational notes" }
                    ul {
                        li {
                            "Render auto-deploys on push to " code { "main" } " (BuildKit cache "
                            "makes source-only deploys ~2 min)."
                        }
                        li {
                            "Rotation cron runs every " code { "ROTATION_INTERVAL_SECS" } " (default "
                            "1h), archiving events older than " code { "HOT_RETENTION_HOURS" } " "
                            "(default 48h) to S3."
                        }
                        li {
                            "S3 lifecycle policies (transition to Glacier after 30d, expire after "
                            "365d) belong on the bucket itself — configure once in AWS Console."
                        }
                        li {
                            "Rotating " code { "DASHBOARD_TOKEN" } ": change in Render env, restart. "
                            "All existing cookies become invalid; users re-paste the new token via "
                            code { "?token=" } "."
                        }
                        li {
                            "Rotating an " code { "INGEST_TOKEN_*" } ": replace the value in Render "
                            "env, restart. Briefly accept both old + new by leaving the old row in "
                            "place during rollout, then delete it."
                        }
                    }

                    h2 { "Links" }
                    a.link-card href="/api" {
                        div.title { "Interactive API reference" }
                        div.sub { "Swagger UI — try ingest + query live" }
                    }
                    a.link-card href="/" {
                        div.title { "Dashboard" }
                        div.sub { "Browse + filter recent events" }
                    }
                    a.link-card href="https://github.com/alcatraz627/logger-crab" target="_blank" rel="noopener" {
                        div.title { "GitHub" }
                        div.sub { "alcatraz627/logger-crab" }
                    }
                }

                script { (PreEscaped(TOGGLE_JS)) }
                // Prism core + the languages used in code samples on this page.
                script src="https://cdnjs.cloudflare.com/ajax/libs/prism/1.29.0/prism.min.js" {}
                script src="https://cdnjs.cloudflare.com/ajax/libs/prism/1.29.0/components/prism-bash.min.js" {}
                script src="https://cdnjs.cloudflare.com/ajax/libs/prism/1.29.0/components/prism-typescript.min.js" {}
                script src="https://cdnjs.cloudflare.com/ajax/libs/prism/1.29.0/components/prism-python.min.js" {}
                script src="https://cdnjs.cloudflare.com/ajax/libs/prism/1.29.0/components/prism-json.min.js" {}
            }
        }
    }
}

/// Tweaks Prism's prism-tomorrow theme so it sits flush with our chrome
/// — same surface bg + border-radius as our other code blocks, and the
/// dark/light theme classes match the rest of the dashboard.
const PRISM_OVERRIDES_CSS: &str = r#"
pre[class*="language-"] {
  background: var(--surface2);
  border: 1px solid var(--border);
  border-radius: 8px;
  padding: 14px 16px;
  font-size: 12.5px;
  line-height: 1.55;
  margin: 14px 0;
  overflow-x: auto;
}
code[class*="language-"], pre[class*="language-"] {
  font-family: "JetBrains Mono", ui-monospace, SFMono-Regular, Menlo, monospace;
  text-shadow: none;
}
/* Light mode adjustments — Prism's tomorrow palette is dark by default;
   override token colors AND the base code color so identifiers (variable
   names, urls, plain text) read as a warm dark gray rather than Prism's
   default light gray. */
body.light pre[class*="language-"],
body.light pre[class*="language-"] code,
body.light code[class*="language-"] {
  background: var(--surface2);
  color: #4a4540;       /* warm dark gray — readable but softer than --text */
}
body.light .token.comment, body.light .token.prolog, body.light .token.doctype, body.light .token.cdata {
  color: #8a857c;       /* warm muted, was #6e7781 (cool) */
}
body.light .token.string, body.light .token.attr-value { color: #5d8e51; }   /* sage */
body.light .token.keyword, body.light .token.tag { color: #c25c52; }          /* coral */
body.light .token.function { color: #8a6ba8; }                                /* plum */
body.light .token.number, body.light .token.boolean, body.light .token.constant { color: #b8862c; }  /* amber */
body.light .token.operator, body.light .token.punctuation { color: #6b6660; } /* warm dim */
body.light .token.property, body.light .token.attr-name { color: #5d6fa3; }   /* slate */
"#;


// Legacy ASCII fallbacks — superseded by HTML/CSS diagrams above. Kept
// for any future ASCII-export use case. Currently unused.
#[allow(dead_code)]
const ARCH_DIAGRAM: &str = "(see arch-diagram HTML element)";
#[allow(dead_code)]
const REQ_ID_DIAGRAM: &str = "(see flow-diagram HTML element)";

const QUICKSTART_CURL: &str = r#"# Replace $TOKEN with the value from any INGEST_TOKEN_<NAME> env var
# (the part AFTER `full:` or `public:` — just the bare token).
curl -X POST https://logger-crab.onrender.com/ingest \
  -H "Authorization: Bearer $TOKEN" \
  -H "Content-Type: application/json" \
  -d '{
    "resource": {"service": "demo", "env": "dev"},
    "events": [{
      "event": "smoke.test",
      "severity_text": "info",
      "severity_number": 9,
      "ts": "2026-05-09T03:30:00Z",
      "request_id": "smoke-001",
      "message": "hello from curl",
      "payload": {"source": "docs-quickstart"}
    }]
  }'
# → {"accepted":1,"rejected":[]}
"#;

const REQUEST_ID_THREADING: &str = r#"// Next.js middleware — mint at the edge
const rid = req.headers.get("x-request-id") ?? crypto.randomUUID();
res.headers.set("x-request-id", rid);

// Outbound fetch — forward the header
await fetch(`${BACKEND_URL}/api/jobs`, {
  headers: { "x-request-id": rid, ... },
});

// FastAPI middleware — read + propagate
rid = request.headers.get("x-request-id") or str(uuid4())
request.state.request_id = rid

# Redis job payload — copy in
job["request_id"] = request.state.request_id
await redis.lpush(QUEUE, json.dumps(job))

# Worker — pull out, emit with same rid
rid = payload.get("request_id", "")
emit("pipeline.start", request_id=rid, ...)
"#;

const SENTRY_TAG_EXAMPLE: &str = r#"// TypeScript / Next.js
import * as Sentry from "@sentry/nextjs";
Sentry.withScope((scope) => {
  scope.setTag("request_id", rid);
  // ... your handler
});

# Python / FastAPI
import sentry_sdk
sentry_sdk.set_tag("request_id", rid)
"#;

// ─── Quickstart env-vars (the registration step) ──────────────────────────
// What to set on each runtime BEFORE wiring any code. URLs/tokens are
// runtime-only — never NEXT_PUBLIC_ for crab, since the client never talks
// to crab directly (client emits go via the server re-emit path).
const ENV_TS_QUICKSTART: &str = r#"# .env / Vercel — frontend (server runtime)
LOGGER_CRAB_URL=https://logger-crab.onrender.com
LOGGER_CRAB_TOKEN=<the bare token AFTER the "full:" prefix>
LOGGER_CRAB_ENABLED=true
LOGGER_CRAB_SERVICE_NAME=versable-app   # appears in dashboard's service column
LOGGER_CRAB_ENV=prod                    # prod | staging | dev — sets env pill

# Optional kill switches
APP_LOGGER_SERVER=true                  # default true; set false to mute server emits
NEXT_PUBLIC_APP_LOGGER_CLIENT=true      # opt-in for prod browser emits (dev is on)
"#;

const ENV_PY_QUICKSTART: &str = r#"# Render env — backend / worker / cron
LOGGER_CRAB_URL=https://logger-crab.onrender.com
LOGGER_CRAB_TOKEN=<the bare token AFTER the "full:" prefix>
LOGGER_CRAB_ENABLED=true
LOGGER_CRAB_SERVICE_NAME=versable-api   # or credit-worker, notify-cron, scraper-api
LOGGER_CRAB_ENV=prod
"#;

// ─── Real TS code — abridged from frontend/src/utils/logger/ ──────────────
// Pattern: pluggable sinks, server-side crab sink is the only path to /ingest.
// Client events reach crab via /api/log → server emit → server crab sink.
const TS_SINK_SHAPE: &str = r#"// frontend/src/utils/logger/sinks/crab-sink-shared.ts (abridged)
// Server-only. Batches 25 events / 2000 ms, POSTs to /ingest, fails silently.

const buildEnvelope = (cfg, events) => JSON.stringify({
  resource: { service: cfg.service, env: cfg.env },
  scope: { name: "logger-crab.ts", version: "1" },
  events: events.map(toWire),
});

const send = (events) => {
  fetch(`${cfg.url}/ingest`, {
    method: "POST",
    headers: {
      Authorization: `Bearer ${cfg.token}`,
      "Content-Type": "application/json",
    },
    body: buildEnvelope(cfg, events),
    keepalive: true, // unload doesn't drop the last batch
  }).catch(() => undefined); // telemetry must never break the app
};
"#;

const TS_CALLSITE: &str = r#"// Anywhere in server code — the call surface is uniform.
import { logger } from "@/utils/logger";

logger.info("apiRoute.start", { route: routeName });

logger.warn("auth.backend_rejected", { user_id: userId, origin });

logger.error("openai.call.error",
  { provider: "openai", status: 429 },
  err,                      // 3rd arg: Error object — auto-flattens to payload
);

// Canonical full-control emit
logger.event({
  event: "pipeline.start",
  severity: "info",
  request_id: rid,          // explicit; otherwise filled from withLoggerContext()
  payload: { job_id, attempt: 1 },
  message: "picked up job",
});
"#;

const TS_BROWSER_BRIDGE: &str = r#"// frontend/src/app/api/log/route.ts — receives client-buffered events,
// re-emits via the server logger so the crab sink picks them up.
// Client never talks to crab directly — no token in the browser bundle.
export async function POST(req: NextRequest) {
  const { events } = await req.json();
  for (const e of events) logger.event(e);   // origin: "client" preserved
  return NextResponse.json({ ok: true });
}
"#;

// ─── Python reference sink — pattern only; backend integration pending ────
// Backend today (2026-05) only configures loguru → stdout. To send to crab,
// add a loguru sink that batches and POSTs to /ingest. Shape below mirrors
// the working TS sink so the wire format stays identical.
const PY_SINK_SHAPE: &str = r#"# backend/lib/logging/crab_sink.py — REFERENCE; not yet wired.
# Wire into the loguru singleton in lib/logging/__init__.py via logger.add(...).
# Mirrors the TS sink: batch 25 events / 2 s, fire-and-forget, lossy on error.

import atexit
import json
import os
import threading
import urllib.request
from collections import deque

URL     = os.environ["LOGGER_CRAB_URL"]
TOKEN   = os.environ["LOGGER_CRAB_TOKEN"]
SERVICE = os.environ.get("LOGGER_CRAB_SERVICE_NAME", "versable-api")
ENV     = os.environ.get("LOGGER_CRAB_ENV", "dev")

BATCH, FLUSH_S = 25, 2.0
_buf: deque = deque()
_lock = threading.Lock()
_timer: threading.Timer | None = None

def _post(events):
    if not events:
        return
    body = json.dumps({
        "resource": {"service": SERVICE, "env": ENV},
        "scope":    {"name": "logger-crab.py", "version": "1"},
        "events":   events,
    }).encode()
    req = urllib.request.Request(
        f"{URL.rstrip('/')}/ingest", data=body,
        headers={"Authorization": f"Bearer {TOKEN}",
                 "Content-Type":  "application/json"},
    )
    try:
        urllib.request.urlopen(req, timeout=5).read()
    except Exception:
        pass  # telemetry must never break the app

def _drain_locked():
    # Caller already holds _lock. Returns events to send (after releasing).
    batch = list(_buf)
    _buf.clear()
    return batch

def _flush():
    global _timer
    with _lock:
        batch = _drain_locked()
        _timer = None
    _post(batch)

def crab_handler(message):
    global _timer
    rec = message.record
    extra = rec["extra"] or {}
    # Callers should bind extra["event"]. The "unbound" fallback makes
    # missing tags visible in the dashboard instead of silently polluting
    # the event taxonomy with loguru module paths.
    event = extra.get("event") or "unbound"
    wire = {
        "event":           event,
        "severity_text":   rec["level"].name.lower(),
        "severity_number": rec["level"].no,
        "ts":              rec["time"].isoformat(),
        "message":         rec["message"],
        "request_id":      extra.get("request_id"),
        "user_id":         extra.get("user_id"),
        "payload":         {k: v for k, v in extra.items()
                            if k not in ("event", "request_id", "user_id")},
    }
    flush_now = None
    with _lock:
        _buf.append(wire)
        if len(_buf) >= BATCH:
            flush_now = _drain_locked()
            if _timer:
                _timer.cancel()
                _timer = None
        elif _timer is None:
            _timer = threading.Timer(FLUSH_S, _flush)
            _timer.daemon = True
            _timer.start()
    if flush_now is not None:
        _post(flush_now)

atexit.register(_flush)  # drain on process exit so short scripts aren't lossy
"#;

const PY_CALLSITE: &str = r#"# Anywhere in backend code — PrintLogger or raw loguru both work.
from lib.logging import PrintLogger
from loguru import logger

log = PrintLogger(label="claimer")
log.info("claimed task")                              # → [INFO] [claimer] claimed task
log.warning("lock contention", label="hot")           # sub-label

# For crab-bound events, bind structured fields via loguru's `extra`:
logger.bind(event="job.start", request_id=rid, job_id=jid).info("picked up")

# In `except` blocks — loguru attaches the active traceback automatically
try:
    do_thing()
except Exception:
    log.exception("openai.call.error", label="429")
"#;
