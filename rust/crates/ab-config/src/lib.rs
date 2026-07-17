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
use std::sync::atomic::{AtomicBool, Ordering};

use serde_json::{Map, Value};

/// Env override for AgentBar config absolute path.
pub const ENV_AGENTBAR_CONFIG: &str = "AGENTBAR_CONFIG";
/// Legacy CodexBar config env override (compat).
pub const ENV_CODEXBAR_CONFIG: &str = "CODEXBAR_CONFIG";

/// Process-global sticky write target after resolve/load.
static STICKY: Mutex<Option<PathBuf>> = Mutex::new(None);

/// Warning flag: both AgentBar + CodexBar files existed (one-shot).
static BOTH_FILES_WARNED: Mutex<bool> = Mutex::new(false);

/// Process-wide lock for config read-modify-write and atomic writes.
/// All `ab_*` may be called from any host thread; concurrent patches must serialize.
static CONFIG_IO: Mutex<()> = Mutex::new(());

/// One-shot: Windows ACL harden failed / unavailable.
static WIN_ACL_WARNED: AtomicBool = AtomicBool::new(false);

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
    Some(
        home_dir()?
            .join(".config")
            .join("agentbar")
            .join("config.json"),
    )
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

/// Current sticky write target if already bound (does **not** auto-resolve).
pub fn sticky_path_bound() -> Option<PathBuf> {
    STICKY.lock().unwrap_or_else(|e| e.into_inner()).clone()
}

/// Current sticky write target, resolving+binding if not yet set.
pub fn sticky_path() -> Option<PathBuf> {
    if let Some(p) = sticky_path_bound() {
        return Some(p);
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

/// Validate patch root: must be a JSON object; `providers`/`hooks` must not be null;
/// if `providers` is present it must be a JSON array (merge-by-id contract).
pub fn validate_patch(patch: &Value) -> io::Result<()> {
    let Value::Object(map) = patch else {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "config patch root must be a JSON object",
        ));
    };
    // Preserve denylist: null must not wipe entire providers/hooks trees.
    for key in ["providers", "hooks"] {
        if let Some(v) = map.get(key)
            && v.is_null()
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("config patch must not set `{key}` to null (would wipe secrets/preserve data)"),
            ));
        }
    }
    // Wrong-typed providers would replace the whole tree and wipe nested secrets.
    if let Some(v) = map.get("providers")
        && !v.is_array()
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "config patch `providers` must be a JSON array (merge-by-id); object/string/number rejected",
        ));
    }
    Ok(())
}

/// Load sticky config as raw JSON. Creates defaults if missing.
pub fn load_raw() -> io::Result<Value> {
    let path = sticky_path()
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "could not resolve config path"))?;
    load_raw_from(&path)
}

/// Backup path used by [`write_atomic`] (`path` + `.bak`).
pub fn bak_path_for(path: &Path) -> PathBuf {
    let dir = path.parent().unwrap_or_else(|| Path::new("."));
    let mut bak_name = path
        .file_name()
        .map(|s| s.to_os_string())
        .unwrap_or_else(|| "config.json".into());
    bak_name.push(".bak");
    dir.join(bak_name)
}

/// Load from an explicit path under [`CONFIG_IO`].
///
/// If the sticky path is missing but `path.bak` exists (mid-replace crash or concurrent
/// rename window), restores bak before treating as create-defaults. Holding the lock for
/// the full read + optional create path prevents concurrent `load_raw` from overwriting a
/// good write that just finished with defaults.
pub fn load_raw_from(path: &Path) -> io::Result<Value> {
    let _io = CONFIG_IO.lock().unwrap_or_else(|e| e.into_inner());
    load_raw_from_locked(path)
}

