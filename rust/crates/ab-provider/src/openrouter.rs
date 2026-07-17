//! OpenRouter API-key provider (first post-MVP expansion wave).
//!
//! Auth: config `apiKey` only (never put into snapshot).
//! Data: `GET /api/v1/credits` → balance; optional `GET /api/v1/key` for rate limits.
//! Hosts need **no** changes — registry catalog + probe path pick it up.

use crate::common::{self, auth_missing, clamp_percent, http_error, json_f64, parse_error};
use crate::ProbeContext;
use ab_http::HttpClient;
use ab_model::{ProviderSnapshot, RateWindow};
use serde_json::Value;

const ID: &str = "openrouter";
const DEFAULT_BASE: &str = "https://openrouter.ai/api/v1";

fn api_key(cfg: &Value) -> Option<String> {
    cfg.get("apiKey")
        .and_then(|v| v.as_str())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

/// Resolve OpenRouter API base.
///
/// Config `apiBase` / `baseUrl` are **ignored** in MVP (AB-004 SSRF): only the
/// hard-coded OpenRouter host is used in production. Test-only injection goes
/// through [`crate::EndpointOverrides::openrouter_base`].
fn base_url(_cfg: &Value, override_base: Option<&str>) -> String {
    if let Some(b) = override_base {
        return b.trim_end_matches('/').to_string();
    }
    DEFAULT_BASE.into()
}

/// Map OpenRouter credits (+ optional key) JSON into a snapshot.
pub fn map_credits_response(
    credits: &Value,
    key: Option<&Value>,
    updated_at: &str,
) -> Result<ProviderSnapshot, String> {
    let data = credits.get("data").unwrap_or(credits);
    let total_credits = data
        .get("total_credits")
        .or_else(|| data.get("totalCredits"))
        .and_then(json_f64)
        .unwrap_or(0.0);
    let total_usage = data
        .get("total_usage")
        .or_else(|| data.get("totalUsage"))
        .and_then(json_f64)
        .unwrap_or(0.0);
    let balance = (total_credits - total_usage).max(0.0);

    let mut snap = ProviderSnapshot::ok(ID, updated_at);
    snap.source_label = Some("api".into());
    snap.credits_remaining = Some(balance);
    snap.data_confidence = Some("exact".into());

    // Key limit → primary percent when present.
    if let Some(k) = key {
        let kdata = k.get("data").unwrap_or(k);
        let limit = kdata
            .get("limit")
            .and_then(json_f64)
            .or_else(|| kdata.get("rate_limit").and_then(|r| r.get("requests")).and_then(json_f64));
        let usage = kdata
            .get("usage")
            .and_then(json_f64)
            .or_else(|| kdata.get("limit_remaining").and_then(json_f64).map(|rem| {
                limit.map(|l| (l - rem).max(0.0)).unwrap_or(0.0)
            }));
        if let (Some(lim), Some(used)) = (limit, usage) {
            if lim > 0.0 {
                let mut w = RateWindow::new(clamp_percent((used / lim) * 100.0));
                w.reset_description = Some("API key limit".into());
                snap.primary = Some(w);
            }
        } else if let Some(pct) = kdata.get("usage_daily").and_then(json_f64) {
            // Some responses expose daily spend only — surface as secondary.
            let mut w = RateWindow::new(0.0);
            w.reset_description = Some(format!("daily spend ${pct:.2}"));
            snap.secondary = Some(w);
        }
    }

    // If no key window, show usage ratio of total credits as soft primary.
    if snap.primary.is_none() && total_credits > 0.0 {
        let mut w = RateWindow::new(clamp_percent((total_usage / total_credits) * 100.0));
        w.reset_description = Some("of purchased credits".into());
        snap.primary = Some(w);
    }

    Ok(snap)
}

pub fn fetch_credits(client: &HttpClient, base: &str, api_key: &str) -> Result<Value, String> {
    let url = format!("{}/credits", base.trim_end_matches('/'));
    let auth = format!("Bearer {api_key}");
    let headers = [
        ("Authorization", auth.as_str()),
        ("Accept", "application/json"),
        ("HTTP-Referer", "https://github.com/yanchenko/AgentBar"),
        ("X-Title", "AgentBar"),
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

pub fn fetch_key(client: &HttpClient, base: &str, api_key: &str) -> Result<Value, String> {
    let url = format!("{}/key", base.trim_end_matches('/'));
    let auth = format!("Bearer {api_key}");
    let headers = [
        ("Authorization", auth.as_str()),
        ("Accept", "application/json"),
    ];
    let resp = client.get(&url, &headers).map_err(|e| e.to_string())?;
    if !resp.is_success() {
        return Err(format!("http:{}", resp.status));
    }
    serde_json::from_slice(&resp.body).map_err(|e| format!("parse: {e}"))
}

/// Probe OpenRouter (API key from config).
pub fn probe(ctx: &ProbeContext) -> ProviderSnapshot {
    let updated = ctx.updated_at.as_str();
    let Some(key) = api_key(&ctx.provider_cfg) else {
        return auth_missing(
            ID,
            updated,
            "OpenRouter API key not set. Add providers[].apiKey for id=openrouter.",
        );
    };
    let base = base_url(
        &ctx.provider_cfg,
        ctx.endpoints.openrouter_base.as_deref(),
    );

    // Test fixture injection
    if let Some(fixture) = &ctx.endpoints.openrouter_credits_fixture_json {
        if let Ok(json) = serde_json::from_str::<Value>(fixture) {
            return match map_credits_response(&json, None, updated) {
                Ok(mut s) => {
                    s.enabled = true;
                    s
                }
                Err(e) => parse_error(ID, updated, &e),
            };
        }
    }

    match fetch_credits(&ctx.http, &base, &key) {
        Ok(credits) => {
            let key_json = fetch_key(&ctx.http, &base, &key).ok();
            match map_credits_response(&credits, key_json.as_ref(), updated) {
                Ok(mut s) => {
                    s.enabled = true;
                    s
                }
                Err(e) => parse_error(ID, updated, &format!("OpenRouter: {e}")),
            }
        }
        Err(e) if e == "unauthorized" => {
            let mut s = ProviderSnapshot::failed(
                ID,
                updated,
                "OpenRouter API key rejected (401/403).",
                Some(common::ERR_AUTH_EXPIRED.into()),
            );
            s.source_label = Some("api".into());
            s
        }
        Err(e) if e.starts_with("http:") => {
            let status: u16 = e.trim_start_matches("http:").parse().unwrap_or(0);
            let mut s = http_error(ID, updated, status, "");
            s.source_label = Some("api".into());
            s
        }
        Err(e) => {
            ab_log::info("openrouter", &format!("credits failed: {e}"));
            common::network_error(ID, updated, "OpenRouter credits request failed")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use httpmock::prelude::*;
    use std::time::Duration;

    #[test]
    fn map_credits_balance() {
        let json = serde_json::json!({
            "data": { "total_credits": 100.0, "total_usage": 25.5 }
        });
        let snap = map_credits_response(&json, None, "t").unwrap();
        assert_eq!(snap.credits_remaining, Some(74.5));
        assert!(snap.primary.is_some());
        assert!(snap.error.is_none());
        let s = serde_json::to_string(&snap).unwrap();
        assert!(!s.contains("apiKey"));
        assert!(!s.contains("sk-"));
    }

    #[test]
    fn httpmock_credits() {
        let server = MockServer::start();
        let m = server.mock(|when, then| {
            when.method(GET)
                .path("/credits")
                .header("Authorization", "Bearer sk-or-test");
            then.status(200)
                .body(r#"{"data":{"total_credits":50,"total_usage":10}}"#);
        });
        let client = HttpClient {
            connect_timeout: Duration::from_secs(2),
            read_timeout: Duration::from_secs(2),
            ..HttpClient::default()
        };
        let json = fetch_credits(&client, &server.base_url(), "sk-or-test").unwrap();
        m.assert();
        let snap = map_credits_response(&json, None, "t").unwrap();
        assert_eq!(snap.credits_remaining, Some(40.0));
    }

    #[test]
    fn probe_auth_missing() {
        let ctx = ProbeContext {
            updated_at: "t".into(),
            provider_cfg: serde_json::json!({"id":"openrouter","enabled":true}),
            http: HttpClient {
                connect_timeout: Duration::from_millis(50),
                read_timeout: Duration::from_millis(50),
                ..HttpClient::default()
            },
            endpoints: Default::default(),
        };
        let snap = probe(&ctx);
        assert_eq!(snap.error_code.as_deref(), Some("auth_missing"));
    }
}
