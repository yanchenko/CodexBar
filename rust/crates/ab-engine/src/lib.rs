//! Engine runtime: lifecycle, refresh, snapshot store.
//!
//! Process-global state (handle-free C ABI). Worker thread builds fake/empty
//! provider snapshots until real providers land.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use ab_model::{ProviderSnapshot, UsageSnapshot};
use serde::{Deserialize, Serialize};

/// Structured last-error DTO (`ab_last_error_json`).
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LastError {
    pub code: String,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_id: Option<String>,
    pub at: String,
}

/// Host adaptive signals (`ab_set_host_signals_json`).
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HostSignals {
    #[serde(default)]
    pub low_power: bool,
    #[serde(default)]
    pub thermal_serious: bool,
}

struct EngineState {
    running: bool,
    snapshot: UsageSnapshot,
    last_error: Option<LastError>,
    host_signals: HostSignals,
    refresh_interval_secs: u32,
    adaptive: bool,
    menu_opened_at: Option<SystemTime>,
    /// Incremented when snapshot changes (for wait).
    seq: u64,
    stop_flag: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
}

impl EngineState {
    fn new() -> Self {
        Self {
            running: false,
            snapshot: UsageSnapshot::empty(0, now_iso()),
            last_error: None,
            host_signals: HostSignals::default(),
            refresh_interval_secs: 300,
            adaptive: false,
            menu_opened_at: None,
            seq: 0,
            stop_flag: Arc::new(AtomicBool::new(false)),
            worker: None,
        }
    }
}

struct EngineInner {
    state: Mutex<EngineState>,
    /// Notified when seq changes or stop.
    cv: Condvar,
}

impl EngineInner {
    fn new() -> Self {
        Self {
            state: Mutex::new(EngineState::new()),
            cv: Condvar::new(),
        }
    }
}

static ENGINE: Mutex<Option<Arc<EngineInner>>> = Mutex::new(None);
static SNAPSHOT_SEQ: AtomicU64 = AtomicU64::new(0);

fn engine() -> Arc<EngineInner> {
    let mut g = ENGINE.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(e) = g.as_ref() {
        return Arc::clone(e);
    }
    let e = Arc::new(EngineInner::new());
    *g = Some(Arc::clone(&e));
    e
}

fn now_iso() -> String {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    // Compact UTC-ish stamp (full chrono optional later).
    format!("{secs}")
}

fn iso_rfc3339ish() -> String {
    // Well-formed ISO-8601 without a time crate: epoch date + unix seconds as fractional.
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    format!("1970-01-01T00:00:00.{secs:010}Z")
}

/// Start engine worker if not running. Returns true if running after call.
pub fn start() -> bool {
    ab_log::init();
    let eng = engine();
    let mut st = eng.state.lock().unwrap_or_else(|e| e.into_inner());
    if st.running {
        return true;
    }

    // Ensure sticky config is bound (do not clobber a path set by tests / migrate).
    if ab_config::sticky_path_bound().is_none()
        && let Some(resolved) = ab_config::resolve_path()
    {
        ab_config::bind_sticky(&resolved);
        if resolved.will_create {
            let _ = ab_config::load_raw_from(&resolved.sticky);
        }
    }

    st.stop_flag.store(false, Ordering::SeqCst);
    let stop = Arc::clone(&st.stop_flag);
    let eng_w = Arc::clone(&eng);
    let handle = thread::Builder::new()
        .name("ab-engine-refresh".into())
        .spawn(move || worker_loop(eng_w, stop))
        .ok();

    if handle.is_none() {
        set_error_locked(
            &mut st,
            "engine.start_failed",
            "failed to spawn worker",
            None,
        );
        return false;
    }
    st.worker = handle;
    st.running = true;
    ab_log::info("engine", "engine started");
    // Immediate empty snapshot so hosts have something to show.
    publish_fake_snapshot(&mut st, &eng);
    true
}

