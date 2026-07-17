# AgentBar security re-audit — multiplatform Rust core + WinUI host

| Field | Value |
| --- | --- |
| **Scope** | `feat/agentbar-multiplatform` — `rust/` crates + `apps/windows/winui` |
| **Date** | 2026-07-17 (re-audit after AB-001..007 fixes) |
| **Branch / tip** | `feat/agentbar-multiplatform` @ `d6b2302f` (+ prior ACL host work in `52e6957b`) |
| **Prior audit** | Same path; AB-001..007 marked fixed; AB-008..010 open |
| **Method** | Code verification only (no live provider probes / Keychain UI). Re-read fix sites; confirmed prior residual findings still present. |

---

## Verdict

**AB-001 through AB-007 are fixed** (verified in tree).  
**Three residual items remain open** (low / informational) — same as prior audit: AB-008, AB-009, AB-010.  
**No new findings.** Not a clean approve until residual low items are accepted as deferred or closed.

---

## Closed: AB-001..007 (re-verified)

| ID | Severity | Verification |
| --- | --- | --- |
| **AB-001** | high | `ab-cli` `set-provider`: bare `--api-key <value>` / `--cookie-header <value>` exit **2** with refuse message. Allowed: `--api-key -` / `--cookie-header -` (stdin), `--api-key-env` / `--cookie-header-env`, fallback `AGENTBAR_API_KEY` / `AGENTBAR_COOKIE_HEADER`. Help documents rejection. (`rust/crates/ab-cli/src/main.rs`) |
| **AB-002** | high | `write_json_atomic_held` applies Unix `0600` + Windows `ab_config::restrict_file_acl_current_user` on `.tmp`, `.bak`, and final path (warn on ACL failure). (`ab-provider/src/common.rs` + exported ACL helper) |
| **AB-003** | medium | WinUI `ApplyConfigPatchJson`: harden `%TEMP%\AgentBar` dir ACL, write patch, `RestrictFileAclCurrentUser`, apply absolute path, delete in `finally`. (`apps/windows/winui/Native.cs`) |
| **AB-004** | medium | OpenRouter `base_url` ignores config `apiBase`/`baseUrl`; production hard-codes `https://openrouter.ai/api/v1`; tests inject via `EndpointOverrides.openrouter_base` only. (`ab-provider/src/openrouter.rs`) |
| **AB-005** | medium | Sibling `auth.json.lock` exclusive `LockFileEx` (Windows) / `flock` (Unix), ~40×25ms retry; in-process `AUTH_IO` retained. Comment notes third-party tools ignoring `.lock` can still race. (`ab-provider/src/common.rs`) |
| **AB-006** | medium | `SECRET_FIELD_KEYS` denylist; `validate_patch` + `merge_patch` / `merge_providers` reject null secrets; test `secret_field_null_patch_rejected`. (`ab-config/src/lib.rs`) |
| **AB-007** | low | Config `write_atomic_locked`: Windows ACL on `.tmp` after create, on `.bak` after rename-aside, and on final path. (`ab-config/src/lib.rs`) |

Checklist alignment: `docs/security-surface.md` documents CLI argv ban, ACL on config+auth, OS locks, OpenRouter hard-coded host, null-secret reject. Review gates L1/L3 still unchecked (maps to AB-008).

---

## Open findings (remaining)

### AB-008 — Log redactor and snapshot error scrub miss bare token shapes; secret structs implement `Debug`

