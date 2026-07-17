//! Claude provider: file credentials / API key / CLI (no Keychain on Windows/Linux).
//!
//! PR7: defaultEnabled false; structured auth_missing without crash.
//!
//! Order (Win/Linux): `~/.claude/.credentials.json` → config `apiKey` → `claude` CLI presence.
//! `defaultEnabled` is false (catalog). Never puts tokens into snapshots.

use crate::common::{self, auth_missing, clamp_percent, http_error, json_f64, parse_error};
use crate::ProbeContext;
use ab_http::HttpClient;
use ab_model::{ProviderSnapshot, RateWindow};
use serde_json::Value;
use std::fs;
use std::path::PathBuf;
use std::time::Duration;

const ID: &str = "claude";
const DEFAULT_USAGE_BASE: &str = "https://api.anthropic.com";
const DEFAULT_TOKEN_URL: &str = "https://platform.claude.com/v1/oauth/token";
const OAUTH_CLIENT_ID: &str = "9d1c250a-e61b-44d9-88ed-5944d1962f5e";
const BETA_HEADER: &str = "oauth-2025-04-20";
const UA: &str = "claude-code/2.1.0";

#[derive(Clone, Debug)]
pub struct ClaudeCredentials {
    pub access_token: String,
    pub refresh_token: Option<String>,
    pub expires_at_ms: Option<f64>,
    pub rate_limit_tier: Option<String>,
    pub subscription_type: Option<String>,
}

impl ClaudeCredentials {
    pub fn is_expired(&self) -> bool {
        match self.expires_at_ms {
            None => false, // unknown expiry — try use
            Some(ms) => {
                let now_ms = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_millis() as f64)
                    .unwrap_or(0.0);
                now_ms >= ms
            }
        }
    }
}

/// `~/.claude/.credentials.json`
pub fn credentials_path() -> Option<PathBuf> {
    common::home_dir().map(|h| h.join(".claude").join(".credentials.json"))
}

/// Parse Claude credentials file (`claudeAiOauth` object).
pub fn parse_credentials_json(data: &str) -> Result<ClaudeCredentials, String> {
    let root: Value =
        serde_json::from_str(data).map_err(|e| format!("invalid credentials JSON: {e}"))?;
    if root.get("claudeAiOauth").is_none() && root.get("mcpOAuth").is_some() {
        return Err("mcp_oauth_only".into());
    }
    let oauth = root
        .get("claudeAiOauth")
        .ok_or_else(|| "missing_oauth".to_string())?;
    let access = oauth
        .get("accessToken")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim()
        .to_string();
    if access.is_empty() {
        return Err("missing_access_token".into());
    }
    let refresh = oauth
        .get("refreshToken")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let expires_at_ms = oauth.get("expiresAt").and_then(json_f64);
    Ok(ClaudeCredentials {
        access_token: access,
        refresh_token: refresh,
        expires_at_ms,
        rate_limit_tier: oauth
            .get("rateLimitTier")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string()),
        subscription_type: oauth
            .get("subscriptionType")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string()),
    })
}

pub fn load_file_credentials() -> Result<ClaudeCredentials, String> {
    let path = credentials_path().ok_or_else(|| "no home".to_string())?;
    if !path.is_file() {
        return Err("not_found".into());
    }
    let data = fs::read_to_string(&path).map_err(|e| e.to_string())?;
    parse_credentials_json(&data)
}

/// Refresh OAuth token (form-urlencoded). Write-back is best-effort.
pub fn refresh_token(
    client: &HttpClient,
    token_url: &str,
    creds: &ClaudeCredentials,
) -> Result<ClaudeCredentials, String> {
    let refresh = creds
        .refresh_token
        .as_deref()
        .filter(|s| !s.is_empty())
        .ok_or_else(|| "no_refresh_token".to_string())?;
    let body = format!(
        "grant_type=refresh_token&refresh_token={}&client_id={}",
        urlencoding_lite(refresh),
        urlencoding_lite(OAUTH_CLIENT_ID)
    );
    let resp = client
        .post(
            token_url,
            &[
                ("Content-Type", "application/x-www-form-urlencoded"),
                ("Accept", "application/json"),
            ],
            body.as_bytes(),
        )
        .map_err(|e| e.to_string())?;
    if !resp.is_success() {
        return Err(format!("refresh HTTP {}", resp.status));
    }
    let json: Value =
        serde_json::from_slice(&resp.body).map_err(|e| format!("refresh JSON: {e}"))?;
    let access = json
        .get("access_token")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "refresh missing access_token".to_string())?
        .to_string();
    let new_refresh = json
        .get("refresh_token")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .or_else(|| creds.refresh_token.clone());
    let expires_in = json.get("expires_in").and_then(json_f64).unwrap_or(3600.0);
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as f64)
        .unwrap_or(0.0);
    Ok(ClaudeCredentials {
        access_token: access,
        refresh_token: new_refresh,
        expires_at_ms: Some(now_ms + expires_in * 1000.0),
        rate_limit_tier: creds.rate_limit_tier.clone(),
        subscription_type: creds.subscription_type.clone(),
    })
}