/// Stop worker and join. Returns true if it was running.
pub fn stop() -> bool {
    let eng = engine();
    let mut st = eng.state.lock().unwrap_or_else(|e| e.into_inner());
    if !st.running {
        return false;
    }
    st.stop_flag.store(true, Ordering::SeqCst);
    eng.cv.notify_all();
    let worker = st.worker.take();
    st.running = false;
    drop(st);
    if let Some(h) = worker {
        let _ = h.join();
    }
    ab_log::info("engine", "engine stopped");
    true
}

/// Re-read config from sticky path. Returns true if engine is running.
pub fn reload() -> bool {
    let eng = engine();
    let mut st = eng.state.lock().unwrap_or_else(|e| e.into_inner());
    match ab_config::load_raw() {
        Ok(_) => {
            ab_log::info("config", "config reloaded from sticky path");
            // Refresh snapshot shape from config (fake providers for now).
            publish_fake_snapshot(&mut st, &eng);
            st.running
        }
        Err(e) => {
            set_error_locked(&mut st, "config.io", &format!("reload failed: {e}"), None);
            false
        }
    }
}

pub fn is_running() -> bool {
    engine()
        .state
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .running
}

/// Current snapshot as JSON string (no secrets).
pub fn snapshot_json() -> String {
    let eng = engine();
    let st = eng.state.lock().unwrap_or_else(|e| e.into_inner());
    st.snapshot.to_json_string().unwrap_or_else(|_| "{}".into())
}

/// Block until seq != since_seq or timeout. Returns current snapshot JSON.
pub fn snapshot_wait(since_seq: u64, timeout_ms: u32) -> String {
    let eng = engine();
    let timeout = Duration::from_millis(u64::from(timeout_ms));
    let st = eng.state.lock().unwrap_or_else(|e| e.into_inner());
    if st.snapshot.seq != since_seq {
        return st.snapshot.to_json_string().unwrap_or_else(|_| "{}".into());
    }
    let (guard, _timeout_result) = eng
        .cv
        .wait_timeout_while(st, timeout, |s| {
            s.snapshot.seq == since_seq && !s.stop_flag.load(Ordering::SeqCst)
        })
        .unwrap_or_else(|e| e.into_inner());
    guard
        .snapshot
        .to_json_string()
        .unwrap_or_else(|_| "{}".into())
}

pub fn sticky_config_path() -> Option<PathBuf> {
    ab_config::sticky_path()
}

pub fn apply_patch_file(path: &Path) -> bool {
    let eng = engine();
    match ab_config::apply_patch_file(path) {
        Ok(_) => {
            ab_log::info("config", "apply_patch_file ok");
            true
        }
        Err(e) => {
            let mut st = eng.state.lock().unwrap_or_else(|e| e.into_inner());
            set_error_locked(
                &mut st,
                "config.patch_invalid",
                &format!("apply_patch_file: {e}"),
                None,
            );
            false
        }
    }
}

pub fn note_menu_opened() {
    let eng = engine();
    let mut st = eng.state.lock().unwrap_or_else(|e| e.into_inner());
    st.menu_opened_at = Some(SystemTime::now());
}

pub fn set_host_signals_json(json: &str) -> bool {
    let eng = engine();
    let mut st = eng.state.lock().unwrap_or_else(|e| e.into_inner());
    if json.trim().is_empty() || json.trim() == "{}" {
        st.host_signals = HostSignals::default();
        return true;
    }
    match serde_json::from_str::<HostSignals>(json) {
        Ok(s) => {
            st.host_signals = s;
            true
        }
        Err(e) => {
            set_error_locked(
                &mut st,
                "engine.host_signals_invalid",
                &format!("invalid host signals: {e}"),
                None,
            );
            false
        }
    }
}

pub fn last_error_json() -> String {
    let eng = engine();
    let st = eng.state.lock().unwrap_or_else(|e| e.into_inner());
    match &st.last_error {
        Some(e) => serde_json::to_string(e).unwrap_or_else(|_| "{}".into()),
        None => "{}".into(),
    }
}

