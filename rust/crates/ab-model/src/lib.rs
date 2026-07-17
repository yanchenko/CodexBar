//! AgentBar usage snapshot wire types (schema v1).
//!
//! Field naming is camelCase. Optional fields are **omitted** when absent —
//! never serialized as JSON `null`. `error` / `errorCode` are omitted when OK.

use serde::{Deserialize, Serialize};

/// Snapshot schema version shipped by the engine.
pub const SCHEMA_VERSION: u32 = 1;

/// Root usage snapshot (engine → host JSON).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageSnapshot {
    pub schema_version: u32,
    pub seq: u64,
    pub updated_at: String,
    pub refreshing: bool,
    pub providers: Vec<ProviderSnapshot>,
}

impl UsageSnapshot {
    /// Empty non-refreshing snapshot at `seq` with current schema version.
    pub fn empty(seq: u64, updated_at: impl Into<String>) -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            seq,
            updated_at: updated_at.into(),
            refreshing: false,
            providers: Vec::new(),
        }
    }

    /// Serialize with omit-null policy (serde skip_serializing_if on fields).
    pub fn to_json_string(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string(self)
    }

    /// Pretty-print JSON.
    pub fn to_json_string_pretty(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string_pretty(self)
    }
}

/// Per-provider usage row.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderSnapshot {
    pub id: String,
    pub enabled: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_label: Option<String>,
    pub updated_at: String,
    /// Human-readable error — **omit when OK** (never JSON null).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// Stable error code when known — omit when OK.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_code: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub primary: Option<RateWindow>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub secondary: Option<RateWindow>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tertiary: Option<RateWindow>,
    /// Omit when `None` or empty (design: never emit `"extraRateWindows":[]`).
    #[serde(default, skip_serializing_if = "extra_rate_windows_omitted")]
    pub extra_rate_windows: Option<Vec<NamedRateWindow>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub credits_remaining: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account_label: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data_confidence: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cursor_requests: Option<CursorRequests>,
}

impl ProviderSnapshot {
    /// Minimal OK provider row (no windows, no error).
    pub fn ok(id: impl Into<String>, updated_at: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            enabled: true,
            source_label: None,
            updated_at: updated_at.into(),
            error: None,
            error_code: None,
            primary: None,
            secondary: None,
            tertiary: None,
            extra_rate_windows: None,
            credits_remaining: None,
            account_label: None,
            data_confidence: None,
            cursor_requests: None,
        }
    }

    /// Failed provider row with required non-empty error string.
    pub fn failed(
        id: impl Into<String>,
        updated_at: impl Into<String>,
        error: impl Into<String>,
        error_code: Option<String>,
    ) -> Self {
        Self {
            id: id.into(),
            enabled: true,
            source_label: None,
            updated_at: updated_at.into(),
            error: Some(error.into()),
            error_code,
            primary: None,
            secondary: None,
            tertiary: None,
            extra_rate_windows: None,
            credits_remaining: None,
            account_label: None,
            data_confidence: None,
            cursor_requests: None,
        }
    }
}

/// Rate / usage window (parity with Swift `RateWindow`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RateWindow {
    pub used_percent: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub window_minutes: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resets_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reset_description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_regen_percent: Option<f64>,
    /// Omit when false (default).
    #[serde(default, skip_serializing_if = "is_false")]
    pub is_synthetic_placeholder: bool,
}

impl RateWindow {
    pub fn new(used_percent: f64) -> Self {
        Self {
            used_percent: finite_or_zero(used_percent),
            window_minutes: None,
            resets_at: None,
            reset_description: None,
            next_regen_percent: None,
            is_synthetic_placeholder: false,
        }
    }
}

/// Named extra rate window.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NamedRateWindow {
    pub id: String,
    pub title: String,
    pub window: RateWindow,
    /// Default true; omit when true.
    #[serde(default = "default_true", skip_serializing_if = "is_true")]
    pub usage_known: bool,
}

/// Cursor-only thin request units DTO.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CursorRequests {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub included: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub used: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remaining: Option<f64>,
}

fn is_false(b: &bool) -> bool {
    !*b
}

fn is_true(b: &bool) -> bool {
    *b
}

fn default_true() -> bool {
    true
}

fn extra_rate_windows_omitted(v: &Option<Vec<NamedRateWindow>>) -> bool {
    match v {
        None => true,
        Some(items) => items.is_empty(),
    }
}

