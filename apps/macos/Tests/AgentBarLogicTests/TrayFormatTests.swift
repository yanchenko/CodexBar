import XCTest
@testable import AgentBarLogic

final class TrayFormatTests: XCTestCase {
    func test_percentLabel_clamps() {
        XCTAssertEqual(TrayFormat.percentLabel(42.4), "42%")
        XCTAssertEqual(TrayFormat.percentLabel(-5), "0%")
        XCTAssertEqual(TrayFormat.percentLabel(150), "100%")
    }

    func test_providerTitle() {
        XCTAssertEqual(TrayFormat.providerTitle("codex"), "Codex")
    }

    func test_snapshot_tray_lines_from_json() {
        let json = """
        {"schemaVersion":1,"seq":1,"updatedAt":"t","refreshing":false,"providers":[
          {"id":"codex","enabled":true,"updatedAt":"t","primary":{"usedPercent":12.0}}
        ]}
        """
        let snap = UsageSnapshot.from(json: json)
        let lines = snap.trayLines()
        XCTAssertEqual(lines.count, 1)
        XCTAssertTrue(lines[0].contains("Codex"))
        XCTAssertTrue(lines[0].contains("12%"))
    }
}
