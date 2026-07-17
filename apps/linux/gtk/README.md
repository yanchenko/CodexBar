# agentbar (Linux GTK)

GTK4 + StatusNotifierItem (ksni) tray host linking the Rust engine in-process via `ab-core`.

## Build

```bash
# Linux (full GUI)
cd apps/linux/gtk && cargo build --release

# Any OS (stub — no GTK/system libs; used on Windows CI)
cd apps/linux/gtk && cargo check --no-default-features
```

## Architecture

- **GTK main loop** — status window, snapshot list rows
- **ksni thread** — StatusNotifierItem; menu commands over `async-channel` (never touch GTK from DBus)
- **snapshot thread** — `ab_snapshot_wait` push into GTK via channel
- **SNI missing** — tray spawn fails soft; window still works

Mirrors DontSpeak `apps/linux/gtk` patterns.

## Packaging

See `../package.sh`, `../agentbar.desktop`, `../tarball-install.sh`.
