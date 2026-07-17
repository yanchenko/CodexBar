//! Engine runtime: lifecycle, refresh, snapshot store.
//!
//! Process-global state (handle-free C ABI). Worker thread probes MVP providers
//! (Codex / Claude / Cursor) and publishes snapshot JSON (no secrets).

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use ab_model::UsageSnapshot;
use serde::{Deserialize, Serialize};

/// Structured last-error DTO (`ab_last_error_json`).
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LastError {
    pub code: String,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_id: Option<String>,
    /// ISO-8601 / RFC3339 UTC timestamp.
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
            snapshot: UsageSnapshot::empty(0, now_rfc3339()),
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

/// Real UTC RFC3339 timestamp (schema v1 `updatedAt` / `LastError.at`).
pub fn now_rfc3339() -> String {
    let dur = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    format_rfc3339_millis(dur.as_secs(), dur.subsec_millis())
}

/// Format unix epoch as `YYYY-MM-DDTHH:MM:SS.mmmZ` (always UTC, always `Z`).
pub fn format_rfc3339_millis(secs: u64, millis: u32) -> String {
    let (year, month, day, hour, min, sec) = civil_utc_from_unix(secs);
    format!("{year:04}-{month:02}-{day:02}T{hour:02}:{min:02}:{sec:02}.{millis:03}Z")
}

/// Convert unix seconds to (Y, M, D, h, m, s) UTC without external time crates.
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

/// Howard Hinnant civil_from_days (proleptic Gregorian), days since 1970-01-01.
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
    drop(st);
    // Immediate probe so hosts have real rows (or structured auth_missing).
    // Config I/O + network run outside the state lock.
    publish_snapshot(&eng);
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
    // Disk I/O outside the engine state lock.
    let load_ok = ab_config::load_raw().is_ok();
    let mut st = eng.state.lock().unwrap_or_else(|e| e.into_inner());
    if !load_ok {
        set_error_locked(&mut st, "config.io", "reload failed", None);
        return false;
    }
    ab_log::info("config", "config reloaded from sticky path");
    let running = st.running;
    drop(st);
    publish_snapshot(&eng);
    running
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
    // Wake worker so it re-reads interval promptly.
    eng.cv.notify_all();
    true
}

pub fn set_adaptive_refresh(on: bool) -> bool {
    let eng = engine();
    let mut st = eng.state.lock().unwrap_or_else(|e| e.into_inner());
    st.adaptive = on;
    eng.cv.notify_all();
    true
}

pub fn refresh_now() -> bool {
    let eng = engine();
    let running = {
        let st = eng.state.lock().unwrap_or_else(|e| e.into_inner());
        st.running
    };
    if !running {
        return false;
    }
    publish_snapshot(&eng);
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
        at: now_rfc3339(),
    });
}

/// Build snapshot from config + provider probes **without** holding the engine state lock.
fn build_snapshot() -> UsageSnapshot {
    let seq = SNAPSHOT_SEQ.fetch_add(1, Ordering::SeqCst) + 1;
    let updated = now_rfc3339();
    let providers = match ab_config::load_raw() {
        Ok(cfg) => ab_provider::probe_enabled_providers(&cfg, &updated),
        Err(e) => {
            ab_log::warn("engine", &format!("config load for snapshot: {e}"));
            Vec::new()
        }
    };
    UsageSnapshot {
        schema_version: ab_model::SCHEMA_VERSION,
        seq,
        updated_at: updated,
        refreshing: false,
        providers,
    }
}

/// Publish a fresh probed snapshot. Config load + probes run outside the state lock.
fn publish_snapshot(eng: &EngineInner) {
    let snap = build_snapshot();
    let mut st = eng.state.lock().unwrap_or_else(|e| e.into_inner());
    st.seq = snap.seq;
    st.snapshot = snap;
    eng.cv.notify_all();
}

fn worker_loop(eng: Arc<EngineInner>, stop: Arc<AtomicBool>) {
    while !stop.load(Ordering::SeqCst) {
        let (interval_secs, adaptive) = {
            let st = eng.state.lock().unwrap_or_else(|e| e.into_inner());
            (st.refresh_interval_secs, st.adaptive)
        };

        // Manual mode: do not advance seq without refresh_now / start.
        // Adaptive cadence lands in PR10; until then treat adaptive like fixed 300s
        // only when interval is also non-zero, else idle like manual.
        if interval_secs == 0 && !adaptive {
            sleep_interruptible(Duration::from_secs(1), &stop, &eng);
            continue;
        }

        // PR3: honor stored fixed interval (default 300). Adaptive ignored for
        // sleep length until PR10; if adaptive with interval 0, use 300 as floor.
        let sleep_secs = if interval_secs == 0 {
            300u64
        } else {
            u64::from(interval_secs)
        };
        sleep_interruptible(Duration::from_secs(sleep_secs), &stop, &eng);
        if stop.load(Ordering::SeqCst) {
            return;
        }
        // Re-check manual after sleep (interval may have been set to 0).
        let still_auto = {
            let st = eng.state.lock().unwrap_or_else(|e| e.into_inner());
            st.refresh_interval_secs != 0 || st.adaptive
        };
        if still_auto {
            publish_snapshot(&eng);
        }
    }
}

