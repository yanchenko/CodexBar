//! Provider registry and strategies.
//!
//! MVP providers (Codex / Claude / Cursor) land in later PRs. This crate holds
//! shared types and a stub catalog for the engine.

#![allow(dead_code)]

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
#[derive(Clone, Debug)]
pub struct CatalogEntry {
    pub id: &'static str,
    pub display_name: &'static str,
    pub default_enabled: bool,
}

/// MVP catalog (no secrets).
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
