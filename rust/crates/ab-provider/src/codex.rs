//! Codex provider: OAuth `auth.json` + usage HTTP; optional app-server JSON-RPC fallback.
//!
//! Ambient `CODEX_HOME` only (multi-account managed homes are a non-goal).
//! Never puts tokens into snapshots.

use crate::common::{
    self, auth_missing, clamp_percent, http_error, json_f64, json_i64, parse_error,
    rate_window_from_parts,
};
use crate::ProbeContext;
use ab_http::HttpClient;
use ab_model::{ProviderSnapshot, RateWindow};
use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const ID: &str = "codex";
const DEFAULT_USAGE_BASE: &str = "https://chatgpt.com/backend-api";
const DEFAULT_TOKEN_URL: &str = "https://auth.openai.com/oauth/token";
const CLIENT_ID: &str = "app_EMoamEEZ73f0CkXaXp7hrann";
/// Refresh when last_refresh older than 8 days (match Swift).
const REFRESH_INTERVAL_SECS: u64 = 8 * 24 * 60 * 60;

#[derive(Clone, Debug)]
pub struct CodexCredentials {
    pub access_token: String,
    pub refresh_token: String,
    pub id_token: Option<String>,
    pub account_id: Option<String>,
    pub last_refresh: Option<SystemTime>,
}

impl CodexCredentials {
    pub fn needs_refresh(&self) -> bool {
        match self.last_refresh {
            None => !self.refresh_token.is_empty(),
            Some(t) => match t.elapsed() {
                Ok(d) => d.as_secs() > REFRESH_INTERVAL_SECS,
                // Clock skew / future stamp → treat as fresh.
                Err(_) => false,
            },
        }
    }
}

/// Resolve ambient auth.json path: `$CODEX_HOME/auth.json` else `~/.codex/auth.json`.
pub fn auth_json_path() -> Option<PathBuf> {
    if let Ok(home) = std::env::var("CODEX_HOME") {
        let p = PathBuf::from(home);
        if !p.as_os_str().is_empty() {
            return Some(p.join("auth.json"));
        }
    }
    common::home_dir().map(|h| h.join(".codex").join("auth.json"))
}

