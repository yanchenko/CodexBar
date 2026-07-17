//! Stable C ABI (scaffold — full surface in PR3).
//!
//! Handle-free; owned strings free with [`ab_string_free`].
#![allow(clippy::not_unsafe_ptr_arg_deref)]

use std::ffi::{CString, c_char};
use std::panic::{AssertUnwindSafe, catch_unwind};

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
