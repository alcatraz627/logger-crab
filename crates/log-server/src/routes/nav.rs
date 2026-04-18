//! Shared top nav used by the dashboard, docs, and Swagger UI routes.
//!
//! Exports `render_nav()` (Maud fragment), `NAV_CSS` (tokens + nav rules +
//! toggle button), and `TOGGLE_JS` (dark/light persistence). Icon helpers
//! are re-exposed because the dashboard filter form reuses them.

use maud::{html, Markup, PreEscaped};

pub const BRAND_NAME: &str = "Versable logger-crab";
pub const GITHUB_URL: &str = "https://github.com/versable/logger-crab";

#[derive(Copy, Clone, PartialEq, Eq)]
pub enum Active {
    Dashboard,
    Api,
    Docs,
}

pub fn render_nav(active: Active, health_ok: Option<bool>) -> Markup {
    let cls = |a: Active| -> &'static str {
        if a == active {
            "nav-link active"
        } else {
            "nav-link"
        }
    };

    html! {
        nav.lc-nav {
            a.brand href="/" title=(BRAND_NAME) {
                img.brand-logo src="/assets/versable-logo.svg" alt="Versable" width="22" height="22";
                span.brand-name { (BRAND_NAME) }
            }
            div.nav-links {
                a class=(cls(Active::Dashboard)) href="/" title="dashboard" {
                    (icon_table()) span { "/" }
                }
                a class=(cls(Active::Api)) href="/api" title="OpenAPI / Swagger UI" {
                    (icon_code()) span { "/api" }
                }
                a class=(cls(Active::Docs)) href="/docs" title="docs" {
                    (icon_book()) span { "/docs" }
                }
                a.nav-link href="/health" title="health endpoint" {
                    (icon_pulse()) span { "/health" }
                }
                a.nav-link href=(GITHUB_URL) target="_blank" rel="noopener" title="source on GitHub" {
                    (icon_github()) span { "/github" }
                }
            }
            @if let Some(ok) = health_ok {
                div.health-chip {
                    @if ok { span.dot.ok { } "hot ok" }
                    @else { span.dot.err { } "hot down" }
                }
            }
            button.toggle id="theme-toggle" title="toggle light/dark" { "☾ / ☀" }
        }
    }
}

/// CSS used only by `routes/docs.rs` and `routes/openapi.rs` — the dashboard
/// already includes these selectors via `dashboard.css`.
pub const NAV_CSS: &str = r#"
:root {
  --bg: #0d1117; --surface: #161b22; --surface2: #21262d; --text: #e6edf3;
  --dim: #7d8590; --muted: #484f58; --border: #30363d;
  --accent: #58a6ff; --accent2: #a371f7;
  --warn: #d29922; --err: #f85149; --ok: #3fb950;
}
body.light {
  --bg: #ffffff; --surface: #f6f8fa; --surface2: #eaeef2; --text: #1f2328;
  --dim: #656d76; --muted: #8c959f; --border: #d0d7de;
  --accent: #0969da; --accent2: #8250df;
  --warn: #9a6700; --err: #cf222e; --ok: #1a7f37;
}
.ic { vertical-align: -2px; flex-shrink: 0; }
nav.lc-nav {
  padding: 10px 24px; display: flex; align-items: center; gap: 16px;
  border-bottom: 1px solid var(--border); background: var(--surface);
  position: sticky; top: 0; z-index: 100;
  backdrop-filter: blur(12px);
}
nav.lc-nav .brand {
  display: inline-flex; align-items: center; gap: 9px;
  text-decoration: none; color: var(--text);
  padding: 4px 10px 4px 6px; border-radius: 8px;
  border: 1px solid transparent;
  transition: background 0.15s, border-color 0.15s;
}
nav.lc-nav .brand:hover {
  background: var(--bg);
  border-color: color-mix(in srgb, var(--accent) 28%, var(--border));
}
nav.lc-nav .brand .brand-logo {
  display: block; width: 22px; height: 22px;
  filter: drop-shadow(0 0 4px color-mix(in srgb, var(--accent) 30%, transparent));
}
nav.lc-nav .brand .brand-name {
  font-size: 14px; font-weight: 600; letter-spacing: -0.01em; color: var(--text);
}
nav.lc-nav .nav-links {
  display: inline-flex; align-items: center; gap: 2px; margin-left: 4px;
}
nav.lc-nav .nav-link {
  display: inline-flex; align-items: center; gap: 6px;
  color: var(--muted); text-decoration: none;
  font-size: 12.5px; font-weight: 500;
  font-family: "JetBrains Mono", monospace;
  padding: 5px 9px; border-radius: 6px;
  transition: color 0.12s, background 0.12s;
}
nav.lc-nav .nav-link:hover { color: var(--text); background: var(--bg); }
nav.lc-nav .nav-link.active { color: var(--accent); background: var(--bg); }
nav.lc-nav .nav-link .ic { color: currentColor; opacity: 0.7; }
nav.lc-nav .nav-link:hover .ic, nav.lc-nav .nav-link.active .ic { opacity: 1; }
.health-chip {
  margin-left: auto; display: inline-flex; align-items: center; gap: 6px;
  padding: 4px 10px; background: var(--bg); border: 1px solid var(--border);
  border-radius: 6px; font-size: 12px; color: var(--dim);
}
.health-chip .dot { width: 7px; height: 7px; border-radius: 50%; display: inline-block; }
.health-chip .dot.ok { background: var(--ok); box-shadow: 0 0 6px var(--ok); }
.health-chip .dot.err { background: var(--err); box-shadow: 0 0 6px var(--err); }
.toggle {
  background: transparent; color: var(--text);
  border: 1px solid var(--border); padding: 5px 11px; border-radius: 6px;
  cursor: pointer; font-size: 12px; font-family: inherit;
  transition: all 0.15s;
}
nav.lc-nav .nav-links + .toggle { margin-left: auto; }
.toggle:hover { background: var(--bg); }
"#;