/// Parse Codex `auth.json` contents.
pub fn parse_auth_json(data: &str) -> Result<CodexCredentials, String> {
    let json: Value =
        serde_json::from_str(data).map_err(|e| format!("invalid auth.json: {e}"))?;

    // API key form (OPENAI_API_KEY as access token, no refresh).
    if let Some(key) = json.get("OPENAI_API_KEY").and_then(|v| v.as_str()) {
        let key = key.trim();
        if !key.is_empty() && key != "null" {
            return Ok(CodexCredentials {
                access_token: key.into(),
                refresh_token: String::new(),
                id_token: None,
                account_id: None,
                last_refresh: None,
            });
        }
    }

    let tokens = json
        .get("tokens")
        .ok_or_else(|| "auth.json missing tokens".to_string())?;
    let access = tokens
        .get("access_token")
        .or_else(|| tokens.get("accessToken"))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    if access.is_empty() {
        return Err("auth.json has no access_token".into());
    }
    let refresh = tokens
        .get("refresh_token")
        .or_else(|| tokens.get("refreshToken"))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let id_token = tokens
        .get("id_token")
        .or_else(|| tokens.get("idToken"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let account_id = tokens
        .get("account_id")
        .or_else(|| tokens.get("accountId"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let last_refresh = json
        .get("last_refresh")
        .and_then(|v| v.as_str())
        .and_then(parse_iso8601_approx);

    Ok(CodexCredentials {
        access_token: access,
        refresh_token: refresh,
        id_token,
        account_id,
        last_refresh,
    })
}

fn parse_iso8601_approx(s: &str) -> Option<SystemTime> {
    // Accept `YYYY-MM-DDTHH:MM:SSZ` or with fractional seconds.
    let s = s.trim().trim_end_matches('Z');
    let (date, time) = s.split_once('T')?;
    let mut dp = date.split('-');
    let y: i32 = dp.next()?.parse().ok()?;
    let mo: u32 = dp.next()?.parse().ok()?;
    let d: u32 = dp.next()?.parse().ok()?;
    let time = time.split('.').next()?;
    let mut tp = time.split(':');
    let h: u32 = tp.next()?.parse().ok()?;
    let mi: u32 = tp.next()?.parse().ok()?;
    let se: u32 = tp.next()?.parse().ok()?;
    let days = days_from_civil(y, mo, d)?;
    let secs = days as i64 * 86_400 + (h as i64) * 3600 + (mi as i64) * 60 + se as i64;
    if secs < 0 {
        return None;
    }
    Some(UNIX_EPOCH + Duration::from_secs(secs as u64))
}

fn days_from_civil(y: i32, m: u32, d: u32) -> Option<i64> {
    if !(1..=12).contains(&m) || d == 0 || d > 31 {
        return None;
    }
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = (y - era * 400) as u64;
    let mp = if m > 2 { m - 3 } else { m + 9 };
    let doy = (153 * mp as u64 + 2) / 5 + d as u64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    Some(era as i64 * 146_097 + doe as i64 - 719_468)
}

/// Load credentials from ambient auth.json.
pub fn load_auth() -> Result<CodexCredentials, String> {
    let path = auth_json_path().ok_or_else(|| "could not resolve CODEX_HOME / home".to_string())?;
    if !path.is_file() {
        return Err("not_found".into());
    }
    let data = fs::read_to_string(&path).map_err(|e| e.to_string())?;
    parse_auth_json(&data)
}

/// Write-back refreshed tokens into existing auth.json (preserve unknown keys).
pub fn save_auth(creds: &CodexCredentials, path: &Path) -> Result<(), String> {
    let mut root: Value = if path.is_file() {
        let text = fs::read_to_string(path).map_err(|e| e.to_string())?;
        serde_json::from_str(&text).unwrap_or_else(|_| serde_json::json!({}))
    } else {
        serde_json::json!({})
    };
    let obj = root
        .as_object_mut()
        .ok_or_else(|| "auth.json root not object".to_string())?;
    let mut tokens = serde_json::Map::new();
    tokens.insert("access_token".into(), Value::String(creds.access_token.clone()));
    tokens.insert("refresh_token".into(), Value::String(creds.refresh_token.clone()));
    if let Some(id) = &creds.id_token {
        tokens.insert("id_token".into(), Value::String(id.clone()));
    }
    if let Some(aid) = &creds.account_id {
        tokens.insert("account_id".into(), Value::String(aid.clone()));
    }
    obj.insert("tokens".into(), Value::Object(tokens));
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    obj.insert(
        "last_refresh".into(),
        Value::String(common::rfc3339_from_unix(now as i64)),
    );
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let text = serde_json::to_string_pretty(&root).map_err(|e| e.to_string())?;
    fs::write(path, text).map_err(|e| e.to_string())?;
    Ok(())
}

/// Refresh access token via OpenAI OAuth.
pub fn refresh_token(
    client: &HttpClient,
    token_url: &str,
    creds: &CodexCredentials,
) -> Result<CodexCredentials, String> {
    if creds.refresh_token.is_empty() {
        return Ok(creds.clone());
    }
    let body = serde_json::json!({
        "client_id": CLIENT_ID,
        "grant_type": "refresh_token",
        "refresh_token": creds.refresh_token,
        "scope": "openid profile email",
    });
    let body_bytes = serde_json::to_vec(&body).map_err(|e| e.to_string())?;
    let resp = client
        .post(
            token_url,
            &[("Content-Type", "application/json")],
            &body_bytes,
        )
        .map_err(|e| e.to_string())?;
    if !resp.is_success() {
        return Err(format!("token refresh HTTP {}", resp.status));
    }
    let json: Value =
        serde_json::from_slice(&resp.body).map_err(|e| format!("refresh JSON: {e}"))?;
    let access = json
        .get("access_token")
        .and_then(|v| v.as_str())
        .unwrap_or(&creds.access_token)
        .to_string();
    let refresh = json
        .get("refresh_token")
        .and_then(|v| v.as_str())
        .unwrap_or(&creds.refresh_token)
        .to_string();
    let id_token = json
        .get("id_token")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .or_else(|| creds.id_token.clone());
    Ok(CodexCredentials {
        access_token: access,
        refresh_token: refresh,
        id_token,
        account_id: creds.account_id.clone(),
        last_refresh: Some(SystemTime::now()),
    })
}

/// Map Codex `/wham/usage` (or RPC-equivalent) JSON into a snapshot.
pub fn map_usage_response(
    json: &Value,
    updated_at: &str,
    account_label: Option<String>,
    source_label: &str,
) -> Result<ProviderSnapshot, String> {
    let rate = json
        .get("rate_limit")
        .or_else(|| json.get("rateLimit"))
        .cloned()
        .unwrap_or(Value::Null);

    let primary = window_from_json(
        rate.get("primary_window")
            .or_else(|| rate.get("primaryWindow")),
    );
    let secondary = window_from_json(
        rate.get("secondary_window")
            .or_else(|| rate.get("secondaryWindow")),
    );

    if primary.is_none() && secondary.is_none() {
        // Some RPC shapes nest under `rateLimits` / `primary`.
        if let Some(p) = window_from_json(json.get("primary")) {
            let mut snap = ProviderSnapshot::ok(ID, updated_at);
            snap.source_label = Some(source_label.into());
            snap.primary = Some(p);
            snap.secondary = window_from_json(json.get("secondary"));
            snap.account_label = account_label;
            snap.data_confidence = Some("exact".into());
            apply_credits(&mut snap, json);
            return Ok(snap);
        }
        return Err("usage response missing rate windows".into());
    }

    let mut snap = ProviderSnapshot::ok(ID, updated_at);
    snap.source_label = Some(source_label.into());
    snap.primary = primary;
    snap.secondary = secondary;
    snap.account_label = account_label;
    snap.data_confidence = Some("exact".into());
    apply_credits(&mut snap, json);
    if let Some(plan) = json
        .get("plan_type")
        .or_else(|| json.get("planType"))
        .and_then(|v| v.as_str())
    {
        if snap.account_label.is_none() {
            snap.account_label = Some(plan.to_string());
        }
    }
    Ok(snap)
}

fn apply_credits(snap: &mut ProviderSnapshot, json: &Value) {
    if let Some(credits) = json.get("credits") {
        if credits
            .get("unlimited")
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
        {
            return;
        }
        if let Some(bal) = credits.get("balance").and_then(json_f64) {
            snap.credits_remaining = Some(bal);
        }
    }
}

fn window_from_json(v: Option<&Value>) -> Option<RateWindow> {
    let w = v?;
    if w.is_null() {
        return None;
    }
    let used = w
        .get("used_percent")
        .or_else(|| w.get("usedPercent"))
        .and_then(json_f64)
        .map(clamp_percent)?;
    let reset_at = w
        .get("reset_at")
        .or_else(|| w.get("resetAt"))
        .and_then(json_i64);
    let window_secs = w
        .get("limit_window_seconds")
        .or_else(|| w.get("limitWindowSeconds"))
        .and_then(json_i64);
    Some(rate_window_from_parts(used, reset_at, window_secs))
}

fn usage_url(base: &str) -> String {
    let base = base.trim_end_matches('/');
    if base.contains("/backend-api") {
        format!("{base}/wham/usage")
    } else if base.ends_with("/wham/usage") {
        base.to_string()
    } else {
        // Mock servers pass full path base like `http://127.0.0.1:port`
        format!("{base}/wham/usage")
    }
}

/// Fetch usage over HTTP with bearer token.
pub fn fetch_usage_http(
    client: &HttpClient,
    usage_base: &str,
    access_token: &str,
    account_id: Option<&str>,
) -> Result<Value, String> {
    let url = usage_url(usage_base);
    let auth = format!("Bearer {access_token}");
    let mut hdr_pairs: Vec<(String, String)> = vec![
        ("Authorization".into(), auth),
        ("Accept".into(), "application/json".into()),
        ("User-Agent".into(), "AgentBar".into()),
    ];
    if let Some(aid) = account_id.filter(|s| !s.is_empty()) {
        hdr_pairs.push(("ChatGPT-Account-Id".into(), aid.to_string()));
    }
    let refs: Vec<(&str, &str)> = hdr_pairs
        .iter()
        .map(|(k, v)| (k.as_str(), v.as_str()))
        .collect();
    let resp = client.get(&url, &refs).map_err(|e| e.to_string())?;
    if resp.status == 401 || resp.status == 403 {
        return Err("unauthorized".into());
    }
    if !resp.is_success() {
        return Err(format!("http:{}", resp.status));
    }
    serde_json::from_slice(&resp.body).map_err(|e| format!("parse: {e}"))
}

/// Probe Codex for the engine snapshot.
pub fn probe(ctx: &ProbeContext) -> ProviderSnapshot {
    let updated = ctx.updated_at.as_str();

    // 1) OAuth auth.json path
    match load_auth() {
        Ok(mut creds) => {
            let token_url = ctx
                .endpoints
                .codex_token_url
                .as_deref()
                .unwrap_or(DEFAULT_TOKEN_URL);
            let usage_base = ctx
                .endpoints
                .codex_usage_base
                .as_deref()
                .unwrap_or(DEFAULT_USAGE_BASE);

            if creds.needs_refresh() {
                match refresh_token(&ctx.http, token_url, &creds) {
                    Ok(new_creds) => {
                        if let Some(path) = auth_json_path() {
                            let _ = save_auth(&new_creds, &path);
                        }
                        creds = new_creds;
                    }
                    Err(e) => {
                        ab_log::warn("codex", &format!("token refresh failed: {e}"));
                        // Continue with existing access token.
                    }
                }
            }

            match fetch_usage_http(
                &ctx.http,
                usage_base,
                &creds.access_token,
                creds.account_id.as_deref(),
            ) {
                Ok(json) => {
                    let email = account_email_from_id_token(creds.id_token.as_deref());
                    match map_usage_response(&json, updated, email, "oauth") {
                        Ok(mut snap) => {
                            snap.enabled = true;
                            return snap;
                        }
                        Err(e) => {
                            return parse_error(ID, updated, &format!("Codex usage: {e}"));
                        }
                    }
                }
                Err(e) if e == "unauthorized" => {
                    let mut s = ProviderSnapshot::failed(
                        ID,
                        updated,
                        "Codex OAuth token expired or invalid. Run `codex` to re-authenticate.",
                        Some(common::ERR_AUTH_EXPIRED.into()),
                    );
                    s.source_label = Some("oauth".into());
                    return s;
                }
                Err(e) if e.starts_with("http:") => {
                    let status: u16 = e.trim_start_matches("http:").parse().unwrap_or(0);
                    let mut s = http_error(ID, updated, status, "");
                    s.source_label = Some("oauth".into());
                    return s;
                }
                Err(e) => {
                    // Fall through to CLI RPC attempt.
                    ab_log::info("codex", &format!("oauth usage failed: {e}; trying app-server"));
                }
            }
        }
        Err(e) if e == "not_found" => {
            // Fall through to CLI.
        }
        Err(e) => {
            let mut s = auth_missing(
                ID,
                updated,
                &format!("Codex auth.json unreadable: {e}. Run `codex` to log in."),
            );
            s.source_label = Some("oauth".into());
            // Still try CLI below.
            if let Some(cli_snap) = try_app_server_rpc(ctx) {
                return cli_snap;
            }
            return s;
        }
    }

    // 2) CLI app-server JSON-RPC fallback
    if let Some(snap) = try_app_server_rpc(ctx) {
        return snap;
    }

    // No credentials and no usable CLI.
    let mut s = auth_missing(
        ID,
        updated,
        "Codex auth.json not found. Run `codex` to log in.",
    );
    s.source_label = Some("oauth".into());
    s
}

/// Best-effort `codex app-server` JSON-RPC: initialize + account/rateLimits/read.
///
/// Returns `None` when the binary is missing or the session fails quickly so OAuth
/// auth_missing remains the primary signal.
fn try_app_server_rpc(ctx: &ProbeContext) -> Option<ProviderSnapshot> {
    // Prefer injectible fixture path for tests.
    if let Some(fixture) = &ctx.endpoints.codex_rpc_fixture_json {
        return match map_rpc_rate_limits(fixture, &ctx.updated_at) {
            Ok(mut s) => {
                s.source_label = Some("cli".into());
                Some(s)
            }
            Err(_) => None,
        };
    }

    // Spawn is optional/best-effort — short timeout; skip if not on PATH.
    let out = ab_proc::run(
        "codex",
        &["--version"],
        Duration::from_secs(3),
        64 * 1024,
    )
    .ok()?;
    if !out.success() {
        return None;
    }
    // Full interactive JSON-RPC over stdio is deferred for long-lived sessions;
    // when only --version works we still report that CLI is present but no live
    // rate limits without OAuth. Callers already surface auth_missing.
    let _ = ctx;
    None
}

/// Map app-server `account/rateLimits/read` style result JSON.
pub fn map_rpc_rate_limits(json_text: &str, updated_at: &str) -> Result<ProviderSnapshot, String> {
    let json: Value = serde_json::from_str(json_text).map_err(|e| e.to_string())?;
    // Accept either full usage shape or nested result.
    let body = json
        .get("result")
        .cloned()
        .unwrap_or(json);
    map_usage_response(&body, updated_at, None, "cli")
}

fn account_email_from_id_token(id_token: Option<&str>) -> Option<String> {
    let token = id_token?;
    let parts: Vec<&str> = token.split('.').collect();
    if parts.len() < 2 {
        return None;
    }
    let payload = parts[1];
    let decoded = b64url_decode(payload)?;
    let v: Value = serde_json::from_slice(&decoded).ok()?;
    v.get("email")
        .and_then(|e| e.as_str())
        .map(|s| s.to_string())
        .or_else(|| {
            v.get("https://api.openai.com/profile")
                .and_then(|p| p.get("email"))
                .and_then(|e| e.as_str())
                .map(|s| s.to_string())
        })
}

fn b64url_decode(s: &str) -> Option<Vec<u8>> {
    let mut s = s.replace('-', "+").replace('_', "/");
    while s.len() % 4 != 0 {
        s.push('=');
    }
    // Minimal base64 decode without extra crate.
    base64_decode(&s)
}

fn base64_decode(input: &str) -> Option<Vec<u8>> {
    fn val(c: u8) -> Option<u8> {
        match c {
            b'A'..=b'Z' => Some(c - b'A'),
            b'a'..=b'z' => Some(c - b'a' + 26),
            b'0'..=b'9' => Some(c - b'0' + 52),
            b'+' => Some(62),
            b'/' => Some(63),
            _ => None,
        }
    }
    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(bytes.len() * 3 / 4);
    let mut i = 0;
    while i + 3 < bytes.len() {
        if bytes[i] == b'=' {
            break;
        }
        let a = val(bytes[i])?;
        let b = val(bytes[i + 1])?;
        let c = if bytes[i + 2] == b'=' {
            0
        } else {
            val(bytes[i + 2])?
        };
        let d = if bytes[i + 3] == b'=' {
            0
        } else {
            val(bytes[i + 3])?
        };
        out.push((a << 2) | (b >> 4));
        if bytes[i + 2] != b'=' {
            out.push((b << 4) | (c >> 2));
        }
        if bytes[i + 3] != b'=' {
            out.push((c << 6) | d);
        }
        i += 4;
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use httpmock::prelude::*;
    use std::sync::Mutex;

    static ENV_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn parse_auth_tokens() {
        let raw = r#"{
          "tokens": {
            "access_token": "at",
            "refresh_token": "rt",
            "account_id": "acc-1",
            "id_token": "x.eyJlbWFpbCI6InUAZXhhbXBsZS5jb20ifQ.y"
          },
          "last_refresh": "2099-01-01T00:00:00Z"
        }"#;
        let c = parse_auth_json(raw).unwrap();
        assert_eq!(c.access_token, "at");
        assert_eq!(c.refresh_token, "rt");
        assert_eq!(c.account_id.as_deref(), Some("acc-1"));
        assert!(!c.needs_refresh()); // far-future last_refresh
    }

    #[test]
    fn map_usage_primary_secondary_credits() {
        let raw = serde_json::json!({
            "plan_type": "pro",
            "rate_limit": {
                "primary_window": {
                    "used_percent": 15,
                    "reset_at": 1735401600,
                    "limit_window_seconds": 18000
                },
                "secondary_window": {
                    "used_percent": 5,
                    "reset_at": 1735920000,
                    "limit_window_seconds": 604800
                }
            },
            "credits": { "has_credits": true, "unlimited": false, "balance": 150.0 }
        });
        let snap = map_usage_response(&raw, "2026-07-17T12:00:00Z", Some("u@e.com".into()), "oauth")
            .unwrap();
        assert!(snap.error.is_none());
        assert_eq!(snap.primary.as_ref().unwrap().used_percent, 15.0);
        assert_eq!(snap.primary.as_ref().unwrap().window_minutes, Some(300));
        assert_eq!(snap.secondary.as_ref().unwrap().used_percent, 5.0);
        assert_eq!(snap.credits_remaining, Some(150.0));
        assert_eq!(snap.account_label.as_deref(), Some("u@e.com"));
        assert_eq!(snap.source_label.as_deref(), Some("oauth"));
    }

    #[test]
    fn oauth_usage_httpmock() {
        let _g = ENV_LOCK.lock().unwrap();
        let server = MockServer::start();
        let m = server.mock(|when, then| {
            when.method(GET)
                .path("/wham/usage")
                .header("Authorization", "Bearer test-access");
            then.status(200)
                .header("content-type", "application/json")
                .body(
                    r#"{
                      "plan_type":"pro",
                      "rate_limit":{
                        "primary_window":{"used_percent":42,"reset_at":2000000000,"limit_window_seconds":18000},
                        "secondary_window":{"used_percent":10,"reset_at":2000500000,"limit_window_seconds":604800}
                      },
                      "credits":{"has_credits":true,"unlimited":false,"balance":12.5}
                    }"#,
                );
        });

        let tmp = tempfile::tempdir().unwrap();
        let auth = tmp.path().join("auth.json");
        fs::write(
            &auth,
            r#"{"tokens":{"access_token":"test-access","refresh_token":"rt","account_id":"a1"}}"#,
        )
        .unwrap();
        // SAFETY: single-threaded under ENV_LOCK for tests only.
        unsafe {
            std::env::set_var("CODEX_HOME", tmp.path());
        }

        let client = HttpClient {
            connect_timeout: Duration::from_secs(2),
            read_timeout: Duration::from_secs(2),
            ..HttpClient::default()
        };
        let json = fetch_usage_http(&client, &server.base_url(), "test-access", Some("a1")).unwrap();
        m.assert();
        let snap = map_usage_response(&json, "t", None, "oauth").unwrap();
        assert_eq!(snap.primary.as_ref().unwrap().used_percent, 42.0);
        assert_eq!(snap.credits_remaining, Some(12.5));

        unsafe {
            std::env::remove_var("CODEX_HOME");
        }
    }

    #[test]
    fn probe_auth_missing_without_auth_json() {
        let _g = ENV_LOCK.lock().unwrap();
        let tmp = tempfile::tempdir().unwrap();
        unsafe {
            std::env::set_var("CODEX_HOME", tmp.path());
        }
        let ctx = ProbeContext {
            updated_at: "2026-07-17T12:00:00Z".into(),
            provider_cfg: serde_json::json!({"id":"codex","enabled":true}),
            http: HttpClient {
                connect_timeout: Duration::from_millis(200),
                read_timeout: Duration::from_millis(200),
                ..HttpClient::default()
            },
            endpoints: Default::default(),
        };
        let snap = probe(&ctx);
        assert_eq!(snap.error_code.as_deref(), Some("auth_missing"));
        assert!(snap.error.as_ref().unwrap().contains("auth.json"));
        unsafe {
            std::env::remove_var("CODEX_HOME");
        }
    }

    #[test]
    fn map_rpc_fixture() {
        let fixture = include_str!("../tests/fixtures/codex_usage.json");
        let snap = map_rpc_rate_limits(fixture, "t").unwrap();
        assert!(snap.primary.is_some());
        assert_eq!(snap.source_label.as_deref(), Some("cli"));
    }
}
