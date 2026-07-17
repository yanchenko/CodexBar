//! Provider registry and strategies (Codex / Claude / Cursor MVP).
//!
//! Engine calls [`probe_enabled_providers`] on refresh. Strategies never put
//! secrets (`apiKey`, `cookieHeader`, tokens) into [`ab_model::ProviderSnapshot`].

mod claude;
mod codex;
mod common;
mod cursor;

use ab_http::HttpClient;
use ab_model::ProviderSnapshot;
use serde::Serialize;
use serde_json::Value;

pub use claude::{map_usage_response as map_claude_usage, parse_credentials_json};
pub use codex::{map_usage_response as map_codex_usage, parse_auth_json};
pub use cursor::{manual_cookie, map_usage_summary as map_cursor_usage};

/// Stable provider id wire string.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ProviderId {
    Codex,
    Claude,
    Cursor,
}

impl ProviderId {
    pub fn as_str(self) -> &'static str {
        match self {
            ProviderId::Codex => "codex",
            ProviderId::Claude => "claude",
            ProviderId::Cursor => "cursor",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "codex" => Some(ProviderId::Codex),
            "claude" => Some(ProviderId::Claude),
            "cursor" => Some(ProviderId::Cursor),
            _ => None,
        }
    }
}

/// Static MVP catalog entry.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CatalogEntry {
    pub id: &'static str,
    pub display_name: &'static str,
    pub default_enabled: bool,
}

/// MVP catalog (no secrets). Single source of truth for FFI `ab_providers_catalog_json`.
pub fn mvp_catalog() -> &'static [CatalogEntry] {
    &[
        CatalogEntry {
            id: "codex",
            display_name: "Codex",
            default_enabled: true,
        },
        CatalogEntry {
            id: "claude",
            display_name: "Claude",
            default_enabled: false,
        },
        CatalogEntry {
            id: "cursor",
            display_name: "Cursor",
            default_enabled: false,
        },
    ]
}

/// Catalog JSON for hosts (no secrets). Shared by FFI.
pub fn mvp_catalog_json() -> String {
    serde_json::to_string(mvp_catalog()).unwrap_or_else(|_| "[]".into())
}

/// Endpoint / fixture overrides (production: all `None`; tests inject httpmock bases).
#[derive(Clone, Debug, Default)]
pub struct EndpointOverrides {
    pub codex_usage_base: Option<String>,
    pub codex_token_url: Option<String>,
    /// When set, Codex CLI path maps this JSON instead of spawning `codex`.
    pub codex_rpc_fixture_json: Option<String>,
    pub claude_usage_base: Option<String>,
    pub claude_token_url: Option<String>,
    pub claude_usage_fixture_json: Option<String>,
    pub cursor_base: Option<String>,
    pub cursor_usage_fixture_json: Option<String>,
}

/// Context passed to each provider probe.
#[derive(Clone, Debug)]
pub struct ProbeContext {
    pub updated_at: String,
    pub provider_cfg: Value,
    pub http: HttpClient,
    pub endpoints: EndpointOverrides,
}

impl ProbeContext {
    pub fn new(updated_at: impl Into<String>, provider_cfg: Value) -> Self {
        Self {
            updated_at: updated_at.into(),
            provider_cfg,
            http: HttpClient::default(),
            endpoints: EndpointOverrides::default(),
        }
    }
}

/// Probe a single enabled provider by id.
pub fn probe_provider(id: ProviderId, ctx: &ProbeContext) -> ProviderSnapshot {
    match id {
        ProviderId::Codex => codex::probe(ctx),
        ProviderId::Claude => claude::probe(ctx),
        ProviderId::Cursor => cursor::probe(ctx),
    }
}

/// Walk config `providers[]` and probe each **enabled** MVP provider.
/// Unknown provider ids are skipped (not crashed). Secrets never copied into rows.
pub fn probe_enabled_providers(config: &Value, updated_at: &str) -> Vec<ProviderSnapshot> {
    probe_enabled_providers_with(config, updated_at, HttpClient::default(), EndpointOverrides::default())
}

