use axum::http::{header, StatusCode};
use axum::response::{Html, IntoResponse};
use maud::{html, PreEscaped, DOCTYPE};

use super::nav::{render_nav, Active, BRAND_NAME, NAV_CSS, TOGGLE_JS};

pub const OPENAPI_YAML: &str = include_str!("../../openapi.yaml");

pub async fn get_openapi_yaml() -> impl IntoResponse {
    (StatusCode::OK, [(header::CONTENT_TYPE, "application/yaml; charset=utf-8")], OPENAPI_YAML)
}

pub async fn get_swagger_ui() -> Html<String> {
    let markup = html! {
        (DOCTYPE)
        html lang="en" {
            head {
                meta charset="utf-8";
                meta name="viewport" content="width=device-width, initial-scale=1";
                title { (BRAND_NAME) " · API" }
                link rel="icon" type="image/svg+xml" href="/favicon.svg";
                link rel="stylesheet" href="https://unpkg.com/swagger-ui-dist@5.17.14/swagger-ui.css";
                link rel="preconnect" href="https://fonts.googleapis.com";
                link rel="preconnect" href="https://fonts.gstatic.com" crossorigin;
                link rel="stylesheet"
                    href="https://fonts.googleapis.com/css2?family=Inter:wght@400;500;600;700&family=JetBrains+Mono:wght@400;500&display=swap";
                style { (PreEscaped(NAV_CSS)) }
                style { (PreEscaped(SWAGGER_CSS)) }
            }
            body {
                (render_nav(Active::Api, None))
                div id="swagger-ui" { }
                script src="https://unpkg.com/swagger-ui-dist@5.17.14/swagger-ui-bundle.js" { }
                script src="https://unpkg.com/swagger-ui-dist@5.17.14/swagger-ui-standalone-preset.js" { }
                script { (PreEscaped(TOGGLE_JS)) }
                script { (PreEscaped(SWAGGER_BOOT_JS)) }
            }
        }
    };
    Html(markup.into_string())
}

const SWAGGER_CSS: &str = r#"
#swagger-ui { padding: 20px; background: var(--bg); }
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
body:not(.light) .swagger-ui .response-col_description__inner p { color: var(--text) !important; }
body:not(.light) .swagger-ui .scheme-container,
body:not(.light) .swagger-ui .opblock .opblock-section-header { background: var(--surface) !important; box-shadow: none; }
body:not(.light) .swagger-ui .opblock { background: var(--surface); border: 1px solid var(--border); }
body:not(.light) .swagger-ui select,
body:not(.light) .swagger-ui input[type=text],
body:not(.light) .swagger-ui textarea { background: var(--bg) !important; color: var(--text) !important; border-color: var(--border) !important; }
"#;

const SWAGGER_BOOT_JS: &str = r#"
window.ui = SwaggerUIBundle({
  url: '/openapi.yaml',
  dom_id: '#swagger-ui',
  deepLinking: true,
  presets: [SwaggerUIBundle.presets.apis, SwaggerUIStandalonePreset],
  plugins: [SwaggerUIBundle.plugins.DownloadUrl],
  layout: 'BaseLayout',
  tryItOutEnabled: true
});
"#;
