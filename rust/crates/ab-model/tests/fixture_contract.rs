//! Contract tests: golden JSON fixtures round-trip through ab-model types.
//!
//! Fixtures are shared with WinUI DTO tests (`apps/windows/winui.tests`).

use ab_model::UsageSnapshot;
use serde_json::Value;
use std::path::PathBuf;

fn fixture_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

fn load(name: &str) -> String {
    let path = fixture_dir().join(name);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()))
}

#[test]
fn success_codex_omits_error_and_round_trips() {
    let raw = load("success_codex.json");
    assert!(
        !raw.contains("\"error\""),
        "success fixture must omit error key: {raw}"
    );
    assert!(!raw.contains("\"error\":null"));

    let v: Value = serde_json::from_str(&raw).unwrap();
    let codex = &v["providers"][0];
    assert!(codex.get("error").is_none());
    assert!(codex.get("errorCode").is_none());
    assert_eq!(codex["primary"]["usedPercent"], 42.5);
    assert_eq!(codex["primary"]["windowMinutes"], 300);
    assert_eq!(codex["creditsRemaining"], 12.5);

    let snap: UsageSnapshot = serde_json::from_str(&raw).unwrap();
    assert_eq!(snap.schema_version, 1);
    assert_eq!(snap.seq, 42);
    assert_eq!(snap.providers.len(), 1);
    assert!(snap.providers[0].error.is_none());
    assert_eq!(snap.providers[0].primary.as_ref().unwrap().used_percent, 42.5);

    let back = snap.to_json_string().unwrap();
    assert!(!back.contains("\"error\":null"));
    assert!(!back.contains("\"error\""));
}

#[test]
fn failure_cursor_includes_string_error() {
    let raw = load("failure_cursor_auth.json");
    let v: Value = serde_json::from_str(&raw).unwrap();
    let p = &v["providers"][0];
    assert_eq!(
        p["error"],
        "Cursor cookie not configured (cookieSource=manual required)"
    );
    assert_eq!(p["errorCode"], "auth_missing");
    assert!(!p["error"].is_null());
    assert!(p.get("primary").is_none());

    let snap: UsageSnapshot = serde_json::from_str(&raw).unwrap();
    assert_eq!(snap.providers[0].error.as_deref(), Some("Cursor cookie not configured (cookieSource=manual required)"));
    assert_eq!(snap.providers[0].error_code.as_deref(), Some("auth_missing"));
}

#[test]
fn rate_window_full_fields() {
    let raw = load("rate_window_full.json");
    let v: Value = serde_json::from_str(&raw).unwrap();
    let primary = &v["providers"][0]["primary"];
    assert_eq!(primary["usedPercent"], 88.0);
    assert_eq!(primary["windowMinutes"], 300);
    assert_eq!(primary["resetsAt"], "2026-07-17T14:00:00Z");
    assert_eq!(primary["resetDescription"], "resets in 2h");
    assert_eq!(primary["nextRegenPercent"], 1.5);
    assert_eq!(primary["isSyntheticPlaceholder"], true);

    // secondary omits false synthetic
    let secondary = &v["providers"][0]["secondary"];
    assert!(secondary.get("isSyntheticPlaceholder").is_none());

    let extras = v["providers"][0]["extraRateWindows"].as_array().unwrap();
    assert_eq!(extras.len(), 1);
    assert_eq!(extras[0]["id"], "extra_session");
    assert_eq!(extras[0]["window"]["usedPercent"], 12.0);

    let snap: UsageSnapshot = serde_json::from_str(&raw).unwrap();
    let p = &snap.providers[0];
    assert!(p.primary.as_ref().unwrap().is_synthetic_placeholder);
    assert_eq!(p.extra_rate_windows.as_ref().unwrap().len(), 1);
}

#[test]
fn cursor_requests_fixture() {
    let raw = load("cursor_requests.json");
    let v: Value = serde_json::from_str(&raw).unwrap();
    let cr = &v["providers"][0]["cursorRequests"];
    assert_eq!(cr["included"], 500.0);
    assert_eq!(cr["used"], 42.0);
    assert_eq!(cr["remaining"], 458.0);

    let snap: UsageSnapshot = serde_json::from_str(&raw).unwrap();
    let cr = snap.providers[0].cursor_requests.as_ref().unwrap();
    assert_eq!(cr.included, Some(500.0));
    assert_eq!(cr.used, Some(42.0));
    assert_eq!(cr.remaining, Some(458.0));
    // success path: no error key
    assert!(snap.providers[0].error.is_none());
    let back = snap.to_json_string().unwrap();
    assert!(!back.contains("\"error\""));
}
