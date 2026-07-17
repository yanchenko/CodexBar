// Thin Swift surface over agentbar.h — in-process engine lifecycle + snapshot JSON.
// Never returns secrets (apiKey / cookieHeader / tokens).

import CAgentBar
import Foundation

enum Native {
    @discardableResult
    static func engineStart() -> Bool { ab_engine_start() != 0 }

    @discardableResult
    static func engineStop() -> Bool { ab_engine_stop() != 0 }

    static func engineRunning() -> Bool { ab_engine_running() != 0 }

    @discardableResult
    static func engineReload() -> Bool { ab_engine_reload() != 0 }

    @discardableResult
    static func refreshNow() -> Bool { ab_refresh_now() != 0 }

    static func noteMenuOpened() { ab_note_menu_opened() }

    static func snapshotJson() -> String { ffiString { ab_snapshot_json() } }

    /// BLOCKING until seq ≠ sinceSeq or timeout. Background thread only.
    static func snapshotWait(sinceSeq: UInt64, timeoutMs: UInt32) -> String {
        ffiString { ab_snapshot_wait(sinceSeq, timeoutMs) }
    }

    static func version() -> String { ffiString { ab_version() } }

    static func lastErrorJson() -> String { ffiString { ab_last_error_json() } }

    static func configPath() -> String { ffiString { ab_config_path() } }

    static func providersCatalogJson() -> String { ffiString { ab_providers_catalog_json() } }
}