pub const TOGGLE_JS: &str = r#"
(function() {
  var saved = localStorage.getItem('logger-crab-theme');
  if (saved === 'light') document.body.classList.add('light');
  var btn = document.getElementById('theme-toggle');
  if (!btn) return;
  btn.addEventListener('click', function() {
    document.body.classList.toggle('light');
    localStorage.setItem('logger-crab-theme',
      document.body.classList.contains('light') ? 'light' : 'dark');
  });
})();
"#;

pub fn svg_icon(path_d: &str) -> Markup {
    let svg = format!(
        r#"<svg class="ic" width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">{path_d}</svg>"#
    );
    PreEscaped(svg)
}

pub fn icon_table() -> Markup {
    svg_icon(r#"<path d="M3 3h18v18H3z"/><path d="M3 9h18"/><path d="M3 15h18"/><path d="M9 3v18"/>"#)
}
pub fn icon_code() -> Markup {
    svg_icon(r#"<polyline points="16 18 22 12 16 6"/><polyline points="8 6 2 12 8 18"/>"#)
}
pub fn icon_book() -> Markup {
    svg_icon(r#"<path d="M4 19.5A2.5 2.5 0 0 1 6.5 17H20"/><path d="M6.5 2H20v20H6.5A2.5 2.5 0 0 1 4 19.5v-15A2.5 2.5 0 0 1 6.5 2z"/>"#)
}
pub fn icon_pulse() -> Markup {
    svg_icon(r#"<polyline points="22 12 18 12 15 21 9 3 6 12 2 12"/>"#)
}
pub fn icon_github() -> Markup {
    svg_icon(r#"<path d="M9 19c-5 1.5-5-2.5-7-3m14 6v-3.87a3.37 3.37 0 0 0-.94-2.61c3.14-.35 6.44-1.54 6.44-7A5.44 5.44 0 0 0 20 4.77 5.07 5.07 0 0 0 19.91 1S18.73.65 16 2.48a13.38 13.38 0 0 0-7 0C6.27.65 5.09 1 5.09 1A5.07 5.07 0 0 0 5 4.77a5.44 5.44 0 0 0-1.5 3.78c0 5.42 3.3 6.61 6.44 7A3.37 3.37 0 0 0 9 18.13V22"/>"#)
}
pub fn icon_hash() -> Markup {
    svg_icon(r#"<line x1="4" y1="9" x2="20" y2="9"/><line x1="4" y1="15" x2="20" y2="15"/><line x1="10" y1="3" x2="8" y2="21"/><line x1="16" y1="3" x2="14" y2="21"/>"#)
}
pub fn icon_box() -> Markup {
    svg_icon(r#"<path d="M21 16V8a2 2 0 0 0-1-1.73l-7-4a2 2 0 0 0-2 0l-7 4A2 2 0 0 0 3 8v8a2 2 0 0 0 1 1.73l7 4a2 2 0 0 0 2 0l7-4A2 2 0 0 0 21 16z"/><polyline points="3.27 6.96 12 12.01 20.73 6.96"/><line x1="12" y1="22.08" x2="12" y2="12"/>"#)
}
pub fn icon_globe() -> Markup {
    svg_icon(r#"<circle cx="12" cy="12" r="10"/><line x1="2" y1="12" x2="22" y2="12"/><path d="M12 2a15.3 15.3 0 0 1 4 10 15.3 15.3 0 0 1-4 10 15.3 15.3 0 0 1-4-10 15.3 15.3 0 0 1 4-10z"/>"#)
}
pub fn icon_branch() -> Markup {
    svg_icon(r#"<line x1="6" y1="3" x2="6" y2="15"/><circle cx="18" cy="6" r="3"/><circle cx="6" cy="18" r="3"/><path d="M18 9a9 9 0 0 1-9 9"/>"#)
}
pub fn icon_search() -> Markup {
    svg_icon(r#"<circle cx="11" cy="11" r="8"/><line x1="21" y1="21" x2="16.65" y2="16.65"/>"#)
}
pub fn icon_check() -> Markup {
    svg_icon(r#"<polyline points="20 6 9 17 4 12"/>"#)
}
pub fn icon_x() -> Markup {
    svg_icon(r#"<line x1="18" y1="6" x2="6" y2="18"/><line x1="6" y1="6" x2="18" y2="18"/>"#)
}
