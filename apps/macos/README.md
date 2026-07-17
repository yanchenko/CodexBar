# AgentBar (macOS)

SwiftUI menu-bar host linking `libab_core.a` via `CAgentBar` / `agentbar.h`.

## Dual-app note

| Binary | Location | Runtime |
| --- | --- | --- |
| **AgentBar** (this package) | `apps/macos` | Rust `ab-core` in-process |
| **CodexBar** (legacy) | `Sources/CodexBar` | Swift Core (`CodexBarCore`) |

Both may appear in a developer checkout. Prefer **AgentBar** for multiplatform work; CodexBar remains the upstream regression oracle during migration.

## Build (macOS only)

```bash
./apps/macos/build.sh          # rust release-ffi + swift build -c release
# or:
cd rust && cargo build --profile release-ffi -p ab-core
cd ../apps/macos && swift build -c release
```

Windows CI does **not** require `swift`; this tree is complete sources + scripts only.

## Layout

- `Package.swift` — SwiftPM executable `AgentBar` + `AgentBarLogic` tests
- `Sources/CAgentBar` — `agentbar.h` header (symbols from staticlib)
- `Sources/AgentBar` — MenuBarExtra, tray rows, settings placeholder
- `Sources/AgentBarLogic` — pure snapshot DTOs / format helpers
- `Bundle/Info.plist` — `LSUIElement` accessory metadata for packaging
- `build.sh` / `package.sh` — build + best-effort `.app` zip
