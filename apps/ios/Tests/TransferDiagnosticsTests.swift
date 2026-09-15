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
  func testColdSnapshotRefreshesPreparationBacklogEvenWhenStorageBlocksExport() async throws {
    let model = BackupModel.shared
    await model.open()
    // Exercise the already-open durable store with an isolated receiver.
    // Photo-library and Keychain startup are outside this regression.
    _ = try await Bridge.call(["op": "sender_status"])
    let previousReady = model.ready
    model.ready = true
    let previousPaused = model.paused
    await model.setPaused(true)
    let previous = model.pairing
    let previousStorage = model.storage
    let previousPending = model.pendingImports
    let previousReason = model.preparationReason
    let receiver = "preparation-test-" + UUID().uuidString
    model.pairing = Pairing(version: 1, receiverID: receiver,
      endpoint: "https://127.0.0.1:1", certificate: "fixture", token: "fixture")
    defer {
      model.pairing = previous; model.storage = previousStorage
      model.pendingImports = previousPending; model.preparationReason = previousReason
      model.ready = previousReady
    }
    _ = try await Bridge.call(["op": "schedule_sources", "receiver_id": receiver,
      "sources": ["fixture-a", "fixture-b", "fixture-c"]])
    let queued = try await Bridge.call(["op": "pending_sources", "receiver_id": receiver])
    XCTAssertEqual((try JSONSerialization.jsonObject(with: queued) as? [String: Any])?["count"] as? Int, 3)
    model.pendingImports = 0
    let data = Data(#"{"settings":{"cache_budget_bytes":5368709120,"receiver_budget_bytes":6442450944,"min_free_bytes":5368709120,"auto_reclaim":true,"log_days":14,"log_limit":5000},"used_bytes":1000,"free_bytes":1000000,"export_allowance":0,"reason":"local_free_space"}"#.utf8)
    model.storage = try JSONDecoder().decode(StorageSnapshot.self, from: data)
    await BackgroundTransfer.shared.recordSnapshot("processing_wake")
    XCTAssertEqual(model.pendingImports, 3)
    await model.updatePreparationStorageState()
    XCTAssertEqual(model.preparationReason, "local_free_space")
    let log = try await Bridge.call(["op": "activity_log", "receiver": false])
    let entries = try XCTUnwrap(JSONSerialization.jsonObject(with: log) as? [[String: Any]])
    let entry = try XCTUnwrap(entries.first { $0["code"] as? String == "preparation_storage_blocked" })
    let context = try XCTUnwrap(entry["context"] as? [String: Any])
    XCTAssertEqual(context["pending_imports"] as? Int, 3)
    XCTAssertEqual(context["export_allowance"] as? Int, 0)
    XCTAssertEqual(context["free_bytes"] as? Int, 1000000)
    for source in ["fixture-a", "fixture-b", "fixture-c"] {
      _ = try await Bridge.call(["op": "source_result", "receiver_id": receiver,
        "source": source, "complete": true])
    }
    await model.setPaused(previousPaused)
  }
}