/// Load while caller already holds [`CONFIG_IO`].
fn load_raw_from_locked(path: &Path) -> io::Result<Value> {
    if path.is_file() {
        let text = fs::read_to_string(path)?;
        let v: Value = serde_json::from_str(&text)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
        return Ok(v);
    }

    // Sticky missing: prefer restoring .bak over wiping with defaults (Issue A).
    let bak = bak_path_for(path);
    if bak.is_file() {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        // Restore bak → sticky so subsequent writers see the prior secret-bearing config.
        fs::rename(&bak, path)?;
        let text = fs::read_to_string(path)?;
        let v: Value = serde_json::from_str(&text)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
        ab_log::warn(
            "config",
            "sticky config missing; restored from .bak (prior write may have crashed mid-replace)",
        );
        return Ok(v);
    }

    let defaults = default_config_value();
    write_atomic_locked(path, &defaults)?;
    Ok(defaults)
}

/// Apply a merge-patch document onto sticky config and write atomically.
pub fn apply_patch(patch: &Value) -> io::Result<Value> {
    let path = sticky_path()
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "could not resolve config path"))?;
    apply_patch_at(&path, patch)
}

/// Apply patch at an explicit path (tests / migrate).
///
/// Holds [`CONFIG_IO`] across read → merge → write so concurrent host threads
/// cannot interleave patches (and drop secret field updates).
pub fn apply_patch_at(path: &Path, patch: &Value) -> io::Result<Value> {
    validate_patch(patch)?;
    let _io = CONFIG_IO.lock().unwrap_or_else(|e| e.into_inner());
    // Use locked load so missing sticky restores .bak instead of blank defaults.
    let mut base = load_raw_from_locked(path)?;
    if !base.is_object() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "sticky config root must be a JSON object",
        ));
    }
    merge_patch(&mut base, patch)?;
    write_atomic_locked(path, &base)?;
    Ok(base)
}

/// Load patch JSON from a host-written file path and apply to sticky config.
///
/// `patch_path` should be absolute (hosts write a temp file). Relative paths are
/// accepted by this lower-level helper; the FFI layer enforces absolute paths.
pub fn apply_patch_file(patch_path: &Path) -> io::Result<Value> {
    let text = fs::read_to_string(patch_path)?;
    let patch: Value =
        serde_json::from_str(&text).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    apply_patch(&patch)
}

/// Deep merge `patch` into `base` (RFC 7396-ish for objects; `providers[]` merged by `id`).
///
/// Returns `InvalidData` if either root is not an object (callers must not replace
/// the entire sticky document with arrays/scalars/null).
pub fn merge_patch(base: &mut Value, patch: &Value) -> io::Result<()> {
    match (base, patch) {
        (Value::Object(base_map), Value::Object(patch_map)) => {
            for (k, pv) in patch_map {
                if k == "providers" {
                    if pv.is_null() {
                        return Err(io::Error::new(
                            io::ErrorKind::InvalidData,
                            "config patch must not set `providers` to null",
                        ));
                    }
                    let Some(patch_arr) = pv.as_array() else {
                        return Err(io::Error::new(
                            io::ErrorKind::InvalidData,
                            "config patch `providers` must be a JSON array (merge-by-id)",
                        ));
                    };
                    merge_providers(base_map, patch_arr);
                    continue;
                }
                if k == "hooks" && pv.is_null() {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "config patch must not set `hooks` to null",
                    ));
                }
                if pv.is_null() {
                    // JSON Merge Patch: null deletes (except preserve-denylist above).
                    base_map.remove(k);
                    continue;
                }
                match base_map.get_mut(k) {
                    Some(bv) if bv.is_object() && pv.is_object() => {
                        merge_patch(bv, pv)?;
                    }
                    _ => {
                        base_map.insert(k.clone(), pv.clone());
                    }
                }
            }
            Ok(())
        }
        _ => Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "config merge-patch requires object base and object patch",
        )),
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
            // Provider entries are objects; ignore merge errors for malformed items.
            let _ = merge_patch(existing, patch_item);
        } else {
            arr.push(patch_item.clone());
        }
    }
}