fn urlencoding_lite(s: &str) -> String {
    let mut out = String::with_capacity(s.len() * 2);
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char);
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// Map Anthropic OAuth usage JSON (`five_hour` / `seven_day` windows).
pub fn map_usage_response(json: &Value, updated_at: &str, source: &str) -> Result<ProviderSnapshot, String> {
    let five = window_from(json.get("five_hour"), 5 * 60);
    let seven = window_from(json.get("seven_day"), 7 * 24 * 60);

    if five.is_none() && seven.is_none() {
        // Try limits array
        if let Some(limits) = json.get("limits").and_then(|v| v.as_array()) {
            let mut primary = None;
            let mut secondary = None;
            for entry in limits {
                let pct = entry
                    .get("percent")
                    .or_else(|| entry.get("utilization"))
                    .and_then(json_f64)
                    .map(clamp_percent);
                let Some(pct) = pct else { continue };
                let resets = entry
                    .get("resets_at")
                    .or_else(|| entry.get("resetsAt"))
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string());
                let mut w = RateWindow::new(pct);
                w.resets_at = resets;
                let group = entry.get("group").and_then(|v| v.as_str()).unwrap_or("");
                if primary.is_none() && (group == "session" || group.is_empty()) {
                    primary = Some(w);
                } else if secondary.is_none() {
                    secondary = Some(w);
                }
            }
            if primary.is_some() || secondary.is_some() {
                let mut snap = ProviderSnapshot::ok(ID, updated_at);
                snap.source_label = Some(source.into());
                snap.primary = primary;
                snap.secondary = secondary;
                snap.data_confidence = Some("exact".into());
                return Ok(snap);
            }
        }
        return Err("no rate windows in Claude usage response".into());
    }

    let mut snap = ProviderSnapshot::ok(ID, updated_at);
    snap.source_label = Some(source.into());
    // Null five_hour → synthetic placeholder (design Annex B).
    snap.primary = match five {
        Some(w) => Some(w),
        None => {
            let mut w = RateWindow::new(0.0);
            w.window_minutes = Some(300);
            w.is_synthetic_placeholder = true;
            Some(w)
        }
    };
    snap.secondary = seven;
    snap.data_confidence = Some("exact".into());
    Ok(snap)
}

fn window_from(v: Option<&Value>, default_minutes: i64) -> Option<RateWindow> {
    let w = v?;
    if w.is_null() {
        return None;
    }
    let util = w.get("utilization").and_then(json_f64).map(clamp_percent)?;
    let resets = w
        .get("resets_at")
        .or_else(|| w.get("resetsAt"))
        .and_then(|x| x.as_str())
        .map(|s| s.to_string());
    let mut rw = RateWindow::new(util);
    rw.window_minutes = Some(default_minutes);
    rw.resets_at = resets.clone();
    if let Some(ref r) = resets {
        rw.reset_description = Some(format!("resets {r}"));
    }
    Some(rw)
}

pub fn fetch_usage_http(
    client: &HttpClient,
    usage_base: &str,
    access_token: &str,
) -> Result<Value, String> {
    let base = usage_base.trim_end_matches('/');
    let url = if base.contains("/api/oauth/usage") {
        base.to_string()
    } else {
        format!("{base}/api/oauth/usage")
    };
    let auth = format!("Bearer {access_token}");
    let headers = [
        ("Authorization", auth.as_str()),
        ("Accept", "application/json"),
        ("Content-Type", "application/json"),
        ("anthropic-beta", BETA_HEADER),
        ("User-Agent", UA),
    ];
    let resp = client.get(&url, &headers).map_err(|e| e.to_string())?;
    if resp.status == 401 {
        return Err("unauthorized".into());
    }
    if resp.status == 429 {
        return Err("rate_limited".into());
    }
    if !resp.is_success() {
        return Err(format!("http:{}", resp.status));
    }
    serde_json::from_slice(&resp.body).map_err(|e| format!("parse: {e}"))
}

