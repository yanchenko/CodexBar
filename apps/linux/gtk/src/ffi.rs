//! Safe Rust wrappers over the ab-core C ABI (`agentbar.h`).

use std::ffi::{CStr, CString};
use std::os::raw::c_char;

use ab_core::ffi as sys;

/// Copy an owned C string from an `ab_*` return into a Rust `String`, then free. NULL → "".
fn take(p: *mut c_char) -> String {
    if p.is_null() {
        return String::new();
    }
    // SAFETY: non-null ab_* returns are valid NUL-terminated owned strings until ab_string_free.
    let s = unsafe { CStr::from_ptr(p) }.to_string_lossy().into_owned();
    sys::ab_string_free(p);
    s
}

pub fn engine_start() -> bool {
    sys::ab_engine_start() != 0
}
pub fn engine_stop() -> bool {
    sys::ab_engine_stop() != 0
}
pub fn engine_running() -> bool {
    sys::ab_engine_running() != 0
}
pub fn refresh_now() -> bool {
    sys::ab_refresh_now() != 0
}
pub fn note_menu_opened() {
    sys::ab_note_menu_opened();
}

pub fn snapshot_json() -> String {
    take(sys::ab_snapshot_json())
}

/// BLOCKING until seq differs or timeout. Background thread only.
pub fn snapshot_wait(since: u64, timeout_ms: u32) -> String {
    take(sys::ab_snapshot_wait(since, timeout_ms))
}

pub fn version() -> String {
    take(sys::ab_version())
}

pub fn config_path() -> String {
    take(sys::ab_config_path())
}

#[allow(dead_code)]
pub fn apply_patch_file(path: &str) -> bool {
    let c = CString::new(path).unwrap_or_default();
    sys::ab_config_apply_patch_file(c.as_ptr()) != 0
}
