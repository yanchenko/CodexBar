// @Observable engine host state — snapshot polling + menu rows.

import AgentBarLogic
import Foundation
import Observation

@Observable
@MainActor
final class Core {
    var snapshot: UsageSnapshot = .empty
    var versionLabel: String = ""
    var engineOk: Bool = false
    private var pollTask: Task<Void, Never>?

    func start() {
        engineOk = Native.engineStart()
        versionLabel = Native.version()
        refreshSnapshot()
        pollTask?.cancel()
        pollTask = Task { [weak self] in
            var lastSeq: UInt64 = 0
            while !Task.isCancelled {
                let json = await Task.detached(priority: .utility) {
                    Native.snapshotWait(sinceSeq: lastSeq, timeoutMs: 2_000)
                }.value
                guard !Task.isCancelled else { break }
                await MainActor.run {
                    guard let self else { return }
                    let snap = UsageSnapshot.from(json: json)
                    if snap.seq != 0 { lastSeq = snap.seq }
                    self.snapshot = snap
                    self.engineOk = Native.engineRunning()
                }
            }
        }
    }

    func stop() {
        pollTask?.cancel()
        pollTask = nil
        _ = Native.engineStop()
        engineOk = false
    }

    func refreshSnapshot() {
        snapshot = UsageSnapshot.from(json: Native.snapshotJson())
    }

    func refreshNow() {
        _ = Native.refreshNow()
        refreshSnapshot()
    }

    func noteMenuOpened() {
        Native.noteMenuOpened()
    }
}
