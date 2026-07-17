//! Shared helpers for provider strategies.

use ab_model::{ProviderSnapshot, RateWindow};
use std::time::{SystemTime, UNIX_EPOCH};

/// Stable error codes used in snapshot rows (never secrets).
pub const ERR_AUTH_MISSING: &str = "auth_missing";
pub const ERR_AUTH_EXPIRED: &str = "auth_expired";
pub const ERR_NETWORK: &str = "network";
pub const ERR_PARSE: &str = "parse";
pub const ERR_HTTP: &str = "http_error";

/// Format unix epoch seconds as RFC3339 UTC (`…Z`).
pub fn rfc3339_from_unix(secs: i64) -> String {
    let secs = if secs < 0 { 0u64 } else { secs as u64 };
    let (year, month, day, hour, min, sec) = civil_utc_from_unix(secs);
    format!("{year:04}-{month:02}-{day:02}T{hour:02}:{min:02}:{sec:02}Z")
}

fn civil_utc_from_unix(secs: u64) -> (i32, u32, u32, u32, u32, u32) {
    let day_secs = 86_400u64;
    let days = (secs / day_secs) as i64;
    let rem = (secs % day_secs) as u32;
    let hour = rem / 3600;
    let min = (rem % 3600) / 60;
    let sec = rem % 60;
    let (y, m, d) = civil_from_days(days);
    (y, m, d, hour, min, sec)
}

fn civil_from_days(z: i64) -> (i32, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let y = (yoe as i64) + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    (y as i32, m as u32, d as u32)
}

/// Human reset description from a unix reset timestamp (relative to now).
pub fn reset_description_from_unix(reset_at: i64) -> String {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let delta = reset_at - now;
    if delta <= 0 {
        return "resets soon".into();
    }
    let mins = delta / 60;
    if mins < 60 {
        return format!("resets in {mins}m");
    }
    let hours = mins / 60;
    if hours < 48 {
        return format!("resets in {hours}h");
    }
    let days = hours / 24;
    format!("resets in {days}d")
}

/// Build a rate window from Codex/Claude-style used percent + optional reset unix + window seconds.
pub fn rate_window_from_parts(
    used_percent: f64,
    reset_at_unix: Option<i64>,
    window_seconds: Option<i64>,
) -> RateWindow {
    let mut w = RateWindow::new(used_percent);
    if let Some(secs) = window_seconds
        && secs > 0
    {
        w.window_minutes = Some(secs / 60);
    }
    if let Some(ts) = reset_at_unix {
        w.resets_at = Some(rfc3339_from_unix(ts));
        w.reset_description = Some(reset_description_from_unix(ts));
    }
    w
}

/// Clamp percent into [0, 100].
pub fn clamp_percent(v: f64) -> f64 {
    if !v.is_finite() {
        return 0.0;
    }
    v.clamp(0.0, 100.0)
}

/// Auth-missing failure row.
pub fn auth_missing(id: &str, updated_at: &str, message: &str) -> ProviderSnapshot {
    ProviderSnapshot::failed(id, updated_at, message, Some(ERR_AUTH_MISSING.into()))
}

/// Network / transport failure row.
pub fn network_error(id: &str, updated_at: &str, message: &str) -> ProviderSnapshot {
    ProviderSnapshot::failed(id, updated_at, message, Some(ERR_NETWORK.into()))
}

/// HTTP status failure.
pub fn http_error(id: &str, updated_at: &str, status: u16, body_hint: &str) -> ProviderSnapshot {
    let msg = if body_hint.is_empty() {
        format!("HTTP {status}")
    } else {
        let short: String = body_hint.chars().take(120).collect();
        format!("HTTP {status}: {short}")
    };
    ProviderSnapshot::failed(id, updated_at, msg, Some(ERR_HTTP.into()))
}

/// Parse failure.
pub fn parse_error(id: &str, updated_at: &str, message: &str) -> ProviderSnapshot {
    ProviderSnapshot::failed(id, updated_at, message, Some(ERR_PARSE.into()))
}

/// Read f64 from JSON value that may be number or string.
pub fn json_f64(v: &serde_json::Value) -> Option<f64> {
    v.as_f64()
        .or_else(|| v.as_i64().map(|i| i as f64))
        .or_else(|| v.as_u64().map(|u| u as f64))
        .or_else(|| v.as_str().and_then(|s| s.trim().parse().ok()))
}

/// Read i64 from JSON value.
pub fn json_i64(v: &serde_json::Value) -> Option<i64> {
    v.as_i64()
        .or_else(|| v.as_u64().map(|u| u as i64))
        .or_else(|| v.as_f64().map(|f| f as i64))
        .or_else(|| v.as_str().and_then(|s| s.trim().parse().ok()))
}

/// Home directory (USERPROFILE / HOME).
pub fn home_dir() -> Option<std::path::PathBuf> {
    ab_config::home_dir()
}
