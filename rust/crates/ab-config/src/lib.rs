//! AgentBar config: sticky path resolution + merge-patch preserve.
//!
//! Primary path: `~/.config/agentbar/config.json` with CodexBar read-compat.
//! All saves write only to the sticky path (no dual-write).
//! Merge-patch preserves unknown top-level keys, unmentioned providers, and
//! sibling fields on patched provider objects.

use std::env;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde_json::{Map, Value};

/// Env override for AgentBar config absolute path.
pub const ENV_AGENTBAR_CONFIG: &str = "AGENTBAR_CONFIG";
/// Legacy CodexBar config env override (compat).
pub const ENV_CODEXBAR_CONFIG: &str = "CODEXBAR_CONFIG";

/// Process-global sticky write target after resolve/load.
static STICKY: Mutex<Option<PathBuf>> = Mutex::new(None);

/// Warning flag: both AgentBar + CodexBar files existed (one-shot).
static BOTH_FILES_WARNED: Mutex<bool> = Mutex::new(false);

/// Result of path resolution.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResolvedPath {
    /// Sticky write target (and preferred read path).
    pub sticky: PathBuf,
    /// True when AgentBar and CodexBar files both existed at resolve time.
    pub both_agentbar_and_codexbar: bool,
    /// True when the sticky file does not exist yet (create path).
    pub will_create: bool,
}

/// Home directory (`USERPROFILE` / `HOME`).
pub fn home_dir() -> Option<PathBuf> {
    env::var_os("USERPROFILE")
        .or_else(|| env::var_os("HOME"))
        .map(PathBuf::from)
}

fn xdg_config_home() -> Option<PathBuf> {
    env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
}

/// Default create path: `$XDG_CONFIG_HOME/agentbar/config.json` or `~/.config/agentbar/config.json`.
pub fn default_agentbar_config_path() -> Option<PathBuf> {
    if let Some(xdg) = xdg_config_home() {
        return Some(xdg.join("agentbar").join("config.json"));
    }
    Some(home_dir()?.join(".config").join("agentbar").join("config.json"))
}

fn agentbar_candidates() -> Vec<PathBuf> {
    let mut v = Vec::new();
    if let Some(xdg) = xdg_config_home() {
        v.push(xdg.join("agentbar").join("config.json"));
    }
    if let Some(home) = home_dir() {
        let p = home.join(".config").join("agentbar").join("config.json");
        if !v.iter().any(|x| x == &p) {
            v.push(p);
        }
    }
    v
}

fn codexbar_candidates() -> Vec<PathBuf> {
    let mut v = Vec::new();
    if let Some(xdg) = xdg_config_home() {
        v.push(xdg.join("codexbar").join("config.json"));
    }
    if let Some(home) = home_dir() {
        let p = home.join(".config").join("codexbar").join("config.json");
        if !v.iter().any(|x| x == &p) {
            v.push(p);
        }
        v.push(home.join(".codexbar").join("config.json"));
    }
    v
}

