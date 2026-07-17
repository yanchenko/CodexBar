/* agentbar.h — AgentBar stable C ABI (ab_*).
 *
 * Handle-free process-global engine. Owned char* must be freed with ab_string_free.
 * Never returns secrets (apiKey, cookieHeader, tokens). Config mutation is path-only:
 * host writes a JSON patch file, then ab_config_apply_patch_file merges in Rust.
 *
 * Build host-linked artifacts with: cargo build --profile release-ffi -p ab-core
 *
 * Copyright (c) AgentBar contributors. MIT.
 */

#ifndef AGENTBAR_H
#define AGENTBAR_H

#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

/* ── Lifecycle ─────────────────────────────────────────────────────────── */

/* Start engine worker if not running. 1 = running, 0 = failure. Idempotent. */
uint8_t ab_engine_start(void);

/* Stop engine (join workers). 1 if was running. Safe on Exit. */
uint8_t ab_engine_stop(void);

/* Re-read sticky config from disk. 1 if engine running. */
uint8_t ab_engine_reload(void);

/* 1 if engine is running. */
uint8_t ab_engine_running(void);

/* ── Refresh ───────────────────────────────────────────────────────────── */

/* Request immediate refresh (coalesced). 1 if accepted. */
uint8_t ab_refresh_now(void);

/* Fixed interval seconds: 0,60,120,300,900,1800. 0 = manual. */
uint8_t ab_set_refresh_interval_secs(uint32_t secs);

/* Adaptive refresh on (1) / off (0). */
uint8_t ab_set_adaptive_refresh(uint8_t on);

/* Adaptive signal: tray menu / flyout opened. */
void ab_note_menu_opened(void);

/* Host signals JSON: {"lowPower":bool,"thermalSerious":bool}. Empty/{} clears. */
uint8_t ab_set_host_signals_json(const char *json);

/* ── Snapshot (JSON; schema v1 — NEVER secrets) ────────────────────────── */

/* Current snapshot. Owned char*; "{}" if empty. Free with ab_string_free. */
char *ab_snapshot_json(void);

/* Block until seq != since_seq or timeout_ms. Background thread only. */
char *ab_snapshot_wait(uint64_t since_seq, uint32_t timeout_ms);

/* ── Paths (not file contents) ─────────────────────────────────────────── */

/* Sticky write-target config path. Owned char*. */
char *ab_config_path(void);

/* Log directory. Owned char*. */
char *ab_log_dir(void);

/* Engine data directory. Owned char*. */
char *ab_data_dir(void);

/* ── Config mutation — path-only; merge in Rust; returns u8 only ───────── */

/* Absolute path to host-written JSON patch. Merge-patch into sticky config. */
uint8_t ab_config_apply_patch_file(const char *patch_path);

/* ── Catalog / status ──────────────────────────────────────────────────── */

/* Provider catalog JSON (metadata + URLs; no secrets). Owned char*. */
char *ab_providers_catalog_json(void);

/* Product / workspace version. Owned char*. */
char *ab_version(void);

/* Last error JSON {"code","message","providerId?","at"} or "{}". Owned char*. */
char *ab_last_error_json(void);

/* ── Memory ────────────────────────────────────────────────────────────── */

/* Free a string returned by any ab_* API. NULL is a no-op. */
void ab_string_free(char *s);

#ifdef __cplusplus
}
#endif

#endif /* AGENTBAR_H */