/// Atomic write (temp + bak + rename). Crash-safe: never deletes the only copy first.
///
/// Semantics:
/// 1. Write `path.tmp` and `fsync`.
/// 2. If `path` exists, rename it to `path.bak` (old content still on disk).
/// 3. Rename `path.tmp` → `path`.
/// 4. Best-effort delete `path.bak`.
///
/// Crash between (2) and (3): sticky path missing but `.bak` holds prior config.
/// Crash before (2): original `path` intact; orphan `.tmp` may remain.
pub fn write_atomic(path: &Path, value: &Value) -> io::Result<()> {
    let _io = CONFIG_IO.lock().unwrap_or_else(|e| e.into_inner());
    write_atomic_locked(path, value)
}

/// Write while caller already holds [`CONFIG_IO`].
fn write_atomic_locked(path: &Path, value: &Value) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let text = serde_json::to_string_pretty(value)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    let dir = path.parent().unwrap_or_else(|| Path::new("."));
    let file_name = path
        .file_name()
        .map(|s| s.to_os_string())
        .unwrap_or_else(|| "config.json".into());

    let mut tmp_name = file_name.clone();
    tmp_name.push(".tmp");
    let tmp_path = dir.join(&tmp_name);

    let mut bak_name = file_name.clone();
    bak_name.push(".bak");
    let bak_path = dir.join(&bak_name);

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

    // Preserve previous content via .bak instead of unlink-then-rename (Windows
    // crash between remove and rename used to destroy the only copy).
    if path.exists() {
        // Replace stale bak if present so rename succeeds.
        if bak_path.exists() {
            let _ = fs::remove_file(&bak_path);
        }
        fs::rename(path, &bak_path)?;
    }

    if let Err(e) = fs::rename(&tmp_path, path) {
        // Best-effort restore previous config if we moved it aside.
        if bak_path.exists() && !path.exists() {
            let _ = fs::rename(&bak_path, path);
        }
        let _ = fs::remove_file(&tmp_path);
        return Err(e);
    }

    let _ = fs::remove_file(&bak_path);

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(path, fs::Permissions::from_mode(0o600));
    }

    #[cfg(windows)]
    {
        if let Err(e) = restrict_acl_current_user(path)
            && !WIN_ACL_WARNED.swap(true, Ordering::SeqCst)
        {
            ab_log::warn(
                "config",
                format!(
                    "Windows ACL harden failed (best-effort; config may inherit default DACL): {e}"
                ),
            );
        }
    }

    // Silence unused on non-windows.
    #[cfg(not(windows))]
    {
        let _ = &WIN_ACL_WARNED;
    }

    Ok(())
}

/// Best-effort: restrict DACL so only the current user has access (design: user ACL).
#[cfg(windows)]
fn restrict_acl_current_user(path: &Path) -> io::Result<()> {
    windows_acl::set_user_only_dacl(path)
}

#[cfg(windows)]
mod windows_acl {
    use std::io;
    use std::os::windows::ffi::OsStrExt;
    use std::path::Path;
    use std::ptr;

    use windows_sys::Win32::Foundation::{CloseHandle, ERROR_SUCCESS, HANDLE, LocalFree};
    use windows_sys::Win32::Security::Authorization::{
        EXPLICIT_ACCESS_W, SET_ACCESS, SE_FILE_OBJECT, SetEntriesInAclW, SetNamedSecurityInfoW,
        TRUSTEE_IS_SID, TRUSTEE_IS_USER, TRUSTEE_W,
    };
    use windows_sys::Win32::Security::{
        ACL as WinAcl, DACL_SECURITY_INFORMATION, GetTokenInformation, NO_INHERITANCE,
        PROTECTED_DACL_SECURITY_INFORMATION, TOKEN_QUERY, TOKEN_USER, TokenUser,
    };
    use windows_sys::Win32::Storage::FileSystem::FILE_ALL_ACCESS;
    use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