fn sleep_interruptible(total: Duration, stop: &AtomicBool, eng: &EngineInner) {
    // Wake early on stop / interval change via condvar, with 200ms floor slices.
    let deadline = SystemTime::now() + total;
    while !stop.load(Ordering::SeqCst) {
        let now = SystemTime::now();
        if now >= deadline {
            break;
        }
        let remaining = deadline.duration_since(now).unwrap_or(Duration::ZERO);
        let slice = remaining.min(Duration::from_millis(200));
        let st = eng.state.lock().unwrap_or_else(|e| e.into_inner());
        let (_guard, _timeout) = eng
            .cv
            .wait_timeout(st, slice)
            .unwrap_or_else(|e| e.into_inner());
        // If stop was signalled, loop condition exits.
    }
}

/// Record rejection of a non-absolute patch path (FFI contract).
pub fn note_relative_patch_rejected(path: &str) {
    let eng = engine();
    let mut st = eng.state.lock().unwrap_or_else(|e| e.into_inner());
    set_error_locked(
        &mut st,
        "config.patch_invalid",
        &format!("patch_path must be absolute, got: {path}"),
        None,
    );
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
    fn rfc3339_format_is_parseable_shape() {
        // Fixed epoch: 0 → 1970-01-01T00:00:00.000Z
        assert_eq!(format_rfc3339_millis(0, 0), "1970-01-01T00:00:00.000Z");
        // 2026-07-17 roughly: use a known stamp
        // 1_753_000_000 ≈ mid 2025; just check pattern of now_rfc3339
        let s = now_rfc3339();
        assert!(
            s.ends_with('Z') && s.contains('T') && s.len() >= 24,
            "bad iso: {s}"
        );
        // YYYY-MM-DDTHH:MM:SS.mmmZ
        let parts: Vec<&str> = s.split('T').collect();
        assert_eq!(parts.len(), 2);
        let date = parts[0];
        assert_eq!(date.len(), 10);
        assert_eq!(&date[4..5], "-");
        assert_eq!(&date[7..8], "-");
        // Near "now": year 2020+
        let year: i32 = date[0..4].parse().unwrap();
        assert!(year >= 2020, "year should be current-ish: {s}");
    }

    #[test]
    fn civil_from_days_known() {
        // 1970-01-01
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        // 2000-01-01 = 10957 days from 1970-01-01
        assert_eq!(civil_from_days(10957), (2000, 1, 1));
    }

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
        // updatedAt must look like real RFC3339
        let updated = v["updatedAt"].as_str().unwrap();
        assert!(updated.ends_with('Z') && updated.contains('T'));
        assert!(!updated.starts_with("1970-01-01T00:00:00."), "got {updated}");
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

    #[test]
    fn manual_mode_does_not_advance_seq_without_refresh() {
        let _g = TEST_LOCK.lock().unwrap();
        reset_for_test();
        let tmp = tempfile::tempdir().unwrap();
        let cfg = tmp.path().join("config.json");
        std::fs::write(&cfg, r#"{"version":1,"providers":[]}"#).unwrap();
        ab_config::set_sticky_path(cfg);

        assert!(start());
        assert!(set_refresh_interval_secs(0));
        let seq1 = {
            let v: Value = serde_json::from_str(&snapshot_json()).unwrap();
            v["seq"].as_u64().unwrap()
        };
        // Worker would have advanced every 5s before; wait ~1.5s and ensure no advance.
        thread::sleep(Duration::from_millis(1500));
        let seq2 = {
            let v: Value = serde_json::from_str(&snapshot_json()).unwrap();
            v["seq"].as_u64().unwrap()
        };
        assert_eq!(seq1, seq2, "manual mode must not thrash seq");
        assert!(refresh_now());
        let seq3 = {
            let v: Value = serde_json::from_str(&snapshot_json()).unwrap();
            v["seq"].as_u64().unwrap()
        };
        assert!(seq3 > seq2);
        stop();
    }

    #[test]
    fn bad_interval_rejected() {
        let _g = TEST_LOCK.lock().unwrap();
        reset_for_test();
        assert!(!set_refresh_interval_secs(7));
        let err: Value = serde_json::from_str(&last_error_json()).unwrap();
        assert_eq!(err["code"], "engine.bad_interval");
        let at = err["at"].as_str().unwrap();
        assert!(at.contains('T') && at.ends_with('Z'), "at={at}");
    }
}
