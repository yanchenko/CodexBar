# AgentBar Windows

Native Windows host for the AgentBar multiplatform rewrite.

| Path | Role |
| --- | --- |
| `winui/` | **AgentBar.WinUI** — WinUI 3 tray host (P/Invoke `ab_core.dll`) |
| `winui.tests/` | xunit DTO / snapshot contract tests (no engine DLL required) |

Design: [`docs/design/multiplatform-rust-core.md`](../../docs/design/multiplatform-rust-core.md).

## Prerequisites

- Windows 10 1809+ / Windows 11
- [.NET 10 SDK](https://dotnet.microsoft.com/download)
- Rust toolchain (`rustc` 1.97+) with MSVC target (`x86_64-pc-windows-msvc`; ARM64: `aarch64-pc-windows-msvc`)
- **No installed Windows App Runtime required** for the default self-contained layout — WAR natives ship beside the exe (`WindowsAppSDKSelfContained=true`)

## Build the Rust engine (required)

The WinUI host loads `ab_core.dll` from the cargo **release-ffi** profile (`panic=unwind` so the C ABI `catch_unwind` fence works):

```powershell
cd rust
cargo build --profile release-ffi -p ab-core
```

Output:

```
rust/target/release-ffi/ab_core.dll
```

ARM64:

```powershell
cargo build --profile release-ffi -p ab-core --target aarch64-pc-windows-msvc
# → rust/target/aarch64-pc-windows-msvc/release-ffi/ab_core.dll
```

The WinUI csproj copies that DLL next to the app when present
(`CargoFfiOutDir` → `CopyToOutputDirectory`). If the DLL is missing, MSBuild emits a **Warning** with the cargo hint (runtime MessageBox is the last line of defense).

## Build / run AgentBar.WinUI

```powershell
# from repo root — x64 (default RID win-x64)
cd apps/windows/winui
dotnet build -c Release -p:Platform=x64
```

ARM64 (must pass RID so `CargoFfiOutDir` points at the aarch64 release-ffi artifact):

```powershell
dotnet build -c Release -p:Platform=ARM64 -r win-arm64
```

Run (after a successful build):

```powershell
dotnet run -c Release -p:Platform=x64 --no-build
# or launch the output exe:
# bin\x64\Release\net10.0-windows10.0.19041.0\win-x64\agentbar-winui.exe
```

Flags:

| Flag | Effect |
| --- | --- |
| `--hidden` / `--tray` | Start tray-only (no Settings window) |

Single-instance: a second launch activates the running instance (named mutex `AgentBar.WinUI.SingleInstance`).

### What the tray shows

- **Text-only** `MenuFlyout` provider rows from `ab_snapshot_json` (no ProgressBars in the flyout).
- **Settings…** — Fluent NavigationView: Dashboard ProgressBars, Providers (toggle/secret paste via temp JSON patch + `ab_config_apply_patch_file`), General (refresh cadence), Advanced (paths).
- **Refresh** — `ab_refresh_now`.
- **Exit** — `ab_engine_stop` + quit.

## Portable zip

```powershell
pwsh apps/windows/installer/build-portable.ps1          # x64 self-contained zip
pwsh apps/windows/installer/build-portable.ps1 -Arch arm64
pwsh apps/windows/installer/build-portable.ps1 -SkipPublish  # re-zip existing stage
```

Output: `apps/windows/installer/Output/agentbar-<ver>-windows-x86_64.zip`

Enabled MVP providers (Codex / Claude / Cursor) are probed by the engine: real usage when credentials exist, otherwise a structured `auth_missing` (or related) error — never a crash.

## Tests (snapshot contract)

Rust fixtures live under `rust/crates/ab-model/tests/fixtures/` and are shared with C# tests.

```powershell
# Rust contract + unit tests
cd rust
cargo test -p ab-model

# C# DTO tests (parse same fixtures; no ab_core.dll)
cd apps/windows/winui.tests
dotnet test -c Release -p:Platform=x64
```

## Troubleshooting

| Symptom | Fix |
| --- | --- |
| Message box: `ab_core.dll was not found` | Build release-ffi (above), rebuild WinUI |
| App starts then exits immediately | Another instance is already running (single-instance mutex) |
| Empty tray / “No providers enabled” | Create `%APPDATA%\AgentBar\config.json` or CodexBar-compat config with an enabled provider |

## Package notes

Dev builds are **unpackaged** (`WindowsPackageType=None`) and **self-contained** (`WindowsAppSDKSelfContained=true`): Windows App Runtime natives (e.g. `Microsoft.ui.xaml.dll`) sit beside the exe. An installed machine WAR is **not** required for this layout. Portable zip packaging is supported via `apps/windows/installer/build-portable.ps1` (self-contained stage + `agentbar-*-windows-*.zip`).
