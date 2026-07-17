// Settings placeholder — provider toggles / patch-file ABI land later (WinUI pattern).

import SwiftUI

struct SettingsView: View {
    @Environment(Core.self) private var core

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            Text("AgentBar Settings")
                .font(.title2.bold())
            Text("Version \(core.versionLabel.isEmpty ? "—" : core.versionLabel)")
                .foregroundStyle(.secondary)
            Text(core.engineOk ? "Engine: running" : "Engine: stopped")
                .foregroundStyle(core.engineOk ? .green : .orange)

            Divider()

            Text("Provider configuration uses the same merge-patch path as the Windows host (`ab_config_apply_patch_file`). Full Fluent/SwiftUI settings land in a follow-up.")
                .font(.callout)
                .foregroundStyle(.secondary)
                .fixedSize(horizontal: false, vertical: true)

            Text("Config path:")
                .font(.caption)
            Text(Native.configPath())
                .font(.system(.caption, design: .monospaced))
                .textSelection(.enabled)

            Spacer()

            Text("Note: This is the multiplatform AgentBar host (Rust ab-core). The legacy CodexBar app under Sources/CodexBar is a separate binary.")
                .font(.caption2)
                .foregroundStyle(.tertiary)
                .fixedSize(horizontal: false, vertical: true)
        }
        .padding(20)
        .frame(minWidth: 420, minHeight: 280)
    }
}
