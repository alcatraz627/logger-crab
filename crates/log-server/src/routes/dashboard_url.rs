//! URL builders for the dashboard. Pure functions over `DashboardQuery` —
//! every dashboard navigation link funnels through these helpers so the
//! filter-preservation rules (drop cursor on filter change, bump page on
//! "older →", etc.) live in one place.
//!
//! Extracted from `dashboard.rs` to keep the rendering file focused on
//! Maud markup; URL state is tested independently here.

use chrono::{DateTime, NaiveDateTime, Utc};

use super::dashboard::DashboardQuery;

/// Returns the (k, v) pairs that should appear in any URL preserving the
/// current filter state. Empty `Some("")` values — produced when the GET
/// form submits with blank inputs — are dropped here so they don't pollute
/// the URL or the active-filter chip strip with bogus "key=" entries.
///
/// Notably **omits cursor**: pagination state is managed separately by
/// [`build_next_url_with_page`] and [`reset_to_first_page`]. Including it
/// here would mean filter changes accidentally carry the cursor.
pub(crate) fn current_pairs(q: &DashboardQuery) -> Vec<(&'static str, String)> {
    let mut pairs: Vec<(&'static str, String)> = Vec::new();
    let push =
        |pairs: &mut Vec<(&'static str, String)>, k: &'static str, v: &Option<String>| {
            if let Some(s) = v {
                if !s.is_empty() {
                    pairs.push((k, s.clone()));
                }
            }
        };
    push(&mut pairs, "request_id", &q.request_id);
    push(&mut pairs, "service", &q.service);
    push(&mut pairs, "env", &q.env);
    push(&mut pairs, "event_prefix", &q.event_prefix);
    push(&mut pairs, "level", &q.level);
    push(&mut pairs, "q", &q.q);
    push(&mut pairs, "since", &q.since);
    push(&mut pairs, "until", &q.until);
    if let Some(v) = q.limit {
        pairs.push(("limit", v.to_string()));
    }
    // page > 1 only — page=1 is the default and shouldn't pollute URLs.
    if let Some(p) = q.page {
        if p > 1 {
            pairs.push(("page", p.to_string()));
        }
    }
    pairs
}

/// Replace one filter key's value, preserving everything else.
pub(crate) fn filter_url_override(q: &DashboardQuery, key: &str, value: &str) -> String {
    let mut pairs: Vec<(&'static str, String)> =
        current_pairs(q).into_iter().filter(|(k, _)| *k != key).collect();
    let static_key: &'static str = match key {
        "request_id" => "request_id",
        "service" => "service",
        "env" => "env",
        "event_prefix" => "event_prefix",
        "level" => "level",
        "q" => "q",
        "since" => "since",
        "until" => "until",
        _ => return pairs_to_url(&pairs),
    };
    pairs.push((static_key, value.to_string()));
    pairs_to_url(&pairs)
}

/// Replace one filter key's value where the value is u32 (e.g. limit).
pub(crate) fn filter_url_override_u32(q: &DashboardQuery, key: &str, value: u32) -> String {
    let mut pairs: Vec<(&'static str, String)> =
        current_pairs(q).into_iter().filter(|(k, _)| *k != key).collect();
    let static_key: &'static str = match key {
        "limit" => "limit",
        _ => return pairs_to_url(&pairs),
    };
    pairs.push((static_key, value.to_string()));
    pairs_to_url(&pairs)
}

/// Drop one filter key without touching the others. Used by the active-filter
/// chip strip's `×` button.
pub(crate) fn filter_url_remove(q: &DashboardQuery, key: &str) -> String {
    let pairs: Vec<(&'static str, String)> =
        current_pairs(q).into_iter().filter(|(k, _)| *k != key).collect();
    pairs_to_url(&pairs)
}

/// "← newest" URL: drop both cursor + page so the user lands on page 1.
pub(crate) fn reset_to_first_page(q: &DashboardQuery) -> String {
    let pairs: Vec<(&'static str, String)> = current_pairs(q)
        .into_iter()
        .filter(|(k, _)| *k != "cursor" && *k != "page")
        .collect();
    pairs_to_url(&pairs)
}

/// "older →" URL: bump the page counter, swap the cursor.
pub(crate) fn build_next_url_with_page(q: &DashboardQuery, cursor: &str) -> String {
    let next_page = q.page.unwrap_or(1) + 1;
    let mut pairs: Vec<(&'static str, String)> = current_pairs(q)
        .into_iter()
        .filter(|(k, _)| *k != "cursor" && *k != "page")
        .collect();
    pairs.push(("cursor", cursor.to_string()));
    pairs.push(("page", next_page.to_string()));
    pairs_to_url(&pairs)
}

/// Encode `(k, v)` pairs into `/?k=v&k=v...` with percent-encoded values.
/// Returns `/` (no query) when the pairs slice is empty.
pub(crate) fn pairs_to_url(pairs: &[(&str, String)]) -> String {
    if pairs.is_empty() {
        return "/".to_string();
    }
    let query: String = pairs
        .iter()
        .map(|(k, v)| format!("{k}={}", urlenc(v)))
        .collect::<Vec<_>>()
        .join("&");
    format!("/?{query}")
}

/// Minimal RFC3986 percent-encoder. Reserves the unreserved set
/// (`A-Z a-z 0-9 - _ . ~`) and percent-encodes everything else.
pub(crate) fn urlenc(s: &str) -> String {
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

/// Parse the browser's datetime-local string into a UTC `DateTime`.
/// Browsers post `2026-05-09T03:30` (no seconds, no offset) — we treat as UTC.
/// Returns None for empty/missing/malformed input.
pub(crate) fn parse_dashboard_dt(s: Option<&str>) -> Option<DateTime<Utc>> {
    let s = s?.trim();
    if s.is_empty() {
        return None;
    }
    // Try with and without seconds.
    for fmt in ["%Y-%m-%dT%H:%M:%S", "%Y-%m-%dT%H:%M"] {
        if let Ok(naive) = NaiveDateTime::parse_from_str(s, fmt) {
            return Some(naive.and_utc());
        }
    }
    None
}

/// Build a `/logs/download.ndjson?...` URL preserving the current filter
/// state so the download contains exactly what the user is looking at.
pub(crate) fn download_url(q: &DashboardQuery) -> String {
    let mut pairs = current_pairs(q);
    // Strip cursor + page so the download is the FILTERED set from the
    // start, not a single-page slice. Caller can still page through the
    // dashboard normally.
    pairs.retain(|(k, _)| *k != "cursor" && *k != "page");
    let qs = pairs_to_url(&pairs);
    if qs == "/" {
        "/logs/download.ndjson".to_string()
    } else {
        format!("/logs/download.ndjson{}", qs.trim_start_matches('/'))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn q_empty() -> DashboardQuery {
        DashboardQuery::default()
    }

    fn q_with(
        svc: Option<&str>,
        env: Option<&str>,
        cursor: Option<&str>,
        page: Option<u32>,
    ) -> DashboardQuery {
        DashboardQuery {
            service: svc.map(String::from),
            env: env.map(String::from),
            cursor: cursor.map(String::from),
            page,
            ..Default::default()
        }
    }

    #[test]
    fn current_pairs_skips_empty_strings() {
        let q = q_with(Some(""), Some("dev"), None, None);
        let pairs = current_pairs(&q);
        assert_eq!(pairs.len(), 1);
        assert_eq!(pairs[0], ("env", "dev".into()));
    }

    #[test]
    fn current_pairs_preserves_filters_and_page() {
        let q = q_with(Some("api"), Some("prod"), Some("c1"), Some(3));
        let pairs = current_pairs(&q);
        assert!(pairs.contains(&("service", "api".into())));
        assert!(pairs.contains(&("env", "prod".into())));
        assert!(pairs.contains(&("page", "3".into())));
        // Cursor is intentionally NOT in current_pairs.
        assert!(!pairs.iter().any(|(k, _)| *k == "cursor"));
    }

    #[test]
    fn current_pairs_omits_page_when_one() {
        let q = q_with(Some("api"), None, None, Some(1));
        let pairs = current_pairs(&q);
        assert!(!pairs.iter().any(|(k, _)| *k == "page"));
    }

    #[test]
    fn current_pairs_includes_date_range() {
        let mut q = DashboardQuery::default();
        q.since = Some("2026-05-01T00:00".into());
        q.until = Some("2026-05-09T00:00".into());
        let pairs = current_pairs(&q);
        assert!(pairs.contains(&("since", "2026-05-01T00:00".into())));
        assert!(pairs.contains(&("until", "2026-05-09T00:00".into())));
    }

    #[test]
    fn filter_url_override_replaces_existing_value() {
        let q = q_with(Some("old-svc"), Some("dev"), None, None);
        let url = filter_url_override(&q, "service", "new-svc");
        assert!(url.contains("service=new-svc"));
        assert!(!url.contains("old-svc"));
        assert!(url.contains("env=dev"), "env should be preserved");
    }

    #[test]
    fn filter_url_remove_drops_just_one_key() {
        let q = q_with(Some("api"), Some("prod"), Some("c1"), None);
        let url = filter_url_remove(&q, "cursor");
        assert!(url.contains("service=api"));
        assert!(url.contains("env=prod"));
        assert!(!url.contains("cursor"));
    }

    #[test]
    fn reset_to_first_page_drops_cursor_and_page() {
        let q = q_with(Some("api"), None, Some("c1"), Some(7));
        let url = reset_to_first_page(&q);
        assert!(url.contains("service=api"));
        assert!(!url.contains("cursor"));
        assert!(!url.contains("page"));
    }

    #[test]
    fn build_next_url_with_page_increments() {
        let q = q_with(Some("api"), None, Some("old-cursor"), Some(2));
        let url = build_next_url_with_page(&q, "new-cursor");
        assert!(url.contains("cursor=new-cursor"));
        assert!(url.contains("page=3"));
        assert!(!url.contains("old-cursor"));
        assert!(!url.contains("page=2"));
    }

    #[test]
    fn build_next_url_with_page_starts_at_2_from_default() {
        let q = q_empty();
        let url = build_next_url_with_page(&q, "c1");
        assert!(url.contains("page=2"));
    }

    #[test]
    fn pairs_to_url_returns_root_when_empty() {
        assert_eq!(pairs_to_url(&[]), "/");
    }

    #[test]
    fn pairs_to_url_urlencodes_values() {
        let pairs = vec![("q", "hello world".to_string())];
        let url = pairs_to_url(&pairs);
        assert!(url.contains("hello%20world"));
    }

    #[test]
    fn parse_dashboard_dt_handles_browser_format() {
        let parsed = parse_dashboard_dt(Some("2026-05-09T03:30")).unwrap();
        assert_eq!(parsed.format("%Y-%m-%d %H:%M:%S").to_string(), "2026-05-09 03:30:00");
    }

    #[test]
    fn parse_dashboard_dt_handles_seconds_form() {
        let parsed = parse_dashboard_dt(Some("2026-05-09T03:30:45")).unwrap();
        assert_eq!(parsed.format("%H:%M:%S").to_string(), "03:30:45");
    }

    #[test]
    fn parse_dashboard_dt_returns_none_on_garbage() {
        assert!(parse_dashboard_dt(Some("not a date")).is_none());
        assert!(parse_dashboard_dt(Some("")).is_none());
        assert!(parse_dashboard_dt(None).is_none());
        assert!(parse_dashboard_dt(Some("   ")).is_none());
    }

    #[test]
    fn download_url_strips_cursor_and_page() {
        let q = q_with(Some("api"), None, Some("c1"), Some(3));
        let url = download_url(&q);
        assert!(url.starts_with("/logs/download.ndjson"));
        assert!(url.contains("service=api"));
        assert!(!url.contains("cursor"));
        assert!(!url.contains("page"));
    }

    #[test]
    fn download_url_no_filters_returns_bare_path() {
        let url = download_url(&DashboardQuery::default());
        assert_eq!(url, "/logs/download.ndjson");
    }
}
