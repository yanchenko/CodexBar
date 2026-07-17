// Pure tray-line formatting helpers (testable without linking ab-core).

import Foundation

public enum TrayFormat {
    /// Format `usedPercent` as an integer percent string, clamped.
    public static func percentLabel(_ used: Double) -> String {
        let v = used.isFinite ? min(100, max(0, used)) : 0
        return "\(Int(v.rounded()))%"
    }

    public static func providerTitle(_ id: String) -> String {
        guard let first = id.first else { return id }
        return String(first).uppercased() + id.dropFirst()
    }
}
