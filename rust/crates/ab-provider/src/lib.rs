//! Provider registry and strategies.
//!
//! MVP providers (Codex / Claude / Cursor) land in later PRs. This crate holds
//! shared types and a stub catalog for the engine.

#![allow(dead_code)]

use serde::Serialize;

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
    }
}
