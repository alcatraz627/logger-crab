use axum::http::{header, StatusCode};
use axum::response::{Html, IntoResponse};

pub const OPENAPI_YAML: &str = include_str!("../../openapi.yaml");

pub async fn get_openapi_yaml() -> impl IntoResponse {
    (StatusCode::OK, [(header::CONTENT_TYPE, "application/yaml; charset=utf-8")], OPENAPI_YAML)
}

pub async fn get_swagger_ui() -> Html<&'static str> {
    Html(SWAGGER_HTML)
}

const SWAGGER_HTML: &str = r#"<!DOCTYPE html>
<html lang="en">
<head>
  <meta charset="utf-8">
  <meta name="viewport" content="width=device-width, initial-scale=1">
  <title>logger-crab · API</title>
  <link rel="stylesheet" href="https://unpkg.com/swagger-ui-dist@5.17.14/swagger-ui.css">
  <style>
    :root {
      --lc-bg: #0e1116; --lc-surface: #161b22; --lc-text: #e6edf3;
      --lc-dim: #7d8590; --lc-border: #30363d; --lc-accent: #58a6ff;
    }
    body.light {
      --lc-bg: #ffffff; --lc-surface: #f6f8fa; --lc-text: #1f2328;
      --lc-dim: #656d76; --lc-border: #d0d7de; --lc-accent: #0969da;
    }
    * { box-sizing: border-box; }
    body { margin: 0; background: var(--lc-bg); color: var(--lc-text);
           font-family: Inter, ui-sans-serif, system-ui, sans-serif; }
    .lc-nav { padding: 12px 24px; display: flex; align-items: center; gap: 18px;
              border-bottom: 1px solid var(--lc-border); background: var(--lc-surface);
              position: sticky; top: 0; z-index: 100; }
    .lc-nav h1 { margin: 0; font-size: 15px; font-weight: 600; letter-spacing: -0.01em; }
    .lc-nav a { color: var(--lc-dim); text-decoration: none; font-size: 13px;
                padding: 4px 10px; border-radius: 6px; transition: all 0.15s; }
    .lc-nav a:hover { color: var(--lc-text); background: var(--lc-bg); }
    .lc-nav a.active { color: var(--lc-accent); background: var(--lc-bg); }
    .lc-toggle { margin-left: auto; background: transparent; color: var(--lc-text);
                 border: 1px solid var(--lc-border); padding: 4px 10px; border-radius: 6px;
                 cursor: pointer; font-size: 12px; font-family: inherit; }
    #swagger-ui { padding: 20px; background: var(--lc-bg); }
    body:not(.light) .swagger-ui,
    body:not(.light) .swagger-ui .info .title,
    body:not(.light) .swagger-ui .info p,
    body:not(.light) .swagger-ui .opblock-tag,
    body:not(.light) .swagger-ui .opblock .opblock-summary-description,
    body:not(.light) .swagger-ui .parameter__name,
    body:not(.light) .swagger-ui table thead tr td,
    body:not(.light) .swagger-ui table thead tr th,
    body:not(.light) .swagger-ui .model,
    body:not(.light) .swagger-ui .model-title,
    body:not(.light) .swagger-ui label,
    body:not(.light) .swagger-ui .tab li,
    body:not(.light) .swagger-ui .response-col_status,
    body:not(.light) .swagger-ui .response-col_description__inner p { color: var(--lc-text) !important; }
    body:not(.light) .swagger-ui .scheme-container,
    body:not(.light) .swagger-ui .opblock .opblock-section-header { background: var(--lc-surface) !important; box-shadow: none; }
    body:not(.light) .swagger-ui .opblock { background: var(--lc-surface); border: 1px solid var(--lc-border); }
    body:not(.light) .swagger-ui select,
    body:not(.light) .swagger-ui input[type=text],
    body:not(.light) .swagger-ui textarea { background: var(--lc-bg) !important; color: var(--lc-text) !important; border-color: var(--lc-border) !important; }
  </style>
</head>
<body>
  <nav class="lc-nav">
    <h1>🦀 logger-crab</h1>
    <a href="/">dashboard</a>
    <a href="/api" class="active">API</a>
    <a href="/docs">docs</a>
    <a href="/health">health</a>
    <button class="lc-toggle" id="theme-toggle">☾ / ☀</button>
  </nav>
  <div id="swagger-ui"></div>
  <script src="https://unpkg.com/swagger-ui-dist@5.17.14/swagger-ui-bundle.js"></script>
  <script src="https://unpkg.com/swagger-ui-dist@5.17.14/swagger-ui-standalone-preset.js"></script>
  <script>
    (function() {
      var saved = localStorage.getItem('logger-crab-theme');
      if (saved === 'light') document.body.classList.add('light');
      document.getElementById('theme-toggle').addEventListener('click', function() {
        document.body.classList.toggle('light');
        localStorage.setItem('logger-crab-theme',
          document.body.classList.contains('light') ? 'light' : 'dark');
      });
      window.ui = SwaggerUIBundle({
        url: '/openapi.yaml',
        dom_id: '#swagger-ui',
        deepLinking: true,
        presets: [SwaggerUIBundle.presets.apis, SwaggerUIStandalonePreset],
        plugins: [SwaggerUIBundle.plugins.DownloadUrl],
        layout: 'BaseLayout',
        tryItOutEnabled: true
      });
    })();
  </script>
</body>
</html>
"#;
