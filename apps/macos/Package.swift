// swift-tools-version: 6.0
//
// AgentBar — macOS SwiftUI menu-bar host (LSUIElement) over the Rust C ABI
// (`CAgentBar/include/agentbar.h` → libab_core.a).
//
// Dual-app note: legacy `Sources/CodexBar` remains the upstream Swift menu-bar
// product during migration. This package is the multiplatform AgentBar host that
// links ab-core (Rust). Do not mix the two executables' state or icons.
//
// Floor: macOS 14 (MenuBarExtra). Link against libab_core.a from build.sh
// (`../../rust/target/release-ffi`); plain `swift build` without that staticlib
// fails at link. On Windows CI this tree is sources-only (swift not required).

import PackageDescription

let package = Package(
    name: "AgentBar",
    platforms: [
        .macOS(.v14)
    ],
    targets: [
        // Header-only C target → `import CAgentBar`; symbols from Rust staticlib.
        .target(
            name: "CAgentBar",
            publicHeadersPath: "include"
        ),
        // Pure helpers only — executable force-loads the staticlib and can't host XCTest.
        .target(
            name: "AgentBarLogic"
        ),
        .testTarget(
            name: "AgentBarLogicTests",
            dependencies: ["AgentBarLogic"]
        ),
        .executableTarget(
            name: "AgentBar",
            dependencies: ["CAgentBar", "AgentBarLogic"],
            linkerSettings: [
                // `-force_load` retains every archive member so `-dead_strip` cannot drop
                // ab_* symbols that Swift only references across the C ABI boundary.
                .unsafeFlags([
                    "-L", "../../rust/target/release-ffi",
                    "-Xlinker", "-force_load",
                    "-Xlinker", "../../rust/target/release-ffi/libab_core.a",
                ]),
                .linkedFramework("AppKit"),
                .linkedFramework("Foundation"),
                .linkedFramework("CoreFoundation"),
                .linkedLibrary("objc"),
                // rustls / ring staticlib may pull system libs on some toolchains.
                .linkedLibrary("resolv"),
            ]
        ),
    ]
)