    /// Grant FILE_ALL_ACCESS only to the process token user; protect DACL inheritance.
    pub fn set_user_only_dacl(path: &Path) -> io::Result<()> {
        // SAFETY: Win32 token/ACL APIs; handles closed, LocalFree on ACL, path is NUL-wide.
        unsafe {
            let mut token: HANDLE = ptr::null_mut();
            if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) == 0 {
                return Err(io::Error::last_os_error());
            }

            // Query token user size then data.
            let mut needed: u32 = 0;
            GetTokenInformation(token, TokenUser, ptr::null_mut(), 0, &mut needed);
            if needed == 0 {
                CloseHandle(token);
                return Err(io::Error::last_os_error());
            }
            let mut buf = vec![0u8; needed as usize];
            if GetTokenInformation(
                token,
                TokenUser,
                buf.as_mut_ptr().cast(),
                needed,
                &mut needed,
            ) == 0
            {
                CloseHandle(token);
                return Err(io::Error::last_os_error());
            }
            CloseHandle(token);

            let token_user = &*(buf.as_ptr() as *const TOKEN_USER);
            let sid = token_user.User.Sid;

            let mut trustee: TRUSTEE_W = std::mem::zeroed();
            trustee.TrusteeForm = TRUSTEE_IS_SID;
            trustee.TrusteeType = TRUSTEE_IS_USER;
            trustee.ptstrName = sid.cast();

            let mut ea: EXPLICIT_ACCESS_W = std::mem::zeroed();
            ea.grfAccessPermissions = FILE_ALL_ACCESS;
            ea.grfAccessMode = SET_ACCESS;
            ea.grfInheritance = NO_INHERITANCE;
            ea.Trustee = trustee;

            let mut new_dacl: *mut WinAcl = ptr::null_mut();
            let rc = SetEntriesInAclW(1, &ea, ptr::null(), &mut new_dacl);
            if rc != ERROR_SUCCESS {
                return Err(io::Error::from_raw_os_error(rc as i32));
            }

            let mut wide: Vec<u16> = path
                .as_os_str()
                .encode_wide()
                .chain(std::iter::once(0))
                .collect();

            let rc = SetNamedSecurityInfoW(
                wide.as_mut_ptr(),
                SE_FILE_OBJECT,
                DACL_SECURITY_INFORMATION | PROTECTED_DACL_SECURITY_INFORMATION,
                ptr::null_mut(),
                ptr::null_mut(),
                new_dacl,
                ptr::null_mut(),
            );
            if !new_dacl.is_null() {
                LocalFree(new_dacl.cast());
            }
            if rc != ERROR_SUCCESS {
                return Err(io::Error::from_raw_os_error(rc as i32));
            }
            Ok(())
        }
    }
}

