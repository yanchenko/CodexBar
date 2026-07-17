// @main: accessory MenuBarExtra + settings window. Hosts the engine in-process
// (`ab_engine_start` / `ab_engine_stop`). LSUIElement via Bundle/Info.plist when packaged.

import AppKit
import SwiftUI

@main
struct AgentBarApp: App {
    @NSApplicationDelegateAdaptor(AppDelegate.self) private var appDelegate

    var body: some Scene {
        MenuBarExtra {
            TrayMenu()
                .environment(appDelegate.core)
        } label: {
            Label("AgentBar", systemImage: "chart.bar.doc.horizontal")
        }
        .menuBarExtraStyle(.menu)

        Window("AgentBar Settings", id: "settings") {
            SettingsView()
                .environment(appDelegate.core)
        }
        .windowResizability(.contentSize)
        .defaultSize(width: 460, height: 320)
    }
}

/// Owns `Core` and engine lifecycle — start on launch (Windows/Linux parity), stop on quit.
@MainActor
final class AppDelegate: NSObject, NSApplicationDelegate {
    let core = Core()

    func applicationDidFinishLaunching(_ notification: Notification) {
        // Dockless accessory when not already set by Info.plist (dev `swift run`).
        NSApp.setActivationPolicy(.accessory)
        // Required: engine must run so tray shows live usage / auth_missing (not empty shell).
        core.start()
    }

    func applicationWillTerminate(_ notification: Notification) {
        core.stop()
    }
}
