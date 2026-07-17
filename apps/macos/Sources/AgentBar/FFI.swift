// Owned-C-string helpers for the `ab_*` ABI (copy → free in one place).

import CAgentBar
import Foundation

/// Call that returns an owned `char*`: copy to Swift String, `ab_string_free`. Empty on NULL.
func ffiString(_ call: () -> UnsafeMutablePointer<CChar>?) -> String {
    guard let ptr = call() else { return "" }
    defer { ab_string_free(ptr) }
    return String(cString: ptr)
}