/// Config `apiKey` / Admin-style key (never returned in snapshot).
fn config_api_key(cfg: &Value) -> Option<String> {
    cfg.get("apiKey")
        .or_else(|| cfg.get("adminApiKey"))
        .and_then(|v| v.as_str())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

/// Probe Claude.
pub fn probe(ctx: &ProbeContext) -> ProviderSnapshot {
    let updated = ctx.updated_at.as_str();
    let usage_base = ctx
        .endpoints
        .claude_usage_base
        .as_deref()
        .unwrap_or(DEFAULT_USAGE_BASE);
    let token_url = ctx
        .endpoints
        .claude_token_url
        .as_deref()
        .unwrap_or(DEFAULT_TOKEN_URL);

    // 1) File credentials
    match load_file_credentials() {
        Ok(mut creds) => {
            if creds.is_expired() {
                if let Ok(refreshed) = refresh_token(&ctx.http, token_url, &creds) {
                    // Best-effort write-back
                    if let Some(path) = credentials_path() {
                        let _ = write_back_credentials(&path, &refreshed);
                    }
                    creds = refreshed;
                }
            }
            match fetch_usage_http(&ctx.http, usage_base, &creds.access_token) {
                Ok(json) => match map_usage_response(&json, updated, "oauth") {
                    Ok(mut s) => {
                        s.enabled = true;
                        if let Some(sub) = creds.subscription_type {
                            s.account_label = Some(sub);
                        }
                        return s;
                    }
                    Err(e) => return parse_error(ID, updated, &format!("Claude usage: {e}")),
                },
                Err(e) if e == "unauthorized" => {
                    let mut s = ProviderSnapshot::failed(
                        ID,
                        updated,
                        "Claude OAuth request unauthorized. Run `claude` to re-authenticate.",
                        Some(common::ERR_AUTH_EXPIRED.into()),
                    );
                    s.source_label = Some("oauth".into());
                    return s;
                }
                Err(e) if e == "rate_limited" => {
                    return ProviderSnapshot::failed(
                        ID,
                        updated,
                        "Claude OAuth usage endpoint is rate limited. Wait a few minutes, then refresh.",
                        Some("rate_limited".into()),
                    );
                }
                Err(e) if e.starts_with("http:") => {
                    let status: u16 = e.trim_start_matches("http:").parse().unwrap_or(0);
                    let mut s = http_error(ID, updated, status, "");
                    s.source_label = Some("oauth".into());
                    return s;
                }
                Err(e) => {
                    ab_log::info("claude", &format!("oauth usage failed: {e}"));
                }
            }
        }
        Err(e) if e == "not_found" || e == "missing_oauth" || e == "mcp_oauth_only" => {
            // fall through
        }
        Err(e) => {
            ab_log::warn("claude", &format!("credentials read: {e}"));
        }
    }

    // 2) API key from config — usage endpoint still needs OAuth for /api/oauth/usage;
    //    surface structured message rather than crash. Admin API is out of MVP depth.
    if config_api_key(&ctx.provider_cfg).is_some() {
        let mut s = ProviderSnapshot::failed(
            ID,
            updated,
            "Claude API key present; OAuth usage endpoint requires Claude CLI login (file credentials).",
            Some(common::ERR_AUTH_MISSING.into()),
        );
        s.source_label = Some("api".into());
        // If httpmock injects a usage base for tests with a fake bearer, still allow file-less tests
        // to call map via fixture.
        if let Some(fixture) = &ctx.endpoints.claude_usage_fixture_json {
            if let Ok(json) = serde_json::from_str::<Value>(fixture) {
                if let Ok(mut snap) = map_usage_response(&json, updated, "api") {
                    snap.enabled = true;
                    return snap;
                }
            }
        }
        return s;
    }

    // 3) CLI presence probe (no PTY scrape as primary)
    if claude_cli_available() {
        let mut s = auth_missing(
            ID,
            updated,
            "Claude CLI found but no file credentials. Run `claude` login or set ~/.claude/.credentials.json.",
        );
        s.source_label = Some("cli".into());
        return s;
    }

    let mut s = auth_missing(
        ID,
        updated,
        "Claude credentials not found. Run `claude` to authenticate.",
    );
    s.source_label = Some("oauth".into());
    s
}

fn write_back_credentials(path: &std::path::Path, creds: &ClaudeCredentials) -> Result<(), String> {
    let mut root: Value = if path.is_file() {
        serde_json::from_str(&fs::read_to_string(path).map_err(|e| e.to_string())?)
            .unwrap_or_else(|_| serde_json::json!({}))
    } else {
        serde_json::json!({})
    };
    let obj = root.as_object_mut().ok_or("root")?;
    let mut oauth = serde_json::Map::new();
    oauth.insert("accessToken".into(), Value::String(creds.access_token.clone()));
    if let Some(r) = &creds.refresh_token {
        oauth.insert("refreshToken".into(), Value::String(r.clone()));
    }
    if let Some(ms) = creds.expires_at_ms {
        oauth.insert("expiresAt".into(), serde_json::json!(ms));
    }
    obj.insert("claudeAiOauth".into(), Value::Object(oauth));
    fs::write(path, serde_json::to_string_pretty(&root).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())
}

fn claude_cli_available() -> bool {
    ab_proc::run("claude", &["--version"], Duration::from_secs(3), 64 * 1024)
        .map(|o| o.success())
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use httpmock::prelude::*;
    use std::sync::Mutex;

    static LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn parse_claude_credentials() {
        let raw = r#"{
          "claudeAiOauth": {
            "accessToken": "sk-ant-oat-x",
            "refreshToken": "rt",
            "expiresAt": 9999999999999,
            "subscriptionType": "pro"
          }
        }"#;
        let c = parse_credentials_json(raw).unwrap();
        assert_eq!(c.access_token, "sk-ant-oat-x");
        assert!(!c.is_expired());
    }

    #[test]
    fn map_five_and_seven_day() {
        let json = serde_json::json!({
            "five_hour": { "utilization": 40.0, "resets_at": "2026-07-17T17:00:00Z" },
            "seven_day": { "utilization": 12.5, "resets_at": "2026-07-24T00:00:00Z" }
        });
        let snap = map_usage_response(&json, "t", "oauth").unwrap();
        assert_eq!(snap.primary.as_ref().unwrap().used_percent, 40.0);
        assert_eq!(snap.secondary.as_ref().unwrap().used_percent, 12.5);
        assert!(snap.error.is_none());
    }

    #[test]
    fn null_five_hour_synthetic() {
        let json = serde_json::json!({
            "five_hour": null,
            "seven_day": { "utilization": 5.0, "resets_at": "2026-07-24T00:00:00Z" }
        });
        let snap = map_usage_response(&json, "t", "oauth").unwrap();
        assert!(snap.primary.as_ref().unwrap().is_synthetic_placeholder);
        assert_eq!(snap.secondary.as_ref().unwrap().used_percent, 5.0);
    }

    #[test]
    fn usage_httpmock() {
        let server = MockServer::start();
        let m = server.mock(|when, then| {
            when.method(GET)
                .path("/api/oauth/usage")
                .header("Authorization", "Bearer tok");
            then.status(200).body(
                r#"{"five_hour":{"utilization":22.0,"resets_at":"2026-07-17T18:00:00Z"},
                   "seven_day":{"utilization":8.0,"resets_at":"2026-07-24T00:00:00Z"}}"#,
            );
        });
        let client = HttpClient {
            connect_timeout: Duration::from_secs(2),
            read_timeout: Duration::from_secs(2),
            ..HttpClient::default()
        };
        let json = fetch_usage_http(&client, &server.base_url(), "tok").unwrap();
        m.assert();
        let snap = map_usage_response(&json, "t", "oauth").unwrap();
        assert_eq!(snap.primary.as_ref().unwrap().used_percent, 22.0);
    }

    #[test]
    fn probe_auth_missing() {
        let _g = LOCK.lock().unwrap();
        // Point HOME at empty temp so credentials file missing.
        let tmp = tempfile::tempdir().unwrap();
        #[cfg(windows)]
        unsafe {
            std::env::set_var("USERPROFILE", tmp.path());
        }
        #[cfg(not(windows))]
        unsafe {
            std::env::set_var("HOME", tmp.path());
        }
        let ctx = ProbeContext {
            updated_at: "t".into(),
            provider_cfg: serde_json::json!({"id":"claude","enabled":true}),
            http: HttpClient {
                connect_timeout: Duration::from_millis(100),
                read_timeout: Duration::from_millis(100),
                ..HttpClient::default()
            },
            endpoints: Default::default(),
        };
        let snap = probe(&ctx);
        assert_eq!(snap.error_code.as_deref(), Some("auth_missing"));
        assert!(snap.error.is_some());
    }

    #[test]
    fn fixture_map() {
        let fixture = include_str!("../tests/fixtures/claude_usage.json");
        let json: Value = serde_json::from_str(fixture).unwrap();
        let snap = map_usage_response(&json, "t", "oauth").unwrap();
        assert!(snap.primary.is_some());
    }
}