| | |
| --- | --- |
| **Severity** | low |
| **Location** | `rust/crates/ab-log/src/lib.rs` (`redact`, `SECRET_KEYS`); `ab-provider/src/lib.rs` (`looks_like_secret`); `CodexCredentials` / `ClaudeCredentials` `#[derive(Debug)]` |
| **Status** | **open** (unchanged) |
| **Evidence** | Redaction remains **key-oriented** (`apiKey=…`, JSON `"apiKey":"…"`, `Bearer …`). No pattern redaction for bare prefixes (`sk-or-…`, `sk-ant-…`, raw `eyJ…` JWTs without Bearer/key context). `looks_like_secret` only checks substrings `sk-`, `Bearer `, `cookieHeader`, `access_token`, `refresh_token`, `WorkosCursorSessionToken=` for snapshot **error** scrub — not used by the logger. `CodexCredentials` / `ClaudeCredentials` still `#[derive(Debug)]` with live `access_token` / `refresh_token` fields (`codex.rs`, `claude.rs`). `docs/security-surface.md` L1/L3 still open. |
| **Impact** | Future log lines that interpolate raw tokens without a secret key name can reach stderr. Accidental `{:?}` on credential structs would dump OAuth material. |
| **Remediation** | Pattern-based redaction for common prefixes; custom `Debug` that redacts token fields; broaden `looks_like_secret` in parallel. |

---

### AB-009 — `ab-proc` inherits full parent environment into child processes

| | |
| --- | --- |
| **Severity** | low |
| **Location** | `rust/crates/ab-proc/src/lib.rs` (`run_with_env`) |
| **Status** | **open** (unchanged) |
| **Evidence** | `Command::new` + optional `cmd.env(k,v)` **adds** vars; no `env_clear` / allowlist. Children inherit AgentBar’s full environment (`OPENAI_API_KEY`, `ANTHROPIC_API_KEY`, etc.). Production spawns remain argv-only version probes (`codex`/`claude` + `--version`) via resolved absolute path when possible, with bare-name fallback. |
| **Impact** | Expands blast radius if a malicious/compromised CLI binary is earlier on PATH than the intended install. Version probes only today. |
| **Remediation** | Minimal env allowlist for probes (`PATH`, `HOME`/`USERPROFILE`, `SystemRoot`, locale). Prefer fail-closed if absolute path unresolved. |

---

### AB-010 — CLI `config patch --file` does not require absolute path (FFI does)

| | |
| --- | --- |
| **Severity** | informational |
| **Location** | `ab-cli` `config patch` → `ab_config::apply_patch_file`; FFI `ab_config_apply_patch_file` |
| **Status** | **open** (unchanged) |
| **Evidence** | CLI passes path straight to `ab_config::apply_patch_file` with no absolute check (`ab-cli/src/main.rs`). Helper docs: relative paths accepted at lower level; **FFI** rejects non-absolute (`ab-core/src/ffi.rs` `relative_patch_path_rejected` test). Sticky write target remains fixed. |
| **Impact** | Low; cwd-relative scripts can apply the wrong file by mistake. Not a sticky-path traversal. |
| **Remediation** | Require absolute paths in CLI or canonicalize + echo resolved path before apply. |

---

## Controls that held (re-confirmed)

| Control | Evidence |
| --- | --- |
| **No secrets over C ABI / snapshot** | Path-only config mutation; catalog/snapshot omit secret keys; engine/host tests |
| **Config providers/hooks wholesale null wipe blocked** | `validate_patch` + tests |
| **Per-field secret null wipe blocked** | AB-006 tests |
| **Process spawn argv-only (no shell)** | `ab_proc::run` / `Command::new(program).args` |
| **Cursor manual-cookie only** | Checklist + design |
| **Claude Keychain avoided on Windows/Linux** | File / apiKey / CLI presence |
| **OpenRouter no config SSRF base** | AB-004 hard-coded host |
| **Relative patch path rejected at ABI** | FFI absolute-path contract |
| **Auth + config Windows ACL + OS locks** | AB-002, AB-005, AB-007 |

---

## Severity summary

| Severity | Open | Closed this cycle |
| --- | --- | --- |
| critical | 0 | — |
| high | 0 | AB-001, AB-002 |
| medium | 0 | AB-003, AB-004, AB-005, AB-006 |
| low | **2** (AB-008, AB-009) | AB-007 |
| informational | **1** (AB-010) | — |

---

## Recommendation

Ship multiplatform core with **documented deferral** of AB-008..010 (all low/info; no remote exploit path identified), or close L1/L3 + env allowlist before wider release if defense-in-depth for local malware/log leaks is required.

**Approve with residual low findings** — not a full clean approve.
