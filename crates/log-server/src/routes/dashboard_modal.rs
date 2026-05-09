//! Modal + toast components for the dashboard — extracted from `dashboard.rs`.
//!
//! - `render_settings_modal` — gear-icon dialog showing registered consumers
//!   and config warnings (tokens never displayed)
//! - `render_kbd_toast` — "?" keyboard cheatsheet floating panel

use maud::{html, Markup};

use super::auth::{AuthRole, TokenRecord};

/// Settings dialog. Lists every consumer registered via `INGEST_TOKEN_<NAME>`
/// (name + tier + source env var) and any non-fatal config warnings collected
/// at boot. Tokens themselves are never displayed.
pub(crate) fn render_settings_modal(
    consumers: &[TokenRecord],
    warnings: &[String],
) -> Markup {
    let full_count = consumers
        .iter()
        .filter(|c| matches!(c.tier, AuthRole::Full))
        .count();
    let public_count = consumers
        .iter()
        .filter(|c| matches!(c.tier, AuthRole::Public))
        .count();

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

/// Floating "Press ? for shortcuts" toast triggered by the `?` key.
/// Hidden by default; gets `.visible` class for ~4.5s when invoked (handled
/// in `dashboard.js`).
pub(crate) fn render_kbd_toast() -> Markup {
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    fn rec(name: &str, tier: AuthRole) -> TokenRecord {
        TokenRecord {
            name: name.into(),
            tier,
            token: Arc::new("redacted".into()),
            source_env_var: format!("INGEST_TOKEN_{}", name.to_uppercase().replace('-', "_")),
        }
    }

    #[test]
    fn settings_modal_lists_consumers() {
        let consumers = vec![
            rec("prod-app", AuthRole::Full),
            rec("dev-browser", AuthRole::Public),
        ];
        let html = render_settings_modal(&consumers, &[]).into_string();
        assert!(html.contains("prod-app"));
        assert!(html.contains("dev-browser"));
        assert!(html.contains("INGEST_TOKEN_PROD_APP"));
        assert!(html.contains("2 total"));
        assert!(html.contains("1 full"));
        assert!(html.contains("1 public"));
        // Tokens themselves should never appear in the modal — sanity check.
        assert!(!html.contains("redacted"));
    }

    #[test]
    fn settings_modal_empty_state_when_no_consumers() {
        let html = render_settings_modal(&[], &[]).into_string();
        assert!(html.contains("No consumers configured"));
        assert!(html.contains("INGEST_TOKEN_&lt;NAME&gt;"));
    }

    #[test]
    fn settings_modal_clean_when_no_warnings() {
        let consumers = vec![rec("prod-app", AuthRole::Full)];
        let html = render_settings_modal(&consumers, &[]).into_string();
        assert!(html.contains("● clean"));
        assert!(html.contains("parsed successfully"));
    }

    #[test]
    fn settings_modal_lists_warnings() {
        let warnings = vec![
            "DEPRECATED: env var 'INGEST_TOKEN' is set".to_string(),
            "env var 'INGEST_TOKEN_BAD' has unknown tier 'weird'".to_string(),
        ];
        let html = render_settings_modal(&[], &warnings).into_string();
        assert!(html.contains("2 issues"));
        assert!(html.contains("DEPRECATED"));
        assert!(html.contains("unknown tier"));
    }

    #[test]
    fn settings_modal_singular_issue_when_one_warning() {
        let warnings = vec!["one warning".to_string()];
        let html = render_settings_modal(&[], &warnings).into_string();
        assert!(html.contains("1 issue"));
        assert!(!html.contains("1 issues"), "should be singular for count of 1");
    }

    #[test]
    fn kbd_toast_lists_all_shortcuts() {
        let html = render_kbd_toast().into_string();
        assert!(html.contains("Keyboard shortcuts"));
        assert!(html.contains(">j<"));
        assert!(html.contains(">k<"));
        assert!(html.contains(">Enter<"));
        assert!(html.contains(">/<"));
        assert!(html.contains(">r<"));
        assert!(html.contains(">Esc<"));
        assert!(html.contains(">?<"));
        assert!(html.contains("next row"));
        assert!(html.contains("focus search"));
    }
}