pub fn set_refresh_interval_secs(secs: u32) -> bool {
    // Allowed set from design; 0 = manual.
    const ALLOWED: &[u32] = &[0, 60, 120, 300, 900, 1800];
    if !ALLOWED.contains(&secs) {
        let eng = engine();
        let mut st = eng.state.lock().unwrap_or_else(|e| e.into_inner());
        set_error_locked(
            &mut st,
            "engine.bad_interval",
            &format!("refresh interval {secs} not in allowed set"),
            None,
        );
        return false;
    }
    let eng = engine();
    let mut st = eng.state.lock().unwrap_or_else(|e| e.into_inner());
    st.refresh_interval_secs = secs;
    true
}

pub fn set_adaptive_refresh(on: bool) -> bool {
    let eng = engine();
    let mut st = eng.state.lock().unwrap_or_else(|e| e.into_inner());
    st.adaptive = on;
    true
}

pub fn refresh_now() -> bool {
    let eng = engine();
    let mut st = eng.state.lock().unwrap_or_else(|e| e.into_inner());
    if !st.running {
        return false;
    }
    publish_fake_snapshot(&mut st, &eng);
    true
}

/// Log / data dirs (best-effort platform paths).
pub fn log_dir() -> PathBuf {
    data_dir().join("logs")
}

pub fn data_dir() -> PathBuf {
    #[cfg(windows)]
    {
        if let Ok(appdata) = std::env::var("APPDATA") {
            return PathBuf::from(appdata).join("AgentBar");
        }
    }
    #[cfg(target_os = "macos")]
    {
        if let Some(home) = ab_config::home_dir() {
            return home
                .join("Library")
                .join("Application Support")
                .join("AgentBar");
        }
    }
    if let Some(home) = ab_config::home_dir() {
        return home.join(".local").join("share").join("agentbar");
    }
    PathBuf::from("agentbar-data")
}

fn set_error_locked(st: &mut EngineState, code: &str, message: &str, provider_id: Option<String>) {
    st.last_error = Some(LastError {
        code: code.into(),
        message: message.into(),
        provider_id,
        at: now_iso(),
    });
}

fn publish_fake_snapshot(st: &mut EngineState, eng: &EngineInner) {
    let seq = SNAPSHOT_SEQ.fetch_add(1, Ordering::SeqCst) + 1;
    let updated = iso_rfc3339ish();
    // Fake empty providers from config enable flags when possible.
    let mut providers = Vec::new();
    if let Ok(cfg) = ab_config::load_raw()
        && let Some(arr) = cfg.get("providers").and_then(|v| v.as_array())
    {
        for p in arr {
            let id = p
                .get("id")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown")
                .to_string();
            let enabled = p.get("enabled").and_then(|v| v.as_bool()).unwrap_or(false);
            if !enabled {
                continue;
            }
            // Never put secrets into snapshot — only id/enabled.
            let mut row = ProviderSnapshot::ok(&id, &updated);
            row.enabled = enabled;
            row.source_label = Some("none".into());
            // Placeholder: no real probe yet.
            row.error = Some("provider probe not implemented".into());
            row.error_code = Some("not_implemented".into());
            providers.push(row);
        }
    }
    st.seq = seq;
    st.snapshot = UsageSnapshot {
        schema_version: ab_model::SCHEMA_VERSION,
        seq,
        updated_at: updated,
        refreshing: false,
        providers,
    };
    eng.cv.notify_all();
}

fn worker_loop(eng: Arc<EngineInner>, stop: Arc<AtomicBool>) {
    while !stop.load(Ordering::SeqCst) {
        // Sleep in short slices so stop is responsive.
        for _ in 0..50 {
            if stop.load(Ordering::SeqCst) {
                return;
            }
            thread::sleep(Duration::from_millis(100));
        }
        if stop.load(Ordering::SeqCst) {
            return;
        }
        let mut st = eng.state.lock().unwrap_or_else(|e| e.into_inner());
        if st.refresh_interval_secs == 0 && !st.adaptive {
            // Manual only — skip periodic refresh.
            continue;
        }
        publish_fake_snapshot(&mut st, &eng);
    }
}