/// Optional: copy sticky content to default AgentBar path and rebind sticky.
pub fn migrate_to_agentbar() -> io::Result<PathBuf> {
    let dest = default_agentbar_config_path()
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "no home for agentbar path"))?;
    let src =
        sticky_path().ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "no sticky path"))?;
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
    fn resolve_env_codexbar_override() {
        with_temp_home(|_home| {
            let tmp = tempfile::tempdir().unwrap();
            let p = tmp.path().join("legacy.json");
            fs::write(&p, r#"{"version":1}"#).unwrap();
            unsafe {
                env::set_var(ENV_CODEXBAR_CONFIG, &p);
            }
            let r = resolve_path().unwrap();
            assert_eq!(r.sticky, p);
            assert!(!r.will_create);
        });
    }

    #[test]
    fn resolve_xdg_agentbar() {
        with_temp_home(|home| {
            let xdg = home.join("xdg-config");
            let p = xdg.join("agentbar").join("config.json");
            fs::create_dir_all(p.parent().unwrap()).unwrap();
            fs::write(&p, r#"{"version":1,"providers":[]}"#).unwrap();
            // Also put a home agentbar that should lose to XDG when XDG file present
            // (candidates: XDG first).
            unsafe {
                env::set_var("XDG_CONFIG_HOME", &xdg);
            }
            let r = resolve_path().unwrap();
            assert_eq!(r.sticky, p);
        });
    }

    #[test]
    fn resolve_xdg_codexbar() {
        with_temp_home(|home| {
            let xdg = home.join("xdg-config");
            let p = xdg.join("codexbar").join("config.json");
            fs::create_dir_all(p.parent().unwrap()).unwrap();
            fs::write(&p, r#"{"version":1}"#).unwrap();
            unsafe {
                env::set_var("XDG_CONFIG_HOME", &xdg);
            }
            let r = resolve_path().unwrap();
            assert_eq!(r.sticky, p);
        });
    }

    #[test]
    fn resolve_legacy_dot_codexbar() {
        with_temp_home(|home| {
            let p = home.join(".codexbar").join("config.json");
            fs::create_dir_all(p.parent().unwrap()).unwrap();
            fs::write(&p, r#"{"version":1}"#).unwrap();
            let r = resolve_path().unwrap();
            assert_eq!(r.sticky, p);
            assert!(!r.will_create);
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
        merge_patch(&mut base, &patch).unwrap();

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
    fn non_object_patch_rejected_file_unchanged() {
        with_temp_home(|home| {
            let cfg = home.join(".config").join("agentbar").join("config.json");
            fs::create_dir_all(cfg.parent().unwrap()).unwrap();
            let initial = json!({
                "version": 1,
                "providers": [
                    { "id": "codex", "enabled": true, "apiKey": "keep-secret" }
                ]
            });
            write_atomic(&cfg, &initial).unwrap();
            set_sticky_path(cfg.clone());
            let before = fs::read_to_string(&cfg).unwrap();

            for bad in [
                Value::Null,
                json!([]),
                json!("string"),
                json!(42),
                json!(true),
            ] {
                let err = apply_patch_at(&cfg, &bad).unwrap_err();
                assert_eq!(err.kind(), io::ErrorKind::InvalidData);
                let after = fs::read_to_string(&cfg).unwrap();
                assert_eq!(after, before, "file must be unchanged for patch {bad}");
                let v: Value = serde_json::from_str(&after).unwrap();
                assert_eq!(v["providers"][0]["apiKey"], "keep-secret");
            }
        });
    }

    #[test]
    fn providers_null_patch_does_not_wipe_secrets() {
        with_temp_home(|home| {
            let cfg = home.join(".config").join("agentbar").join("config.json");
            fs::create_dir_all(cfg.parent().unwrap()).unwrap();
            let initial = json!({
                "version": 1,
                "hooks": { "x": 1 },
                "providers": [
                    { "id": "codex", "enabled": true, "apiKey": "codex-secret" },
                    { "id": "cursor", "enabled": false, "cookieHeader": "sess=1" }
                ]
            });
            write_atomic(&cfg, &initial).unwrap();
            set_sticky_path(cfg.clone());

            let err = apply_patch_at(&cfg, &json!({"providers": null})).unwrap_err();
            assert_eq!(err.kind(), io::ErrorKind::InvalidData);

            let v: Value = serde_json::from_str(&fs::read_to_string(&cfg).unwrap()).unwrap();
            assert_eq!(v["providers"][0]["apiKey"], "codex-secret");
            assert_eq!(v["providers"][1]["cookieHeader"], "sess=1");
            assert_eq!(v["hooks"]["x"], 1);

            let err = apply_patch_at(&cfg, &json!({"hooks": null})).unwrap_err();
            assert_eq!(err.kind(), io::ErrorKind::InvalidData);
            let v: Value = serde_json::from_str(&fs::read_to_string(&cfg).unwrap()).unwrap();
            assert_eq!(v["hooks"]["x"], 1);
        });
    }

    #[test]
    fn write_atomic_replaces_without_leaving_missing_target() {
        with_temp_home(|home| {
            let cfg = home.join("cfg.json");
            write_atomic(&cfg, &json!({"version": 1, "n": 1})).unwrap();
            assert!(cfg.is_file());
            write_atomic(&cfg, &json!({"version": 1, "n": 2})).unwrap();
            let v: Value = serde_json::from_str(&fs::read_to_string(&cfg).unwrap()).unwrap();
            assert_eq!(v["n"], 2);
            // bak cleaned up after successful replace
            assert!(!home.join("cfg.json.bak").exists());
            assert!(!home.join("cfg.json.tmp").exists());
        });
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

    #[test]
    fn non_array_providers_patch_rejected_file_unchanged() {
        with_temp_home(|home| {
            let cfg = home.join(".config").join("agentbar").join("config.json");
            fs::create_dir_all(cfg.parent().unwrap()).unwrap();
            let initial = json!({
                "version": 1,
                "providers": [
                    { "id": "codex", "enabled": true, "apiKey": "keep-secret" },
                    { "id": "cursor", "enabled": false, "cookieHeader": "sess=xyz" }
                ]
            });
            write_atomic(&cfg, &initial).unwrap();
            set_sticky_path(cfg.clone());
            let before = fs::read_to_string(&cfg).unwrap();

            for bad in [
                json!({"providers": {}}),
                json!({"providers": "x"}),
                json!({"providers": 1}),
                json!({"providers": true}),
            ] {
                let err = apply_patch_at(&cfg, &bad).unwrap_err();
                assert_eq!(err.kind(), io::ErrorKind::InvalidData, "patch={bad}");
                let after = fs::read_to_string(&cfg).unwrap();
                assert_eq!(after, before, "file must be unchanged for patch {bad}");
                let v: Value = serde_json::from_str(&after).unwrap();
                assert_eq!(v["providers"][0]["apiKey"], "keep-secret");
                assert_eq!(v["providers"][1]["cookieHeader"], "sess=xyz");
            }
        });
    }

    #[test]
    fn load_raw_restores_bak_when_sticky_missing() {
        with_temp_home(|home| {
            let cfg = home.join("cfg.json");
            let secret_cfg = json!({
                "version": 1,
                "providers": [
                    { "id": "codex", "enabled": true, "apiKey": "bak-secret" }
                ]
            });
            write_atomic(&cfg, &secret_cfg).unwrap();
            // Simulate crash mid-replace: sticky gone, only .bak remains.
            let bak = bak_path_for(&cfg);
            fs::rename(&cfg, &bak).unwrap();
            assert!(!cfg.exists());
            assert!(bak.is_file());

            let loaded = load_raw_from(&cfg).unwrap();
            assert_eq!(loaded["providers"][0]["apiKey"], "bak-secret");
            assert!(cfg.is_file(), "sticky must be restored from bak");
            let on_disk: Value =
                serde_json::from_str(&fs::read_to_string(&cfg).unwrap()).unwrap();
            assert_eq!(on_disk["providers"][0]["apiKey"], "bak-secret");
            // Must not have written default_config_value() over secrets.
            assert_ne!(on_disk["providers"][0].get("apiKey"), None);
        });
    }

    #[test]
    fn concurrent_load_during_apply_cannot_overwrite_with_defaults() {
        use std::sync::atomic::{AtomicBool, Ordering};
        use std::sync::Arc;
        use std::thread;

        with_temp_home(|home| {
            let cfg = home.join("race-cfg.json");
            let initial = json!({
                "version": 1,
                "providers": [
                    { "id": "codex", "enabled": true, "apiKey": "race-secret" }
                ]
            });
            write_atomic(&cfg, &initial).unwrap();
            set_sticky_path(cfg.clone());

            let stop = Arc::new(AtomicBool::new(false));
            let cfg_load = cfg.clone();
            let stop_load = Arc::clone(&stop);
            let loader = thread::spawn(move || {
                while !stop_load.load(Ordering::SeqCst) {
                    let _ = load_raw_from(&cfg_load);
                    thread::yield_now();
                }
            });

            // Many apply patches while loader races; secrets must survive.
            for i in 0..40 {
                let patch = json!({
                    "providers": [{ "id": "codex", "enabled": i % 2 == 0 }]
                });
                apply_patch_at(&cfg, &patch).unwrap();
            }
            stop.store(true, Ordering::SeqCst);
            loader.join().unwrap();

            let v: Value = serde_json::from_str(&fs::read_to_string(&cfg).unwrap()).unwrap();
            assert_eq!(
                v["providers"][0]["apiKey"], "race-secret",
                "concurrent load must not replace secret-bearing config with defaults"
            );
            // Still an array of providers (not wiped to default-only shape without secret).
            assert!(v["providers"].as_array().unwrap().len() >= 1);
        });
    }
}
