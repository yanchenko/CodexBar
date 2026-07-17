//! AgentBar C ABI surface for native hosts (`agentbar.h` / `ab_*`).
//!
//! - Windows: WinUI links `ab_core.dll` (cdylib)
//! - macOS / Linux: staticlib link
//!
//! Scaffold exports `ab_version` only; full lifecycle lands in PR3.

pub mod ffi;

/// Workspace version shared by every platform UI (`ab_version`).
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