/// Reset process engine state (unit/integration tests only — not for production hosts).
pub fn reset_for_test() {
    let _ = stop();
    *ENGINE.lock().unwrap_or_else(|e| e.into_inner()) = None;
    SNAPSHOT_SEQ.store(0, Ordering::SeqCst);
    ab_config::clear_sticky();
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;
    use std::sync::Mutex;

    static TEST_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn start_stop_snapshot_no_secrets() {
        let _g = TEST_LOCK.lock().unwrap();
        reset_for_test();
        let tmp = tempfile::tempdir().unwrap();
        let cfg = tmp.path().join("config.json");
        std::fs::write(
            &cfg,
            r#"{
              "version": 1,
              "providers": [
                { "id": "codex", "enabled": true, "apiKey": "SECRET_KEY_XYZ" },
                { "id": "cursor", "enabled": true, "cookieHeader": "session=SECRET_COOKIE" }
              ]
            }"#,
        )
        .unwrap();
        ab_config::set_sticky_path(cfg);

        assert!(start());
        assert!(is_running());
        let json = snapshot_json();
        assert!(
            !json.contains("SECRET_KEY_XYZ"),
            "snapshot leaked apiKey: {json}"
        );
        assert!(
            !json.contains("SECRET_COOKIE"),
            "snapshot leaked cookie: {json}"
        );
        assert!(!json.contains("apiKey"));
        assert!(!json.contains("cookieHeader"));
        let v: Value = serde_json::from_str(&json).unwrap();
        assert_eq!(v["schemaVersion"], 1);
        assert!(v["providers"].as_array().unwrap().len() >= 1);
        assert!(stop());
        assert!(!is_running());
    }

    #[test]
    fn apply_patch_preserves_unknowns() {
        let _g = TEST_LOCK.lock().unwrap();
        reset_for_test();
        let tmp = tempfile::tempdir().unwrap();
        let cfg = tmp.path().join("config.json");
        std::fs::write(
            &cfg,
            r#"{
              "version": 1,
              "hooks": { "x": 1 },
              "providers": [
                { "id": "codex", "enabled": false, "apiKey": "keep" },
                { "id": "other", "enabled": true, "apiKey": "other-secret" }
              ]
            }"#,
        )
        .unwrap();
        ab_config::set_sticky_path(cfg.clone());

        let patch = tmp.path().join("patch.json");
        std::fs::write(&patch, r#"{"providers":[{"id":"codex","enabled":true}]}"#).unwrap();
        assert!(apply_patch_file(&patch));
        let text = std::fs::read_to_string(&cfg).unwrap();
        let v: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(v["hooks"]["x"], 1);
        assert_eq!(v["providers"][0]["enabled"], true);
        assert_eq!(v["providers"][0]["apiKey"], "keep");
        assert_eq!(v["providers"][1]["apiKey"], "other-secret");
    }

    #[test]
    fn snapshot_wait_returns_on_change() {
        let _g = TEST_LOCK.lock().unwrap();
        reset_for_test();
        let tmp = tempfile::tempdir().unwrap();
        let cfg = tmp.path().join("config.json");
        std::fs::write(&cfg, r#"{"version":1,"providers":[]}"#).unwrap();
        ab_config::set_sticky_path(cfg);

        assert!(start());
        let json = snapshot_json();
        let v: Value = serde_json::from_str(&json).unwrap();
        let seq = v["seq"].as_u64().unwrap();
        assert!(refresh_now());
        let waited = snapshot_wait(seq, 2000);
        let v2: Value = serde_json::from_str(&waited).unwrap();
        assert!(v2["seq"].as_u64().unwrap() != seq || v2["seq"] == seq);
        // After refresh_now, seq should advance
        assert!(v2["seq"].as_u64().unwrap() > seq);
        stop();
    }
}
