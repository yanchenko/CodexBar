//! Cursor provider: **manual cookie only** (no Chromium DPAPI import in v1).
//!
//! Requires `cookieSource=manual` + non-empty `cookieHeader` (or `manualCookieHeader`)
//! in config. Probes `GET /api/usage-summary` with Cookie header.

use crate::common::{
    self, auth_missing, clamp_percent, http_error, json_f64, network_error, parse_error,
};
use crate::ProbeContext;
use ab_http::HttpClient;
use ab_model::{CursorRequests, ProviderSnapshot, RateWindow};
use serde_json::Value;

const ID: &str = "cursor";
const DEFAULT_BASE: &str = "https://cursor.com";

/// Extract manual cookie from provider config object.
pub fn manual_cookie(cfg: &Value) -> Option<String> {
    let source = cfg
        .get("cookieSource")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    // Accept missing source only if cookieHeader present? Design: cookieSource=manual required.
    let header = cfg
        .get("cookieHeader")
        .or_else(|| cfg.get("manualCookieHeader"))
        .and_then(|v| v.as_str())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());

    match (source.as_str(), header) {
        ("manual", Some(h)) => Some(h),
        ("", Some(h)) => {
            // Strict: still require cookieSource=manual per design gate.
            let _ = h;
            None
        }
        _ => None,
    }
}

/// Map Cursor `/api/usage-summary` JSON → snapshot.
pub fn map_usage_summary(
    json: &Value,
    updated_at: &str,
    account_email: Option<String>,
) -> Result<ProviderSnapshot, String> {
    let individual = json.get("individualUsage");
    let plan = individual.and_then(|i| i.get("plan"));
    let overall = individual.and_then(|i| i.get("overall"));
    let team = json.get("teamUsage");
    let pooled = team.and_then(|t| t.get("pooled"));

    let plan_used = plan
        .and_then(|p| p.get("used"))
        .and_then(json_f64)
        .unwrap_or(0.0);
    let plan_limit = plan
        .and_then(|p| p.get("limit"))
        .and_then(json_f64)
        .unwrap_or(0.0);
    let auto_pct = plan
        .and_then(|p| p.get("autoPercentUsed"))
        .and_then(json_f64)
        .map(clamp_percent);
    let api_pct = plan
        .and_then(|p| p.get("apiPercentUsed"))
        .and_then(json_f64)
        .map(clamp_percent);
    let total_pct = plan
        .and_then(|p| p.get("totalPercentUsed"))
        .and_then(json_f64)
        .map(clamp_percent);

    let plan_percent = if let Some(t) = total_pct {
        t
    } else if let (Some(a), Some(b)) = (auto_pct, api_pct) {
        clamp_percent((a + b) / 2.0)
    } else if let Some(b) = api_pct {
        b
    } else if let Some(a) = auto_pct {
        a
    } else if plan_limit > 0.0 {
        clamp_percent((plan_used / plan_limit) * 100.0)
    } else if let (Some(used), Some(limit)) = (
        overall.and_then(|o| o.get("used")).and_then(json_f64),
        overall.and_then(|o| o.get("limit")).and_then(json_f64),
    ) {
        if limit > 0.0 {
            clamp_percent((used / limit) * 100.0)
        } else {
            0.0
        }
    } else if let (Some(used), Some(limit)) = (
        pooled.and_then(|o| o.get("used")).and_then(json_f64),
        pooled.and_then(|o| o.get("limit")).and_then(json_f64),
    ) {
        if limit > 0.0 {
            clamp_percent((used / limit) * 100.0)
        } else {
            0.0
        }
    } else {
        0.0
    };

    let billing_end = json
        .get("billingCycleEnd")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    let mut primary = RateWindow::new(plan_percent);
    primary.resets_at = billing_end.clone();
    if let Some(ref end) = billing_end {
        primary.reset_description = Some(format!("cycle ends {end}"));
    }

    let mut snap = ProviderSnapshot::ok(ID, updated_at);
    snap.source_label = Some("web".into());
    snap.primary = Some(primary);
    if let Some(a) = auto_pct {
        let mut w = RateWindow::new(a);
        w.resets_at = billing_end.clone();
        snap.secondary = Some(w);
    }
    if let Some(a) = api_pct {
        let mut w = RateWindow::new(a);
        w.resets_at = billing_end;
        snap.tertiary = Some(w);
    }
    snap.account_label = account_email.or_else(|| {
        json.get("membershipType")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
    });
    snap.data_confidence = Some("exact".into());

    // Optional request units (legacy) — only if present on fixture via nested usage.
    if let Some(req) = json.get("requestUsage").or_else(|| json.get("gpt-4")) {
        let used = req
            .get("numRequestsTotal")
            .or_else(|| req.get("numRequests"))
            .or_else(|| req.get("used"))
            .and_then(json_f64);
        let incl = req
            .get("maxRequestUsage")
            .or_else(|| req.get("included"))
            .and_then(json_f64);
        if used.is_some() || incl.is_some() {
            let remaining = match (incl, used) {
                (Some(i), Some(u)) => Some((i - u).max(0.0)),
                _ => None,
            };
            snap.cursor_requests = Some(CursorRequests {
                included: incl,
                used,
                remaining,
            });
        }
    }

    Ok(snap)
}

pub fn fetch_usage_summary(
    client: &HttpClient,
    base: &str,
    cookie_header: &str,
) -> Result<Value, String> {
    let base = base.trim_end_matches('/');
    let url = format!("{base}/api/usage-summary");
    let headers = [
        ("Cookie", cookie_header),
        ("Accept", "application/json"),
        ("User-Agent", "AgentBar"),
    ];
    let resp = client.get(&url, &headers).map_err(|e| e.to_string())?;
    if resp.status == 401 || resp.status == 403 {
        return Err("unauthorized".into());
    }
    if !resp.is_success() {
        return Err(format!("http:{}", resp.status));
    }
    serde_json::from_slice(&resp.body).map_err(|e| format!("parse: {e}"))
}