/// Same as [`probe_enabled_providers`] with injectable HTTP + endpoints (tests).
pub fn probe_enabled_providers_with(
    config: &Value,
    updated_at: &str,
    http: HttpClient,
    endpoints: EndpointOverrides,
) -> Vec<ProviderSnapshot> {
    let mut out = Vec::new();
    let Some(arr) = config.get("providers").and_then(|v| v.as_array()) else {
        return out;
    };
    for p in arr {
        let id_str = p.get("id").and_then(|v| v.as_str()).unwrap_or("");
        let enabled = p.get("enabled").and_then(|v| v.as_bool()).unwrap_or(false);
        if !enabled {
            continue;
        }
        let Some(pid) = ProviderId::parse(id_str) else {
            // Non-MVP / unknown: skip (preserve secrets on disk; do not surface).
            continue;
        };
        let ctx = ProbeContext {
            updated_at: updated_at.to_string(),
            provider_cfg: p.clone(),
            http: http.clone(),
            endpoints: endpoints.clone(),
        };
        let mut snap = probe_provider(pid, &ctx);
        snap.enabled = true;
        // Hard ban: never leak secret-looking fields into error text beyond generic messages.
        if let Some(err) = &snap.error {
            if looks_like_secret(err) {
                snap.error = Some("provider error (redacted)".into());
            }
        }
        out.push(snap);
    }
    out
}

fn looks_like_secret(s: &str) -> bool {
    s.contains("sk-")
        || s.contains("Bearer ")
        || s.contains("cookieHeader")
        || s.contains("access_token")
        || s.contains("WorkosCursorSessionToken=")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_json_has_mvp_ids() {
        let s = mvp_catalog_json();
        assert!(s.contains("codex"));
        assert!(s.contains("claude"));
        assert!(s.contains("cursor"));
        assert!(s.contains("displayName"));
        assert!(!s.contains("apiKey"));
        // defaultEnabled: codex true, others false
        let v: Value = serde_json::from_str(&s).unwrap();
        let arr = v.as_array().unwrap();
        assert_eq!(arr[0]["id"], "codex");
        assert_eq!(arr[0]["defaultEnabled"], true);
        assert_eq!(arr[1]["defaultEnabled"], false);
        assert_eq!(arr[2]["defaultEnabled"], false);
    }

    #[test]
    fn probe_skips_disabled_and_unknown() {
        let cfg = serde_json::json!({
            "providers": [
                { "id": "codex", "enabled": false },
                { "id": "other", "enabled": true, "apiKey": "SECRET" },
                { "id": "cursor", "enabled": true }
            ]
        });
        let rows = probe_enabled_providers(&cfg, "t");
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].id, "cursor");
        assert_eq!(rows[0].error_code.as_deref(), Some("auth_missing"));
        let json = serde_json::to_string(&rows[0]).unwrap();
        assert!(!json.contains("SECRET"));
        assert!(!json.contains("apiKey"));
    }

    #[test]
    fn snapshot_never_contains_cookie_from_config() {
        let cfg = serde_json::json!({
            "providers": [{
                "id": "cursor",
                "enabled": true,
                "cookieSource": "manual",
                "cookieHeader": "WorkosCursorSessionToken=SUPER_SECRET_COOKIE_VALUE"
            }]
        });
        // Force network fail quickly — cookie must not appear in snapshot JSON.
        let http = HttpClient {
            connect_timeout: std::time::Duration::from_millis(50),
            read_timeout: std::time::Duration::from_millis(50),
            ..HttpClient::default()
        };
        let rows = probe_enabled_providers_with(
            &cfg,
            "t",
            http,
            EndpointOverrides {
                cursor_base: Some("http://127.0.0.1:1".into()),
                ..Default::default()
            },
        );
        let json = serde_json::to_string(&rows).unwrap();
        assert!(!json.contains("SUPER_SECRET_COOKIE_VALUE"));
        assert!(!json.contains("cookieHeader"));
    }
}