/// Resolve sticky path per design order (does not bind process sticky until [`bind_sticky`]).
pub fn resolve_path() -> Option<ResolvedPath> {
    // 1. AGENTBAR_CONFIG
    if let Some(p) = env::var_os(ENV_AGENTBAR_CONFIG) {
        let path = PathBuf::from(p);
        if path.is_absolute() {
            let exists = path.is_file();
            return Some(ResolvedPath {
                sticky: path,
                both_agentbar_and_codexbar: false,
                will_create: !exists,
            });
        }
    }
    // 2. CODEXBAR_CONFIG
    if let Some(p) = env::var_os(ENV_CODEXBAR_CONFIG) {
        let path = PathBuf::from(p);
        if path.is_absolute() {
            let exists = path.is_file();
            return Some(ResolvedPath {
                sticky: path,
                both_agentbar_and_codexbar: false,
                will_create: !exists,
            });
        }
    }

    let agentbar_existing: Vec<PathBuf> = agentbar_candidates()
        .into_iter()
        .filter(|p| p.is_file())
        .collect();
    let codexbar_existing: Vec<PathBuf> = codexbar_candidates()
        .into_iter()
        .filter(|p| p.is_file())
        .collect();

    let both = !agentbar_existing.is_empty() && !codexbar_existing.is_empty();

    // 3–4. AgentBar present
    if let Some(p) = agentbar_existing.into_iter().next() {
        return Some(ResolvedPath {
            sticky: p,
            both_agentbar_and_codexbar: both,
            will_create: false,
        });
    }

    // 5–7. CodexBar / legacy present — sticky in place
    if let Some(p) = codexbar_existing.into_iter().next() {
        return Some(ResolvedPath {
            sticky: p,
            both_agentbar_and_codexbar: false,
            will_create: false,
        });
    }

    // 8. Create default AgentBar path
    let create = default_agentbar_config_path()?;
    Some(ResolvedPath {
        sticky: create,
        both_agentbar_and_codexbar: false,
        will_create: true,
    })
}

/// Bind sticky path for the process (after resolve). Logs both-files warning once.
pub fn bind_sticky(resolved: &ResolvedPath) {
    if resolved.both_agentbar_and_codexbar {
        let mut warned = BOTH_FILES_WARNED.lock().unwrap_or_else(|e| e.into_inner());
        if !*warned {
            ab_log::warn(
                "config",
                "both AgentBar and CodexBar config files exist; using AgentBar sticky path \
                 (CodexBar file ignored for read/write — risk of drift if edited by hand)",
            );
            *warned = true;
        }
    }
    *STICKY.lock().unwrap_or_else(|e| e.into_inner()) = Some(resolved.sticky.clone());
}

/// Current sticky write target, resolving+binding if not yet set.
pub fn sticky_path() -> Option<PathBuf> {
    {
        let g = STICKY.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(p) = g.as_ref() {
            return Some(p.clone());
        }
    }
    let r = resolve_path()?;
    bind_sticky(&r);
    Some(r.sticky)
}

/// Override sticky path (tests / migrate rebind).
pub fn set_sticky_path(path: PathBuf) {
    *STICKY.lock().unwrap_or_else(|e| e.into_inner()) = Some(path);
}

/// Clear process sticky (tests).
pub fn clear_sticky() {
    *STICKY.lock().unwrap_or_else(|e| e.into_inner()) = None;
    *BOTH_FILES_WARNED.lock().unwrap_or_else(|e| e.into_inner()) = false;
}

/// Default config document (Codex enabled; Claude/Cursor disabled).
pub fn default_config_value() -> Value {
    serde_json::json!({
        "version": 1,
        "providers": [
            { "id": "codex", "enabled": true },
            { "id": "claude", "enabled": false },
            { "id": "cursor", "enabled": false }
        ]
    })
}

/// Load sticky config as raw JSON. Creates defaults if missing.
pub fn load_raw() -> io::Result<Value> {
    let path = sticky_path().ok_or_else(|| {
        io::Error::new(io::ErrorKind::NotFound, "could not resolve config path")
    })?;
    load_raw_from(&path)
}

/// Load from an explicit path; if missing, write defaults and return them.
pub fn load_raw_from(path: &Path) -> io::Result<Value> {
    if path.is_file() {
        let text = fs::read_to_string(path)?;
        let v: Value = serde_json::from_str(&text)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
        return Ok(v);
    }
    let defaults = default_config_value();
    write_atomic(path, &defaults)?;
    Ok(defaults)
}

/// Apply a merge-patch document onto sticky config and write atomically.
pub fn apply_patch(patch: &Value) -> io::Result<Value> {
    let path = sticky_path().ok_or_else(|| {
        io::Error::new(io::ErrorKind::NotFound, "could not resolve config path")
    })?;
    apply_patch_at(&path, patch)
}

