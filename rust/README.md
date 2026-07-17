# AgentBar Rust core

Shared engine for **AgentBar** (multiplatform fork of CodexBar): provider probes, config I/O, refresh loop, and snapshot JSON.

## Workspace

| Crate | Role |
| --- | --- |
| `ab-model` | `UsageSnapshot` / `RateWindow` wire types (schema v1) |
| `ab-config` | Sticky config paths + merge-patch preserve |
| `ab-log` | Redacting structured logger |
| `ab-proc` | Argv-only process runner (timeouts, output caps) |
| `ab-http` | Blocking HTTPS (rustls) |
| `ab-provider` | Provider registry + strategies |
| `ab-engine` | Runtime: lifecycle, refresh, snapshot store |
| `ab-core` | C ABI (`agentbar.h` / `ab_*`) — cdylib + staticlib |
| `ab-cli` | `agentbar` CLI binary |

Design: [`docs/design/multiplatform-rust-core.md`](../docs/design/multiplatform-rust-core.md).

## Build

```bash
cd rust
cargo build -p ab-core
cargo test --workspace
cargo build --profile release-ffi -p ab-core
```

FFI host builds **must** use `--profile release-ffi` (panic=unwind + symbols kept).

Config: primary `~/.config/agentbar/config.json` with CodexBar path read-compat.
