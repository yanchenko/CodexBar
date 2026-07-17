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
- [Windows App SDK](https://learn.microsoft.com/windows/apps/windows-app-sdk/) runtime (unpackaged host bootstrapper loads the installed WAR)
- Rust toolchain (`rustc` 1.97+) with MSVC target (`x86_64-pc-windows-msvc`)

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

(ARM64: `cargo build --profile release-ffi -p ab-core --target aarch64-pc-windows-msvc` →
`rust/target/aarch64-pc-windows-msvc/release-ffi/ab_core.dll`.)

The WinUI csproj copies that DLL next to the app when present
(`CargoFfiOutDir` → `CopyToOutputDirectory`).

## Build / run AgentBar.WinUI

```powershell
# from repo root
cd apps/windows/winui
dotnet build -c Release -p:Platform=x64
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
- **Settings…** — placeholder window (version, config path, snapshot lines).
- **Refresh** — `ab_refresh_now`.
- **Exit** — `ab_engine_stop` + quit.

Until real providers land, enabled providers appear with a structured `not_implemented` error from the engine fake snapshot.

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

Dev builds are **unpackaged** (`WindowsPackageType=None`) with `WindowsAppSDKSelfContained=true` so Windows App Runtime natives (e.g. `Microsoft.ui.xaml.dll`) sit beside the exe. Fully portable zip packaging is a later release PR.