/// Apply patch at an explicit path (tests / migrate).
pub fn apply_patch_at(path: &Path, patch: &Value) -> io::Result<Value> {
    let mut base = if path.is_file() {
        let text = fs::read_to_string(path)?;
        serde_json::from_str(&text)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?
    } else {
        default_config_value()
    };
    merge_patch(&mut base, patch);
    write_atomic(path, &base)?;
    Ok(base)
}

/// Load patch JSON from a host-written file path and apply to sticky config.
pub fn apply_patch_file(patch_path: &Path) -> io::Result<Value> {
    let text = fs::read_to_string(patch_path)?;
    let patch: Value = serde_json::from_str(&text)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    apply_patch(&patch)
}

/// Deep merge `patch` into `base` (RFC 7396-ish for objects; `providers[]` merged by `id`).
pub fn merge_patch(base: &mut Value, patch: &Value) {
    match (base, patch) {
        (Value::Object(base_map), Value::Object(patch_map)) => {
            for (k, pv) in patch_map {
                if k == "providers" {
                    if let Some(patch_arr) = pv.as_array() {
                        merge_providers(base_map, patch_arr);
                        continue;
                    }
                }
                if pv.is_null() {
                    // JSON Merge Patch: null deletes. We still allow it for known keys.
                    base_map.remove(k);
                    continue;
                }
                match base_map.get_mut(k) {
                    Some(bv) if bv.is_object() && pv.is_object() => {
                        merge_patch(bv, pv);
                    }
                    _ => {
                        base_map.insert(k.clone(), pv.clone());
                    }
                }
            }
        }
        (base, patch) => {
            *base = patch.clone();
        }
    }
}

fn merge_providers(base_map: &mut Map<String, Value>, patch_arr: &[Value]) {
    let base_arr = base_map
        .entry("providers".to_string())
        .or_insert_with(|| Value::Array(Vec::new()));
    let Some(arr) = base_arr.as_array_mut() else {
        *base_arr = Value::Array(patch_arr.to_vec());
        return;
    };

    for patch_item in patch_arr {
        let Some(pid) = patch_item.get("id").and_then(|v| v.as_str()) else {
            // No id — append as-is
            arr.push(patch_item.clone());
            continue;
        };
        if let Some(existing) = arr.iter_mut().find(|e| {
            e.get("id")
                .and_then(|v| v.as_str())
                .is_some_and(|id| id == pid)
        }) {
            // Deep-merge object keys; unmentioned siblings preserved.
            merge_patch(existing, patch_item);
        } else {
            arr.push(patch_item.clone());
        }
    }
}

/// Atomic write (temp + rename) with best-effort 0600 on Unix.
pub fn write_atomic(path: &Path, value: &Value) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let text = serde_json::to_string_pretty(value)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    let dir = path.parent().unwrap_or_else(|| Path::new("."));
    let mut tmp_name = path
        .file_name()
        .map(|s| s.to_os_string())
        .unwrap_or_else(|| "config.json".into());
    tmp_name.push(".tmp");
    let tmp_path = dir.join(tmp_name);
    {
        let mut f = fs::File::create(&tmp_path)?;
        f.write_all(text.as_bytes())?;
        f.write_all(b"\n")?;
        f.sync_all()?;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(&tmp_path, fs::Permissions::from_mode(0o600));
    }
    // Windows: rename over existing is allowed; on some FS need remove first.
    if path.exists() {
        let _ = fs::remove_file(path);
    }
    fs::rename(&tmp_path, path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(path, fs::Permissions::from_mode(0o600));
    }
    Ok(())
}

