//! Shared helpers for provider strategies.

use ab_model::{ProviderSnapshot, RateWindow};
use serde_json::Value;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

/// Process-wide lock for OAuth credential read-modify-write (Codex auth.json, Claude credentials).
/// Best-effort against concurrent engine + CLI refresh clobber; not a cross-process flock.
static AUTH_IO: Mutex<()> = Mutex::new(());

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
pub fn home_dir() -> Option<PathBuf> {
    ab_config::home_dir()
}

/// Run `f` while holding the process-wide auth I/O lock.
pub fn with_auth_lock<R>(f: impl FnOnce() -> R) -> R {
    let _g = AUTH_IO.lock().unwrap_or_else(|e| e.into_inner());
    f()
}

/// Atomic JSON write: temp + fsync + rename (with `.bak` safety like ab-config).
///
/// Holds [`AUTH_IO`]. Sets `0600` on Unix. Concurrent GUI+CLI still race across
/// processes; within one process, refreshes are serialized.
#[allow(dead_code)] // public helper for providers that write without a prior read merge
pub fn write_json_atomic(path: &Path, value: &Value) -> Result<(), String> {
    with_auth_lock(|| write_json_atomic_held(path, value))
}

/// Atomic write while caller already holds the auth lock via [`with_auth_lock`].
pub fn write_json_atomic_held(path: &Path, value: &Value) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let text = serde_json::to_string_pretty(value).map_err(|e| e.to_string())?;
    let dir = path.parent().unwrap_or_else(|| Path::new("."));
    let file_name = path
        .file_name()
        .map(|s| s.to_os_string())
        .unwrap_or_else(|| "auth.json".into());

    let mut tmp_name = file_name.clone();
    tmp_name.push(".tmp");
    let tmp_path = dir.join(&tmp_name);

    let mut bak_name = file_name.clone();
    bak_name.push(".bak");
    let bak_path = dir.join(&bak_name);

    {
        let mut f = fs::File::create(&tmp_path).map_err(|e| e.to_string())?;
        f.write_all(text.as_bytes()).map_err(|e| e.to_string())?;
        f.write_all(b"\n").map_err(|e| e.to_string())?;
        f.sync_all().map_err(|e| e.to_string())?;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(&tmp_path, fs::Permissions::from_mode(0o600));
    }

    if path.exists() {
        if bak_path.exists() {
            let _ = fs::remove_file(&bak_path);
        }
        fs::rename(path, &bak_path).map_err(|e| e.to_string())?;
    }

    if let Err(e) = fs::rename(&tmp_path, path) {
        if bak_path.exists() && !path.exists() {
            let _ = fs::rename(&bak_path, path);
        }
        let _ = fs::remove_file(&tmp_path);
        return Err(e.to_string());
    }
    let _ = fs::remove_file(&bak_path);

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(path, fs::Permissions::from_mode(0o600));
    }
    Ok(())
}

/// Resolve a CLI program to an absolute path when possible.
///
/// Prefers known install locations, then PATH entries that contain an executable
/// of `name` (with Windows `.exe` / `.cmd` suffixes). Falls back to bare `name`
/// so argv-only spawn still works if nothing is resolvable.
pub fn resolve_cli_program(name: &str) -> PathBuf {
    resolve_cli_program_opt(name).unwrap_or_else(|| PathBuf::from(name))
}

fn resolve_cli_program_opt(name: &str) -> Option<PathBuf> {
    let home = home_dir();
    let mut candidates: Vec<PathBuf> = Vec::new();

    if let Some(ref h) = home {
        candidates.push(h.join(".local").join("bin").join(name));
        candidates.push(h.join(".cargo").join("bin").join(name));
        candidates.push(h.join("bin").join(name));
        // npm / nvm common layouts
        candidates.push(h.join(".npm-global").join("bin").join(name));
        candidates.push(h.join("AppData").join("Roaming").join("npm").join(name));
        #[cfg(windows)]
        {
            candidates.push(h.join("AppData").join("Roaming").join("npm").join(format!("{name}.cmd")));
            candidates.push(h.join("AppData").join("Roaming").join("npm").join(format!("{name}.exe")));
            candidates.push(h.join(".local").join("bin").join(format!("{name}.exe")));
            candidates.push(h.join(".cargo").join("bin").join(format!("{name}.exe")));
        }
    }

    #[cfg(unix)]
    {
        candidates.push(PathBuf::from(format!("/opt/homebrew/bin/{name}")));
        candidates.push(PathBuf::from(format!("/usr/local/bin/{name}")));
        candidates.push(PathBuf::from(format!("/usr/bin/{name}")));
        candidates.push(PathBuf::from(format!("/home/linuxbrew/.linuxbrew/bin/{name}")));
    }

    for c in &candidates {
        if is_executable(c) {
            return Some(c.clone());
        }
    }

    // PATH search → absolute
    if let Ok(path_var) = std::env::var("PATH") {
        for dir in std::env::split_paths(&path_var) {
            let p = dir.join(name);
            if is_executable(&p) {
                return Some(p);
            }
            #[cfg(windows)]
            {
                for ext in ["exe", "cmd", "bat"] {
                    let p = dir.join(format!("{name}.{ext}"));
                    if is_executable(&p) {
                        return Some(p);
                    }
                }
            }
        }
    }
    None
}

fn is_executable(path: &Path) -> bool {
    if !path.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        return fs::metadata(path)
            .map(|m| m.permissions().mode() & 0o111 != 0)
            .unwrap_or(false);
    }
    #[cfg(not(unix))]
    {
        true
    }
}