/// NaN/Inf → 0.0 (schema: finite f64 only).
pub fn finite_or_zero(v: f64) -> f64 {
    if v.is_finite() { v } else { 0.0 }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;

    #[test]
    fn ok_provider_omits_error_fields() {
        let p = ProviderSnapshot::ok("codex", "2026-07-17T12:00:00Z");
        let v: Value = serde_json::to_value(&p).unwrap();
        assert!(v.get("error").is_none(), "error must be omitted when OK");
        assert!(v.get("errorCode").is_none());
        assert_eq!(v["id"], "codex");
        assert_eq!(v["enabled"], true);
    }

    #[test]
    fn failed_provider_emits_error_string() {
        let p = ProviderSnapshot::failed(
            "cursor",
            "2026-07-17T12:00:00Z",
            "Cursor cookie not configured",
            Some("auth_missing".into()),
        );
        let v: Value = serde_json::to_value(&p).unwrap();
        assert_eq!(v["error"], "Cursor cookie not configured");
        assert_eq!(v["errorCode"], "auth_missing");
        // Never null
        assert!(!v["error"].is_null());
    }

    #[test]
    fn rate_window_omits_false_synthetic() {
        let w = RateWindow::new(42.5);
        let v: Value = serde_json::to_value(&w).unwrap();
        assert_eq!(v["usedPercent"], 42.5);
        assert!(v.get("isSyntheticPlaceholder").is_none());
    }

    #[test]
    fn rate_window_emits_synthetic_when_true() {
        let mut w = RateWindow::new(0.0);
        w.is_synthetic_placeholder = true;
        let v: Value = serde_json::to_value(&w).unwrap();
        assert_eq!(v["isSyntheticPlaceholder"], true);
    }

    #[test]
    fn finite_or_zero_handles_nan_inf() {
        assert_eq!(finite_or_zero(f64::NAN), 0.0);
        assert_eq!(finite_or_zero(f64::INFINITY), 0.0);
        assert_eq!(finite_or_zero(1.5), 1.5);
    }

    #[test]
    fn snapshot_round_trip_design_example() {
        let snap = UsageSnapshot {
            schema_version: 1,
            seq: 42,
            updated_at: "2026-07-17T12:00:00Z".into(),
            refreshing: false,
            providers: vec![
                {
                    let mut p = ProviderSnapshot::ok("codex", "2026-07-17T12:00:00Z");
                    p.source_label = Some("oauth".into());
                    p.primary = Some({
                        let mut w = RateWindow::new(42.5);
                        w.window_minutes = Some(300);
                        w.resets_at = Some("2026-07-17T17:00:00Z".into());
                        w
                    });
                    p.secondary = Some({
                        let mut w = RateWindow::new(10.0);
                        w.window_minutes = Some(10080);
                        w
                    });
                    p.credits_remaining = Some(12.5);
                    p.account_label = Some("user@example.com".into());
                    p.data_confidence = Some("exact".into());
                    p
                },
                {
                    let mut p = ProviderSnapshot::failed(
                        "cursor",
                        "2026-07-17T12:00:00Z",
                        "Cursor cookie not configured (cookieSource=manual required)",
                        Some("auth_missing".into()),
                    );
                    p.source_label = Some("web".into());
                    p
                },
            ],
        };
        let json = snap.to_json_string().unwrap();
        assert!(!json.contains("\"error\":null"));
        assert!(json.contains("\"error\":\"Cursor cookie"));
        // codex OK provider must not have error key near codex block — parse
        let v: Value = serde_json::from_str(&json).unwrap();
        let codex = &v["providers"][0];
        assert!(codex.get("error").is_none());
        assert_eq!(codex["cursorRequests"], Value::Null); // missing → null on Value get, but key absent:
        assert!(codex.get("cursorRequests").is_none());
        let back: UsageSnapshot = serde_json::from_str(&json).unwrap();
        assert_eq!(back.seq, 42);
        assert_eq!(back.providers.len(), 2);
    }

    #[test]
    fn cursor_requests_omit_when_empty_object_fields() {
        let mut p = ProviderSnapshot::ok("cursor", "2026-07-17T12:00:00Z");
        p.cursor_requests = Some(CursorRequests {
            included: Some(500.0),
            used: Some(42.0),
            remaining: None,
        });
        let v: Value = serde_json::to_value(&p).unwrap();
        assert_eq!(v["cursorRequests"]["included"], 500.0);
        assert!(v["cursorRequests"].get("remaining").is_none());
    }

    #[test]
    fn empty_extra_rate_windows_omitted() {
        let mut p = ProviderSnapshot::ok("codex", "2026-07-17T12:00:00Z");
        p.extra_rate_windows = Some(vec![]);
        let v: Value = serde_json::to_value(&p).unwrap();
        assert!(
            v.get("extraRateWindows").is_none(),
            "empty extraRateWindows must be omitted: {v}"
        );
    }
}