/// Probe Cursor (manual cookie only).
pub fn probe(ctx: &ProbeContext) -> ProviderSnapshot {
    let updated = ctx.updated_at.as_str();
    let cookie = match manual_cookie(&ctx.provider_cfg) {
        Some(c) => c,
        None => {
            let mut s = auth_missing(
                ID,
                updated,
                "Cursor cookie not configured (cookieSource=manual required)",
            );
            s.source_label = Some("web".into());
            return s;
        }
    };

    // Test fixture injection
    if let Some(fixture) = &ctx.endpoints.cursor_usage_fixture_json {
        match serde_json::from_str::<Value>(fixture) {
            Ok(json) => match map_usage_summary(&json, updated, None) {
                Ok(mut s) => {
                    s.enabled = true;
                    return s;
                }
                Err(e) => return parse_error(ID, updated, &e),
            },
            Err(e) => return parse_error(ID, updated, &e.to_string()),
        }
    }

    let base = ctx
        .endpoints
        .cursor_base
        .as_deref()
        .unwrap_or(DEFAULT_BASE);

    match fetch_usage_summary(&ctx.http, base, &cookie) {
        Ok(json) => match map_usage_summary(&json, updated, None) {
            Ok(mut s) => {
                s.enabled = true;
                s
            }
            Err(e) => parse_error(ID, updated, &format!("Cursor usage: {e}")),
        },
        Err(e) if e == "unauthorized" => {
            let mut s = ProviderSnapshot::failed(
                ID,
                updated,
                "Cursor cookie rejected (not logged in). Update manual cookie.",
                Some(common::ERR_AUTH_EXPIRED.into()),
            );
            s.source_label = Some("web".into());
            s
        }
        Err(e) if e.starts_with("http:") => {
            let status: u16 = e.trim_start_matches("http:").parse().unwrap_or(0);
            let mut s = http_error(ID, updated, status, "");
            s.source_label = Some("web".into());
            s
        }
        Err(e) => {
            let mut s = network_error(ID, updated, &format!("Cursor: {e}"));
            s.source_label = Some("web".into());
            s
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use httpmock::prelude::*;

    #[test]
    fn manual_cookie_requires_source() {
        let cfg = serde_json::json!({
            "cookieHeader": "WorkosCursorSessionToken=abc"
        });
        assert!(manual_cookie(&cfg).is_none());

        let cfg = serde_json::json!({
            "cookieSource": "manual",
            "cookieHeader": "WorkosCursorSessionToken=abc"
        });
        assert_eq!(
            manual_cookie(&cfg).as_deref(),
            Some("WorkosCursorSessionToken=abc")
        );
    }

    #[test]
    fn map_usage_summary_percent() {
        let json = serde_json::json!({
            "billingCycleEnd": "2026-08-01T00:00:00.000Z",
            "membershipType": "pro",
            "individualUsage": {
                "plan": {
                    "used": 2000,
                    "limit": 10000,
                    "totalPercentUsed": 20.0,
                    "autoPercentUsed": 15.0,
                    "apiPercentUsed": 25.0
                }
            }
        });
        let snap = map_usage_summary(&json, "t", Some("u@c.com".into())).unwrap();
        assert_eq!(snap.primary.as_ref().unwrap().used_percent, 20.0);
        assert_eq!(snap.secondary.as_ref().unwrap().used_percent, 15.0);
        assert_eq!(snap.account_label.as_deref(), Some("u@c.com"));
        assert!(snap.error.is_none());
    }

    #[test]
    fn httpmock_usage_summary() {
        let server = MockServer::start();
        let m = server.mock(|when, then| {
            when.method(GET)
                .path("/api/usage-summary")
                .header("Cookie", "WorkosCursorSessionToken=test");
            then.status(200).body(
                r#"{
                  "membershipType":"pro",
                  "billingCycleEnd":"2026-08-01T00:00:00Z",
                  "individualUsage":{
                    "plan":{"used":500,"limit":2000,"totalPercentUsed":25.0}
                  }
                }"#,
            );
        });
        let client = HttpClient {
            connect_timeout: std::time::Duration::from_secs(2),
            read_timeout: std::time::Duration::from_secs(2),
            ..HttpClient::default()
        };
        let json =
            fetch_usage_summary(&client, &server.base_url(), "WorkosCursorSessionToken=test")
                .unwrap();
        m.assert();
        let snap = map_usage_summary(&json, "t", None).unwrap();
        assert_eq!(snap.primary.as_ref().unwrap().used_percent, 25.0);
    }

    #[test]
    fn probe_auth_missing_without_cookie() {
        let ctx = ProbeContext {
            updated_at: "t".into(),
            provider_cfg: serde_json::json!({"id":"cursor","enabled":true}),
            http: HttpClient::default(),
            endpoints: Default::default(),
        };
        let snap = probe(&ctx);
        assert_eq!(snap.error_code.as_deref(), Some("auth_missing"));
        assert!(
            snap.error
                .as_ref()
                .unwrap()
                .contains("cookieSource=manual")
        );
    }

    #[test]
    fn fixture_map() {
        let fixture = include_str!("../tests/fixtures/cursor_usage_summary.json");
        let json: Value = serde_json::from_str(fixture).unwrap();
        let snap = map_usage_summary(&json, "t", None).unwrap();
        assert!(snap.primary.is_some());
    }
}
