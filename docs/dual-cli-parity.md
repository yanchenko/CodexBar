# Dual CLI fixture compare notes (Swift vs Rust)

Non-blocking optional gate (PR16). Compare `agentbar` (Rust) usage JSON with the
legacy Swift CLI for MVP providers when both are available (typically macOS).

## Intent

- Keep schema v1 honest across the migration: same `schemaVersion`, omit-null
  policy, banned secret keys.
- Do **not** require bit-identical provider payloads early (timestamps, ordering,
  secondary window presence may differ).

## Fixture-level compare (CI-friendly, no live auth)

```bash
# Rust ab-model fixtures (canonical)
cd rust && cargo test -p ab-model --test fixture_contract

# Host DTO mirrors (when on platform):
# - apps/windows/winui.tests — C# golden fixtures
# - apps/macos AgentBarLogicTests — tray line shape from same JSON
```

Shared fixture paths:

| Fixture | Path |
| --- | --- |
| Success Codex | `rust/crates/ab-model/tests/fixtures/success_codex.json` |
| Cursor requests | `rust/crates/ab-model/tests/fixtures/cursor_requests.json` |
| Failure auth | `rust/crates/ab-model/tests/fixtures/failure_cursor_auth.json` |

## Live dual CLI (macOS, manual / scheduled)

When `Sources/CodexBarCLI` and `rust/crates/ab-cli` both build:

```bash
# Rust
cd rust && cargo build -p ab-cli --release
./target/release/agentbar usage --format json > /tmp/ab-rust.json

# Swift (legacy) — only on macOS with Package.swift products
# swift run codexbar usage --format json > /tmp/ab-swift.json

# Compare shape (example with jq):
jq '{schemaVersion, providerIds: [.providers[].id] | sort}' /tmp/ab-rust.json
# jq '{…}' /tmp/ab-swift.json
```

### Compare checklist

1. `schemaVersion == 1`
2. No `apiKey` / `cookieHeader` / `access_token` substrings in either JSON
3. Enabled MVP provider ids present or structured `errorCode`
4. `usedPercent` in `[0, 100]` when primary window exists
5. Timestamps may differ — do not fail on `updatedAt` drift

## Stub test (Rust)

`ab-model` fixture contract tests already enforce omit-null and banned keys.
A dedicated dual-CLI job is **optional / continue-on-error** until Swift CLI
is wired in multiplatform CI.
