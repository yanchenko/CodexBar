//! Dual CLI parity notes stub (PR16) — documents non-blocking compare intent.
//!
//! Real dual CLI compare runs on macOS when both Swift and Rust CLIs are present.
//! This test only pins schema invariants that both CLIs must honor.

use ab_model::UsageSnapshot;

#[test]
fn dual_cli_schema_invariants_from_fixture() {
    let raw = include_str!("fixtures/success_codex.json");
    let snap: UsageSnapshot = serde_json::from_str(raw).expect("fixture");
    assert_eq!(snap.schema_version, ab_model::SCHEMA_VERSION);
    let json = snap.to_json_string().unwrap();
    // Both CLIs must never emit secret field names in usage JSON.
    for banned in ["apiKey", "cookieHeader", "access_token", "refresh_token"] {
        assert!(
            !json.contains(banned),
            "banned key {banned} in dual-CLI contract JSON"
        );
    }
}

#[test]
fn dual_cli_parity_doc_exists() {
    // Path relative to crate: ../../../docs/dual-cli-parity.md
    let doc = include_str!("../../../../docs/dual-cli-parity.md");
    assert!(doc.contains("schemaVersion"));
    assert!(doc.contains("Non-blocking"));
}
