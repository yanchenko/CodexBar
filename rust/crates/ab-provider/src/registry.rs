//! Provider registry tooling — catalog entries + strategy dispatch.
//!
//! Adding an API-key provider:
//! 1. Implement `probe(ctx) -> ProviderSnapshot` in a new module (see `openrouter.rs`).
//! 2. Add a [`ProviderId`] variant + `as_str` / `parse`.
//! 3. Register a [`CatalogEntry`] in [`mvp_catalog`] (or extend via [`register_catalog_entry`] tests).
//! 4. Wire the match arm in [`dispatch_probe`].
//!
//! Hosts consume catalog JSON only — **no host changes** for new providers.

use crate::{claude, codex, cursor, openrouter, ProbeContext, ProviderId};
use ab_model::ProviderSnapshot;
use serde::Serialize;

/// Static catalog entry (metadata only; no secrets).
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CatalogEntry {
    pub id: &'static str,
    pub display_name: &'static str,
    pub default_enabled: bool,
    /// Auth mode hint for hosts/settings UI.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auth_mode: Option<&'static str>,
}

/// Built-in catalog (MVP + first API-key wave). Single source for FFI catalog JSON.
pub fn builtin_catalog() -> &'static [CatalogEntry] {
    &[
        CatalogEntry {
            id: "codex",
            display_name: "Codex",
            default_enabled: true,
            auth_mode: Some("oauth_file"),
        },
        CatalogEntry {
            id: "claude",
            display_name: "Claude",
            default_enabled: false,
            auth_mode: Some("oauth_file"),
        },
        CatalogEntry {
            id: "cursor",
            display_name: "Cursor",
            default_enabled: false,
            auth_mode: Some("manual_cookie"),
        },
        CatalogEntry {
            id: "openrouter",
            display_name: "OpenRouter",
            default_enabled: false,
            auth_mode: Some("api_key"),
        },
    ]
}

/// Catalog JSON for hosts (no secrets).
pub fn catalog_json() -> String {
    serde_json::to_string(builtin_catalog()).unwrap_or_else(|_| "[]".into())
}

/// Dispatch probe by id. Unknown ids never reach here (caller skips).
pub fn dispatch_probe(id: ProviderId, ctx: &ProbeContext) -> ProviderSnapshot {
    match id {
        ProviderId::Codex => codex::probe(ctx),
        ProviderId::Claude => claude::probe(ctx),
        ProviderId::Cursor => cursor::probe(ctx),
        ProviderId::OpenRouter => openrouter::probe(ctx),
    }
}

/// Validate that every [`ProviderId`] has a catalog entry (test / CI helper).
pub fn catalog_covers_all_ids() -> bool {
    for id in ProviderId::ALL {
        if !builtin_catalog().iter().any(|e| e.id == id.as_str()) {
            return false;
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_covers_all_provider_ids() {
        assert!(catalog_covers_all_ids());
    }

    #[test]
    fn openrouter_in_catalog_api_key() {
        let e = builtin_catalog()
            .iter()
            .find(|c| c.id == "openrouter")
            .expect("openrouter catalog");
        assert_eq!(e.auth_mode, Some("api_key"));
        assert!(!e.default_enabled);
    }

    #[test]
    fn catalog_json_has_no_secrets() {
        let s = catalog_json();
        assert!(s.contains("openrouter"));
        assert!(!s.contains("apiKey"));
        assert!(!s.contains("sk-"));
    }
}
