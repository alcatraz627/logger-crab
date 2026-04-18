use axum::extract::State;
use axum::response::Html;
use maud::{html, Markup, PreEscaped, DOCTYPE};

use super::nav::{render_nav, Active, BRAND_NAME, NAV_CSS, TOGGLE_JS};
use super::AppState;

pub async fn get_docs(State(state): State<AppState>) -> Html<String> {
    let markup = render(&state);
    Html(markup.into_string())
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
                style { (PreEscaped(NAV_CSS)) }
                style { (PreEscaped(CSS)) }
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

                    h2 { "Architecture" }
                    p {
                        "Services emit logs through thin client libraries (TS + Python) "
                        "to the log-server over HTTP. The server writes to the "
                        a href="#hot-store" { "hot store" } " (SQLite WAL) for fast queries, and "
                        "periodically rotates older rows to the "
                        a href="#cold-store" { "cold store" } " (S3 NDJSON) for long-term archive."
                    }
                    div.ascii { (ARCH_DIAGRAM) }

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

                    h2 { "Request-ID backbone" }
                    p {
                        "Every request gets a " code { "request_id" } " at the edge (UI or API gateway) "
                        "and threads it through every downstream hop via the "
                        code { "X-Request-ID" } " header. Logs emitted anywhere along the trace "
                        "carry the same id, so one filter groups the whole journey."
                    }
                    div.ascii { (REQ_ID_DIAGRAM) }
                    p {
                        "The id is also attached to Sentry scope, so exceptions can be cross-referenced "
                        "against the full log trace."
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
                            tr { td { code { "INGEST_TOKEN" } }
                                 td { code { (if boot.has_ingest_token { "set" } else { "(unset — unauth)" }) } }
                                 td { "Bearer on POST /ingest" } }
                            tr { td { code { "DASHBOARD_TOKEN" } }
                                 td { code { (if boot.has_dashboard_token { "set" } else { "(unset — unauth)" }) } }
                                 td { "Bearer on GET /logs + /" } }
                            tr { td { code { "APP_ENV" } }
                                 td { code { (boot.env_name) } }
                                 td { "Display only" } }
                        }
                    }

                    h2 { "Client library quickstart" }
                    h3 { "TypeScript" }
                    pre { code { (TS_EXAMPLE) } }
                    h3 { "Python" }
                    pre { code { (PY_EXAMPLE) } }

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
            }
        }
    }
}


const ARCH_DIAGRAM: &str = r#"┌─────────────┐  ┌─────────────┐  ┌──────────────┐  ┌──────────┐
│ Next.js /   │  │ FastAPI     │  │ Credit Wkr   │  │ Cron     │
│ Vercel      │  │ /Render     │  │ /Render      │  │ /Render  │
│  (ts-lib)   │  │  (py-lib)   │  │  (py-lib)    │  │ (py-lib) │
└──────┬──────┘  └──────┬──────┘  └──────┬───────┘  └─────┬────┘
       │                │                │                │
       └────────────────┴────────┬───────┴────────────────┘
                                 │  HTTPS POST /ingest  (Bearer)
                                 ▼
                     ┌────────────────────────┐
                     │  log-server (axum, RS) │
                     │     on Render          │
                     └──────────┬─────────────┘
                                │ write + rotate
                         ┌──────┴─────────┐
                         ▼                ▼
                  ┌────────────┐   ┌────────────────┐
                  │ SQLite WAL │   │ S3 NDJSON gz   │
                  │  hot 24-48h│   │  cold archive  │
                  └────────────┘   └────────────────┘
"#;

const REQ_ID_DIAGRAM: &str = r#"  UI ─────▶ FastAPI ─────▶ Redis enqueue ─────▶ Credit Worker
  │          │                  │                     │
  │ X-Request-ID: req_alice_01                         │
  │                                                    │
  └────────── same request_id in every log ────────────┘
                     │
                     └─▶ attached to Sentry scope for cross-reference
"#;

const TS_EXAMPLE: &str = r#"import { logger } from "@versable/logger-crab-client";

await logger.emit({
  event: "pipeline.start",
  severity: "info",
  request_id: req.headers["x-request-id"],
  service: "versable-api",
  message: `picked up job ${job.id}`,
  attrs: { job_id: job.id, attempt: 1 },
});
"#;

const PY_EXAMPLE: &str = r#"from logger_crab import emit

await emit(
    event="openai.call.error",
    severity="error",
    request_id=req_id,
    service="credit-worker",
    message="OpenAI 429",
    attrs={"provider": "openai", "status": 429, "retry_after_s": 30},
)
"#;
