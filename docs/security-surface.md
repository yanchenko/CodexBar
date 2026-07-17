# AgentBar security surface checklist

Living checklist for the multiplatform Rust core + native hosts. Aligns with
`docs/design/multiplatform-rust-core.md` threat model.

**Last updated:** 2026-07-17 (PR14)

## Secrets & credentials

| Check | Status | Notes |
| --- | --- | --- |
| Snapshots / FFI never carry `apiKey`, `cookieHeader`, tokens | Required | `ProviderSnapshot` / `UsageSnapshot` schema; host DTO tests; engine tests |
| Config mutation is path-only over ABI (`ab_config_apply_patch_file`) | Required | Hosts write temp patch file; Rust merges |
| Logs redact secret-looking strings | Required | `ab_log` redactor |
| Codex `auth.json` write-back preserves unknowns + atomic | Required | Process lock + temp/rename; nested `tokens` merge |
| Claude credentials write-back preserves oauth unknowns + atomic | Required | Merge into `claudeAiOauth`; no wholesale replace |
| Cursor is **manual cookie only** (no Chromium DPAPI) | Required | `cookieSource=manual` gate |
| Claude: no Keychain path on Windows/Linux | Required | File → API key → CLI only |
| Codex: ambient `CODEX_HOME` / `~/.codex` only | Required | No multi-home in MVP |

## Process & network

| Check | Status | Notes |
| --- | --- | --- |
| Process runner is argv-only (no shell) | Required | `ab_proc::run` |
| Prefer absolute CLI paths when resolvable | Hardening | `resolve_cli_program` for `codex` / `claude` |
| HTTPS client uses rustls (no native-tls default) | Required | `ab_http` / attohttpc |
| No user-controlled program string from config | Required | Fixed program names |
| Provider endpoint overrides / custom hosts | Out of MVP | SSRF surface deferred |

## Host / packaging

| Check | Status | Notes |
| --- | --- | --- |
| Portable Windows zip self-contained publish path | Win packaging | `apps/windows/installer` |
| macOS LSUIElement host links staticlib | Host | `apps/macos` |
| Linux ksni tray fails soft without SNI | Host | Window still works |
| Config file perms `0600` on Unix after write | Required | ab-config + auth atomic writes |
| Simultaneous GUI+CLI auth refresh | Documented limit | In-process lock; cross-process flock best-effort later |

## Review gates (provider)

- [x] M1 Claude write-back preserves oauth fields
- [x] M2 Atomic + locked auth writes
- [x] M3 Absolute CLI path preference
- [x] M4 Codex nested `tokens` unknowns preserved
- [ ] L1 Broaden `looks_like_secret` if body hints ever ship
- [ ] L3 Custom `Debug` on secret-bearing structs

## How to re-verify

```bash
cd rust && cargo test --workspace -- --test-threads=1
# Provider-focused:
cargo test -p ab-provider -- --test-threads=1
```

Banned secret keys in fixture JSON: `apiKey`, `cookieHeader`, raw bearer tokens.