/// Optional: copy sticky content to default AgentBar path and rebind sticky.
pub fn migrate_to_agentbar() -> io::Result<PathBuf> {
    let dest = default_agentbar_config_path().ok_or_else(|| {
        io::Error::new(io::ErrorKind::NotFound, "no home for agentbar path")
    })?;
    let src = sticky_path().ok_or_else(|| {
        io::Error::new(io::ErrorKind::NotFound, "no sticky path")
    })?;
    if src == dest {
        return Ok(dest);
    }
    let value = load_raw_from(&src)?;
    write_atomic(&dest, &value)?;
    set_sticky_path(dest.clone());
    Ok(dest)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::sync::Mutex;

    /// Serialize tests that touch process-global sticky / env.
    static TEST_LOCK: Mutex<()> = Mutex::new(());

    fn with_temp_home(f: impl FnOnce(&Path)) {
        let _g = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        clear_sticky();
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().join("home");
        fs::create_dir_all(&home).unwrap();
        // Isolate from real machine paths
        let old_home = env::var_os("HOME");
        let old_up = env::var_os("USERPROFILE");
        let old_xdg = env::var_os("XDG_CONFIG_HOME");
        let old_ab = env::var_os(ENV_AGENTBAR_CONFIG);
        let old_cb = env::var_os(ENV_CODEXBAR_CONFIG);
        // SAFETY: single-threaded under TEST_LOCK; restored after.
        unsafe {
            env::set_var("HOME", &home);
            env::set_var("USERPROFILE", &home);
            env::remove_var("XDG_CONFIG_HOME");
            env::remove_var(ENV_AGENTBAR_CONFIG);
            env::remove_var(ENV_CODEXBAR_CONFIG);
        }
        f(&home);
        clear_sticky();
        unsafe {
            restore_env("HOME", old_home);
            restore_env("USERPROFILE", old_up);
            restore_env("XDG_CONFIG_HOME", old_xdg);
            restore_env(ENV_AGENTBAR_CONFIG, old_ab);
            restore_env(ENV_CODEXBAR_CONFIG, old_cb);
        }
    }

    unsafe fn restore_env(key: &str, val: Option<std::ffi::OsString>) {
        match val {
            Some(v) => unsafe { env::set_var(key, v) },
            None => unsafe { env::remove_var(key) },
        }
    }

    #[test]
    fn resolve_create_default_agentbar() {
        with_temp_home(|home| {
            let r = resolve_path().unwrap();
            assert!(r.will_create);
            assert_eq!(
                r.sticky,
                home.join(".config").join("agentbar").join("config.json")
            );
            assert!(!r.both_agentbar_and_codexbar);
        });
    }

    #[test]
    fn resolve_agentbar_only() {
        with_temp_home(|home| {
            let p = home.join(".config").join("agentbar").join("config.json");
            fs::create_dir_all(p.parent().unwrap()).unwrap();
            fs::write(&p, r#"{"version":1,"providers":[]}"#).unwrap();
            let r = resolve_path().unwrap();
            assert_eq!(r.sticky, p);
            assert!(!r.will_create);
        });
    }

    #[test]
    fn resolve_codexbar_only_sticky_in_place() {
        with_temp_home(|home| {
            let p = home.join(".config").join("codexbar").join("config.json");
            fs::create_dir_all(p.parent().unwrap()).unwrap();
            fs::write(&p, r#"{"version":1,"providers":[]}"#).unwrap();
            let r = resolve_path().unwrap();
            assert_eq!(r.sticky, p);
            assert!(!r.will_create);
        });
    }

    #[test]
    fn resolve_both_prefers_agentbar() {
        with_temp_home(|home| {
            let ab = home.join(".config").join("agentbar").join("config.json");
            let cb = home.join(".config").join("codexbar").join("config.json");
            fs::create_dir_all(ab.parent().unwrap()).unwrap();
            fs::create_dir_all(cb.parent().unwrap()).unwrap();
            fs::write(&ab, r#"{"version":1}"#).unwrap();
            fs::write(&cb, r#"{"version":1}"#).unwrap();
            let r = resolve_path().unwrap();
            assert_eq!(r.sticky, ab);
            assert!(r.both_agentbar_and_codexbar);
        });
    }

    #[test]
    fn resolve_env_agentbar_override() {
        with_temp_home(|_home| {
            let tmp = tempfile::tempdir().unwrap();
            let p = tmp.path().join("custom.json");
            fs::write(&p, r#"{"version":1}"#).unwrap();
            unsafe {
                env::set_var(ENV_AGENTBAR_CONFIG, &p);
            }
            let r = resolve_path().unwrap();
            assert_eq!(r.sticky, p);
        });
    }

    #[test]
    fn merge_patch_preserves_unknowns_and_secrets_golden() {
        let mut base = json!({
            "version": 1,
            "customTopLevel": { "keep": true },
            "hooks": { "onRefresh": "echo hi" },
            "providers": [
                { "id": "codex", "enabled": false, "apiKey": "codex-secret" },
                { "id": "claude", "enabled": true, "apiKey": "claude-secret" },
                { "id": "cursor", "enabled": false, "cookieHeader": "session=abc" },
                { "id": "grok", "enabled": true, "apiKey": "grok-secret" },
                { "id": "gemini", "enabled": false, "apiKey": "gem-secret" },
                { "id": "copilot", "enabled": true, "token": "copilot-tok" },
                { "id": "openai", "enabled": false, "apiKey": "oai-secret" },
                { "id": "kimi", "enabled": true, "apiKey": "kimi-secret" },
                { "id": "minimax", "enabled": false, "apiKey": "mm-secret" },
                { "id": "zai", "enabled": true, "apiKey": "zai-secret", "extra": 1 }
            ]
        });
        let patch = json!({
            "providers": [
                { "id": "codex", "enabled": true }
            ]
        });
        merge_patch(&mut base, &patch);

        assert_eq!(base["customTopLevel"]["keep"], true);
        assert_eq!(base["hooks"]["onRefresh"], "echo hi");
        let providers = base["providers"].as_array().unwrap();
        assert_eq!(providers.len(), 10);

        let codex = providers.iter().find(|p| p["id"] == "codex").unwrap();
        assert_eq!(codex["enabled"], true);
        assert_eq!(codex["apiKey"], "codex-secret");

        let zai = providers.iter().find(|p| p["id"] == "zai").unwrap();
        assert_eq!(zai["apiKey"], "zai-secret");
        assert_eq!(zai["extra"], 1);

        let cursor = providers.iter().find(|p| p["id"] == "cursor").unwrap();
        assert_eq!(cursor["cookieHeader"], "session=abc");
    }

    #[test]
    fn apply_patch_file_round_trip() {
        with_temp_home(|home| {
            let cfg = home.join(".config").join("agentbar").join("config.json");
            fs::create_dir_all(cfg.parent().unwrap()).unwrap();
            let initial = json!({
                "version": 1,
                "mystery": 42,
                "providers": [
                    { "id": "codex", "enabled": false, "apiKey": "s3cret" },
                    { "id": "other", "enabled": true, "apiKey": "keep-me" }
                ]
            });
            write_atomic(&cfg, &initial).unwrap();
            set_sticky_path(cfg.clone());

            let patch_path = home.join("patch.json");
            fs::write(
                &patch_path,
                r#"{"providers":[{"id":"codex","enabled":true}]}"#,
            )
            .unwrap();
            let out = apply_patch_file(&patch_path).unwrap();
            assert_eq!(out["mystery"], 42);
            assert_eq!(out["providers"][0]["enabled"], true);
            assert_eq!(out["providers"][0]["apiKey"], "s3cret");
            assert_eq!(out["providers"][1]["apiKey"], "keep-me");
        });
    }

    #[test]
    fn migrate_rebinds_sticky() {
        with_temp_home(|home| {
            let cb = home.join(".config").join("codexbar").join("config.json");
            fs::create_dir_all(cb.parent().unwrap()).unwrap();
            write_atomic(&cb, &json!({"version":1,"providers":[]})).unwrap();
            set_sticky_path(cb.clone());
            let dest = migrate_to_agentbar().unwrap();
            assert_eq!(
                dest,
                home.join(".config").join("agentbar").join("config.json")
            );
            assert!(dest.is_file());
            assert_eq!(sticky_path().unwrap(), dest);
            // Old file not deleted
            assert!(cb.is_file());
        });
    }
}
