//! Dashboard login flow — extracted from `dashboard.rs`.
//!
//! Three pieces:
//!   1. `render_login_page` — the standalone HTML form shown when auth is missing
//!   2. `login_redirect_with_cookie` — sets the auth cookie, 302 to `/`
//!   3. `is_https` — detects HTTPS via `X-Forwarded-Proto` so we can flag
//!      the cookie `Secure` only in production
//!
//! Self-contained CSS + theme-detect JS bundled inline so the login page
//! works even before the main dashboard CSS loads.

use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
use axum::response::{Html, IntoResponse, Redirect, Response};
use maud::{html, PreEscaped, DOCTYPE};

use super::nav::BRAND_NAME;

/// Builds the 302 redirect that lands the user on `/` with the dashboard
/// cookie set. `Max-Age=2592000` = 30 days; `HttpOnly` blocks JS access;
/// `SameSite=Strict` blocks cross-site auto-send. `Secure` is added when
/// the request reached us over HTTPS.
pub(crate) fn login_redirect_with_cookie(
    token: &str,
    headers: &HeaderMap,
    next: Option<&str>,
) -> Response {
    let secure_flag = if is_https(headers) { "; Secure" } else { "" };
    let cookie = format!(
        "{}={}; Path=/; HttpOnly; SameSite=Strict; Max-Age=2592000{}",
        super::auth::DASHBOARD_COOKIE,
        token,
        secure_flag,
    );
    // `next` was already validated by `safe_next`; treat None as "/".
    let target = next.unwrap_or("/");
    let mut response = Redirect::to(target).into_response();
    if let Ok(value) = HeaderValue::from_str(&cookie) {
        response.headers_mut().insert(header::SET_COOKIE, value);
    }
    response
}

/// Detect whether the original request was HTTPS via `X-Forwarded-Proto`,
/// which Render and most reverse proxies set after TLS termination.
pub(crate) fn is_https(headers: &HeaderMap) -> bool {
    headers
        .get("x-forwarded-proto")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.eq_ignore_ascii_case("https"))
        .unwrap_or(false)
}

/// Custom login page — single password input, no username field. Replaces
/// the browser-native HTTP Basic prompt. Submits via GET to `/?token=...`
/// which the dashboard handler turns into a cookie + redirect.
pub(crate) fn render_login_page(
    token_was_invalid: bool,
    next: Option<&str>,
) -> impl IntoResponse {
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
                        @if let Some(n) = next {
                            input type="hidden" name="next" value=(n);
                        }
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

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderValue;

    #[test]
    fn is_https_returns_false_when_no_header() {
        assert!(!is_https(&HeaderMap::new()));
    }

    #[test]
    fn is_https_detects_lowercase() {
        let mut h = HeaderMap::new();
        h.insert("x-forwarded-proto", HeaderValue::from_static("https"));
        assert!(is_https(&h));
    }

    #[test]
    fn is_https_detects_uppercase() {
        let mut h = HeaderMap::new();
        h.insert("x-forwarded-proto", HeaderValue::from_static("HTTPS"));
        assert!(is_https(&h));
    }

    #[test]
    fn is_https_returns_false_for_http() {
        let mut h = HeaderMap::new();
        h.insert("x-forwarded-proto", HeaderValue::from_static("http"));
        assert!(!is_https(&h));
    }

    #[test]
    fn render_login_page_contains_login_form() {
        use axum::body::to_bytes;
        let resp = render_login_page(false).into_response();
        let bytes = futures::executor::block_on(to_bytes(resp.into_body(), 65536)).unwrap();
        let body = std::str::from_utf8(&bytes).unwrap();
        assert!(body.contains("Versable logger-crab"));
        assert!(body.contains(r#"action="/""#));
        assert!(body.contains(r#"name="token""#));
        assert!(body.contains("Sign in"));
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
    fn login_redirect_sets_cookie_and_location() {
        let resp = login_redirect_with_cookie("test-tok", &HeaderMap::new());
        let cookie = resp.headers().get(header::SET_COOKIE).unwrap().to_str().unwrap();
        assert!(cookie.contains("test-tok"));
        assert!(cookie.contains("HttpOnly"));
        assert!(cookie.contains("SameSite=Strict"));
        assert!(cookie.contains("Max-Age=2592000"));
        // No Secure flag without HTTPS forwarded-proto
        assert!(!cookie.contains("Secure"));
        let location = resp.headers().get(header::LOCATION).unwrap().to_str().unwrap();
        assert_eq!(location, "/");
    }

    #[test]
    fn login_redirect_adds_secure_when_https() {
        let mut h = HeaderMap::new();
        h.insert("x-forwarded-proto", HeaderValue::from_static("https"));
        let resp = login_redirect_with_cookie("test-tok", &h);
        let cookie = resp.headers().get(header::SET_COOKIE).unwrap().to_str().unwrap();
        assert!(cookie.contains("Secure"));
    }
}
