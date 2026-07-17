// @main: accessory MenuBarExtra + settings window. Hosts the engine in-process
// (`ab_engine_start` / `ab_engine_stop`). LSUIElement via Bundle/Info.plist when packaged.

import AppKit
import SwiftUI

@main
struct AgentBarApp: App {
    @NSApplicationDelegateAdaptor(AppDelegate.self) private var appDelegate
    @State private var core = Core()

    var body: some Scene {
        MenuBarExtra {
            TrayMenu()
                .environment(core)
        } label: {
            Label("AgentBar", systemImage: "chart.bar.doc.horizontal")
        }
        .menuBarExtraStyle(.menu)

        Window("AgentBar Settings", id: "settings") {
            SettingsView()
                .environment(core)
        }
        .windowResizability(.contentSize)
        .defaultSize(width: 460, height: 320)
    }
}

final class AppDelegate: NSObject, NSApplicationDelegate {
    func applicationDidFinishLaunching(_ notification: Notification) {
        // Dockless accessory when not already set by Info.plist (dev `swift run`).
        NSApp.setActivationPolicy(.accessory)
    }

    func applicationWillTerminate(_ notification: Notification) {
        // Core.stop is also called from Quit; belt-and-suspenders for crash-free exit.
        _ = Native.engineStop()
    }
}
