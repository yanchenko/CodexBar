//! Stable C ABI (`agentbar.h`). Handle-free; owned strings free with [`ab_string_free`].
//!
//! Panic fence: every `extern "C"` uses `catch_unwind`. Host builds MUST use
//! `cargo build --profile release-ffi -p ab-core` (panic=unwind).
#![allow(clippy::not_unsafe_ptr_arg_deref)]

use std::ffi::{CString, c_char};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::Path;

// `catch_unwind` is a no-op under panic=abort. Host builds MUST use release-ffi.
#[cfg(panic = "abort")]
compile_error!(
    "ab-core must not be built with panic=\"abort\" — extern \"C\" relies on catch_unwind. \
     Use `cargo build --profile release-ffi -p ab-core` (or any panic=unwind profile)."
);

/// Product / workspace version string. Owned `char*`; free with [`ab_string_free`].
#[unsafe(no_mangle)]
pub extern "C" fn ab_version() -> *mut c_char {
    guard_str("", || to_cstring(crate::VERSION))
}

/// Start engine worker if not running. `1` if running after call, `0` on failure.
#[unsafe(no_mangle)]
pub extern "C" fn ab_engine_start() -> u8 {
    guard_val(0, || ab_engine::start() as u8)
}

/// Stop engine and join worker. `1` if it was running.
#[unsafe(no_mangle)]
pub extern "C" fn ab_engine_stop() -> u8 {
    guard_val(0, || ab_engine::stop() as u8)
}

/// Re-read sticky config from disk. `1` if engine running after reload.
#[unsafe(no_mangle)]
pub extern "C" fn ab_engine_reload() -> u8 {
    guard_val(0, || ab_engine::reload() as u8)
}

/// `1` if engine is running.
#[unsafe(no_mangle)]
pub extern "C" fn ab_engine_running() -> u8 {
    guard_val(0, || ab_engine::is_running() as u8)
}

/// Request an immediate refresh (coalesced). `1` if accepted.
#[unsafe(no_mangle)]
pub extern "C" fn ab_refresh_now() -> u8 {
    guard_val(0, || ab_engine::refresh_now() as u8)
}

/// Set fixed refresh interval seconds (allowed set; `0` = manual).
#[unsafe(no_mangle)]
pub extern "C" fn ab_set_refresh_interval_secs(secs: u32) -> u8 {
    guard_val(0, || ab_engine::set_refresh_interval_secs(secs) as u8)
}

/// Enable/disable adaptive refresh (`on != 0`).
#[unsafe(no_mangle)]
pub extern "C" fn ab_set_adaptive_refresh(on: u8) -> u8 {
    guard_val(0, || ab_engine::set_adaptive_refresh(on != 0) as u8)
}

/// Adaptive signal: menu / flyout opened.
#[unsafe(no_mangle)]
pub extern "C" fn ab_note_menu_opened() {
    guard_val((), ab_engine::note_menu_opened);
}

/// Host power/thermal signals JSON. Empty/`{}` clears. `1` on success.
#[unsafe(no_mangle)]
pub extern "C" fn ab_set_host_signals_json(json: *const c_char) -> u8 {
    guard_val(0, || {
        let s = cstr_or_empty(json);
        ab_engine::set_host_signals_json(&s) as u8
    })
}

/// Current snapshot JSON (never secrets). `"{}"` if empty/failure. Owned `char*`.
#[unsafe(no_mangle)]
pub extern "C" fn ab_snapshot_json() -> *mut c_char {
    guard_str("{}", || to_cstring(ab_engine::snapshot_json()))
}

/// Block until snapshot `seq` differs from `since_seq` or `timeout_ms` elapses.
/// Call only from a background thread. Owned `char*`.
#[unsafe(no_mangle)]
pub extern "C" fn ab_snapshot_wait(since_seq: u64, timeout_ms: u32) -> *mut c_char {
    guard_str("{}", || {
        to_cstring(ab_engine::snapshot_wait(since_seq, timeout_ms))
    })
}

/// Sticky config write-target path (not file contents). Owned `char*`.
#[unsafe(no_mangle)]
pub extern "C" fn ab_config_path() -> *mut c_char {
    guard_str("", || match ab_engine::sticky_config_path() {
        Some(p) => to_cstring(p.to_string_lossy().into_owned()),
        None => to_cstring(""),
    })
}

/// Merge-patch sticky config from a host-written JSON file path. Returns `1`/`0` only.
#[unsafe(no_mangle)]
pub extern "C" fn ab_config_apply_patch_file(patch_path: *const c_char) -> u8 {
    guard_val(0, || {
        let path = cstr_or_empty(patch_path);
        if path.is_empty() {
            return 0;
        }
        ab_engine::apply_patch_file(Path::new(&path)) as u8
    })
}

/// Log directory path. Owned `char*`.
#[unsafe(no_mangle)]
pub extern "C" fn ab_log_dir() -> *mut c_char {
    guard_str("", || {
        to_cstring(ab_engine::log_dir().to_string_lossy().into_owned())
    })
}

/// Data directory path. Owned `char*`.
#[unsafe(no_mangle)]
pub extern "C" fn ab_data_dir() -> *mut c_char {
    guard_str("", || {
        to_cstring(ab_engine::data_dir().to_string_lossy().into_owned())
    })
}

