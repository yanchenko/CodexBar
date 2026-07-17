// Menu-bar dropdown: provider text rows from snapshot, refresh, settings, quit.

import AgentBarLogic
import AppKit
import SwiftUI

struct TrayMenu: View {
    @Environment(Core.self) private var core
    @Environment(\.openWindow) private var openWindow

    var body: some View {
        Group {
            if core.snapshot.refreshing {
                Text("Refreshing…")
                    .foregroundStyle(.secondary)
            }

            let lines = core.snapshot.trayLines()
            ForEach(Array(lines.enumerated()), id: \.offset) { _, line in
                Text(line)
            }

            Divider()

            Button("Refresh Now") {
                core.refreshNow()
            }

            Button("Settings…") {
                openWindow(id: "settings")
                NSApp.activate(ignoringOtherApps: true)
            }

            Divider()

            Button("Quit AgentBar") {
                core.stop()
                NSApp.terminate(nil)
            }
        }
        // Once per menu open (not every body re-eval — avoids adaptive-signal spam).
        .onAppear {
            core.noteMenuOpened()
        }
    }
}
