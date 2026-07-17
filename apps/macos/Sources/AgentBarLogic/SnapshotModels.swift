// Hand-mirrored ab-model UsageSnapshot schema v1 (no secrets). Kept honest by
// rust/crates/ab-model/tests/fixtures + host format tests.

import Foundation

public struct UsageSnapshot: Codable, Sendable, Equatable {
    public var schemaVersion: Int
    public var seq: UInt64
    public var updatedAt: String
    public var refreshing: Bool
    public var providers: [ProviderSnap]

    public static let empty = UsageSnapshot(
        schemaVersion: 1, seq: 0, updatedAt: "", refreshing: false, providers: [])

    public init(
        schemaVersion: Int, seq: UInt64, updatedAt: String, refreshing: Bool,
        providers: [ProviderSnap]
    ) {
        self.schemaVersion = schemaVersion
        self.seq = seq
        self.updatedAt = updatedAt
        self.refreshing = refreshing
        self.providers = providers
    }

    public static func from(json: String) -> UsageSnapshot {
        guard !json.isEmpty, json != "{}",
              let data = json.data(using: .utf8),
              let snap = try? JSONDecoder().decode(UsageSnapshot.self, from: data)
        else { return .empty }
        return snap
    }

    /// One human-readable tray line per provider (text only).
    public func trayLines() -> [String] {
        if providers.isEmpty { return ["No providers enabled"] }
        return providers.map { $0.formatTrayLine() }
    }
}

public struct ProviderSnap: Codable, Sendable, Equatable {
    public var id: String
    public var enabled: Bool
    public var sourceLabel: String?
    public var updatedAt: String?
    public var error: String?
    public var errorCode: String?
    public var primary: RateWindow?
    public var secondary: RateWindow?
    public var creditsRemaining: Double?
    public var accountLabel: String?

    public func formatTrayLine() -> String {
        let name = TrayFormat.providerTitle(id)
        if let err = error, !err.isEmpty {
            let code = errorCode.map { " (\($0))" } ?? ""
            return "\(name): \(err)\(code)"
        }
        if let pct = primary?.usedPercent {
            var s = "\(name): \(TrayFormat.percentLabel(pct))"
            if let label = accountLabel, !label.isEmpty {
                s += " · \(label)"
            }
            return s
        }
        if let credits = creditsRemaining {
            return String(format: "%@: %.2f credits", name, credits)
        }
        return "\(name): —"
    }
}

public struct RateWindow: Codable, Sendable, Equatable {
    public var usedPercent: Double
    public var windowMinutes: Int?
    public var resetsAt: String?
    public var resetDescription: String?
    public var isSyntheticPlaceholder: Bool?
}