/// Provider catalog JSON (static metadata; no secrets). Owned `char*`.
#[unsafe(no_mangle)]
pub extern "C" fn ab_providers_catalog_json() -> *mut c_char {
    guard_str("[]", || {
        // MVP stub catalog — real registry in provider PRs.
        to_cstring(
            r#"[{"id":"codex","displayName":"Codex","defaultEnabled":true},{"id":"claude","displayName":"Claude","defaultEnabled":false},{"id":"cursor","displayName":"Cursor","defaultEnabled":false}]"#,
        )
    })
}

/// Last structured error JSON or `"{}"`. Owned `char*`.
#[unsafe(no_mangle)]
pub extern "C" fn ab_last_error_json() -> *mut c_char {
    guard_str("{}", || to_cstring(ab_engine::last_error_json()))
}

/// Free a string returned by any `ab_*` API. NULL is a no-op.
#[unsafe(no_mangle)]
pub extern "C" fn ab_string_free(s: *mut c_char) {
    guard_val((), || {
        if s.is_null() {
            return;
        }
        // SAFETY: `s` was allocated by `CString::into_raw` in this crate (or is NULL).
        unsafe {
            drop(CString::from_raw(s));
        }
    });
}

fn guard_val<T>(default: T, f: impl FnOnce() -> T) -> T {
    catch_unwind(AssertUnwindSafe(f)).unwrap_or(default)
}

fn guard_str(default: &'static str, f: impl FnOnce() -> *mut c_char) -> *mut c_char {
    catch_unwind(AssertUnwindSafe(f)).unwrap_or_else(|_| to_cstring(default))
}

fn to_cstring(s: impl Into<Vec<u8>>) -> *mut c_char {
    match CString::new(s) {
        Ok(c) => c.into_raw(),
        Err(_) => CString::new("").unwrap().into_raw(),
    }
}

fn cstr_or_empty(p: *const c_char) -> String {
    if p.is_null() {
        return String::new();
    }
    // SAFETY: non-null; C ABI contract — NUL-terminated for the call duration.
    unsafe { std::ffi::CStr::from_ptr(p) }
        .to_str()
        .unwrap_or("")
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::CStr;
    use std::sync::Mutex;

    static TEST_LOCK: Mutex<()> = Mutex::new(());

    unsafe fn take_string(p: *mut c_char) -> String {
        if p.is_null() {
            return String::new();
        }
        let s = unsafe { CStr::from_ptr(p) }.to_string_lossy().into_owned();
        ab_string_free(p);
        s
    }

    #[test]
    fn version_and_lifecycle() {
        let _g = TEST_LOCK.lock().unwrap();
        ab_engine::reset_for_test();
        let ver = unsafe { take_string(ab_version()) };
        assert_eq!(ver, crate::VERSION);

        let tmp = tempfile::tempdir().unwrap();
        let cfg = tmp.path().join("config.json");
        std::fs::write(
            &cfg,
            r#"{"version":1,"providers":[{"id":"codex","enabled":true,"apiKey":"NOPE"}]}"#,
        )
        .unwrap();
        ab_config::set_sticky_path(cfg.clone());

        assert_eq!(ab_engine_start(), 1);
        assert_eq!(ab_engine_running(), 1);

        let snap = unsafe { take_string(ab_snapshot_json()) };
        assert!(!snap.contains("NOPE"));
        assert!(!snap.contains("apiKey"));

        let path = unsafe { take_string(ab_config_path()) };
        assert!(path.contains("config.json"));

        let patch = tmp.path().join("p.json");
        std::fs::write(
            &patch,
            r#"{"providers":[{"id":"codex","enabled":false}],"extraTop":true}"#,
        )
        .unwrap();
        let c_path = CString::new(patch.to_string_lossy().as_bytes()).unwrap();
        assert_eq!(ab_config_apply_patch_file(c_path.as_ptr()), 1);
        assert_eq!(ab_engine_reload(), 1);

        let text = std::fs::read_to_string(&cfg).unwrap();
        assert!(text.contains("extraTop") || text.contains("\"extraTop\""));
        // enabled false preserved apiKey
        assert!(text.contains("NOPE"));

        ab_note_menu_opened();
        let sig = CString::new(r#"{"lowPower":true}"#).unwrap();
        assert_eq!(ab_set_host_signals_json(sig.as_ptr()), 1);

        assert_eq!(ab_engine_stop(), 1);
        assert_eq!(ab_engine_running(), 0);
    }

    #[test]
    fn snapshot_omits_secret_keys() {
        let _g = TEST_LOCK.lock().unwrap();
        ab_engine::reset_for_test();
        let tmp = tempfile::tempdir().unwrap();
        let cfg = tmp.path().join("config.json");
        std::fs::write(
            &cfg,
            r#"{
              "version": 1,
              "providers": [
                { "id": "codex", "enabled": true, "apiKey": "sk-secret" },
                { "id": "cursor", "enabled": true, "cookieHeader": "sess=1" }
              ]
            }"#,
        )
        .unwrap();
        ab_config::set_sticky_path(cfg);
        assert_eq!(ab_engine_start(), 1);
        let snap = unsafe { take_string(ab_snapshot_json()) };
        for banned in ["sk-secret", "sess=1", "apiKey", "cookieHeader"] {
            assert!(
                !snap.contains(banned),
                "snapshot must not contain {banned:?}: {snap}"
            );
        }
        ab_engine_stop();
    }
}
