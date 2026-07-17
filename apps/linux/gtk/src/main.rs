//! AgentBar Linux GUI host — GTK4 + ksni StatusNotifierItem.
//!
//! Hosts the engine in-process via the ab-core C ABI (same surface as WinUI/Swift).
//! Without the `gui` feature (Windows CI), this is a small stub that starts/stops
//! the engine once and exits.

#[cfg(feature = "gui")]
mod ffi;
#[cfg(feature = "gui")]
mod tray;
#[cfg(feature = "gui")]
mod ui;

#[cfg(feature = "gui")]
fn main() -> gtk::glib::ExitCode {
    ui::run()
}

#[cfg(not(feature = "gui"))]
fn main() {
    // Stub: verify ab-core links and lifecycle works without GTK/DBus.
    let started = ab_core::ffi::ab_engine_start();
    eprintln!("agentbar-gtk stub: ab_engine_start -> {started}");
    let json = unsafe {
        let p = ab_core::ffi::ab_snapshot_json();
        if p.is_null() {
            "{}".into()
        } else {
            let s = std::ffi::CStr::from_ptr(p)
                .to_string_lossy()
                .into_owned();
            ab_core::ffi::ab_string_free(p);
            s
        }
    };
    eprintln!(
        "agentbar-gtk stub: snapshot len={} (secrets never expected)",
        json.len()
    );
    debug_assert!(!json.contains("apiKey"));
    debug_assert!(!json.contains("cookieHeader"));
    let _ = ab_core::ffi::ab_engine_stop();
}
