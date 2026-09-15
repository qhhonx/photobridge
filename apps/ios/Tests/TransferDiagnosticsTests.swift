import XCTest
@testable import PhotoBridge

@MainActor final class TransferDiagnosticsTests: XCTestCase {
  func testOldPersistedTasksRemainReadableAndNewCorrelationSurvivesRelaunch() throws {
    let legacy = try JSONDecoder().decode(NativeAttempt.self,
      from: Data(#"{"job_id":7,"generation":2,"receiver_id":"peer"}"#.utf8))
    XCTAssertNil(legacy.requestID)
    var updated = legacy
    updated.requestID = 12345
    updated.submittedAtMS = 1789430000000
    updated.submissionExecution = "background"
    let saved = try JSONDecoder().decode(NativeAttempt.self, from: JSONEncoder().encode(updated))
    XCTAssertEqual(saved.requestID, 12345)
    XCTAssertEqual(saved.submittedAtMS, 1789430000000)
    XCTAssertEqual(saved.submissionExecution, "background")
    XCTAssertEqual(saved.jobID, 7)
  }
  func testDiagnosticSnapshotIsMetadataOnly() {
    let snapshot = TransferDiagnostics.shared.snapshot()
    let allowed: Set<String> = ["observed_at_ms", "low_power", "thermal_state", "battery_percent",
      "charging", "network_available", "network_wifi", "network_cellular", "network_expensive", "network_constrained"]
    XCTAssertTrue(Set(snapshot.keys).isSubset(of: allowed))
    XCTAssertNotNil(snapshot["observed_at_ms"] as? UInt64)
    XCTAssertEqual(TransferDiagnostics.milliseconds(Date(timeIntervalSince1970: 123.456)), 123456)
  }
}
