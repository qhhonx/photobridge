import XCTest
import SwiftUI
@testable import PhotoBridge

@MainActor final class NetworkProgressTests: XCTestCase {
  private var pairing: Pairing {
    Pairing(version: 1, receiverID: "test", endpoint: "https://127.0.0.1:8443",
      certificate: "test", token: "test")
  }

  func testCellularAndUnreachableWifiBlockPreparationUntilReceiverReturns() async {
    var probes = 0
    var reachable = false
    let gate = ReceiverAvailability(monitorNetwork: false, probe: { _ in
      probes += 1; return reachable
    })
    gate.networkChanged(allowed: false)
    let cellular = await gate.check(pairing)
    XCTAssertFalse(cellular)
    XCTAssertEqual(probes, 0)
    gate.networkChanged(allowed: true)
    let foreignWifi = await gate.check(pairing)
    XCTAssertFalse(foreignWifi)
    XCTAssertEqual(probes, 1)
    reachable = true
    gate.networkChanged(allowed: true)
    let returned = await gate.check(pairing)
    XCTAssertTrue(returned)
    XCTAssertEqual(probes, 2)
  }

  func testNetworkChangeRejectsAnOldSuccessfulProbe() async {
    var completion: CheckedContinuation<Bool, Never>?
    let gate = ReceiverAvailability(monitorNetwork: false, probe: { _ in
      await withCheckedContinuation { completion = $0 }
    })
    gate.networkChanged(allowed: true)
    let check = Task { await gate.check(pairing) }
    while completion == nil { await Task.yield() }
    gate.networkChanged(allowed: false)
    completion?.resume(returning: true)
    let value = await check.value
    XCTAssertFalse(value)
  }

  func testConcurrentProgressDoesNotOverwriteAnotherJobOrResetOnPolling() {
    var tracker = TaskProgressTracker()
    let a = NativeAttempt(jobID: 1, generation: 1)
    let b = NativeAttempt(jobID: 2, generation: 1)
    tracker.update(taskID: 10, attempt: a, sent: 250, expected: 1000, phase: "bundle", baseline: 0)
    tracker.update(taskID: 20, attempt: b, sent: 700, expected: 1000, phase: "bundle", baseline: 0)
    XCTAssertEqual(tracker.jobs[1]?.displayedBytes(confirmed: 0, total: 800), 200)
    tracker.waiting(taskID: 10)
    tracker.update(taskID: 10, attempt: a, sent: 250, expected: 1000, phase: "bundle", baseline: 0)
    XCTAssertEqual(tracker.jobs[1]?.waitingForNetwork, true)
    tracker.remove(taskID: 20)
    XCTAssertNotNil(tracker.jobs[1])
    tracker.update(taskID: 10, attempt: a, sent: 500, expected: 1000, phase: "bundle", baseline: 0)
    XCTAssertEqual(tracker.jobs[1]?.waitingForNetwork, false)
    XCTAssertEqual(tracker.jobs[1]?.displayedBytes(confirmed: 0, total: 800), 400)
    XCTAssertEqual(tracker.jobs[1]?.displayedBytes(confirmed: 600, total: 800), 600)
  }
}


extension NetworkProgressTests {
  func testStatusIndicatorHeightRemainsStableForLongErrorsAndNetworkWarnings() async {
    let model = BackupModel(root: FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString))
    let host = UIHostingController(rootView: BackupStatusIndicator(model: model))
    host.loadViewIfNeeded()
    let size = CGSize(width: 340, height: 1000)
    let initial = host.sizeThatFits(in: size).height
    XCTAssertEqual(initial, 44, accuracy: 1)
    model.message = String(repeating: "A long error message requiring details. ", count: 30)
    await Task.yield()
    host.view.layoutIfNeeded()
    XCTAssertEqual(host.sizeThatFits(in: size).height, initial, accuracy: 1)
    model.message = nil
    model.waitingForNetwork = true
    model.pendingImports = 12_000
    await Task.yield()
    host.view.layoutIfNeeded()
    XCTAssertEqual(host.sizeThatFits(in: size).height, initial, accuracy: 1)
  }
}


extension NetworkProgressTests {
  func testActivityLogUpdatesWithoutReplacingUnchangedOrReadingRows() async {
    func entry(_ id: Int64) -> ActivityEntry {
      ActivityEntry(id: id, timestamp: id, code: "app_foreground", job_id: nil, bytes: nil, context: nil)
    }
    var response = ActivityLogUpdate(entries: [entry(2), entry(1)], oldest_id: 1, newest_id: 2, reset: false)
    var shouldFail = false
    var requested: [Int64] = []
    let model = ActivityLogModel { cursor in
      requested.append(cursor)
      if shouldFail { throw NSError(domain: "test", code: 1) }
      return response
    }
    var publications = 0
    let subscription = model.$entries.sink { _ in publications += 1 }
    defer { subscription.cancel() }
    await model.refresh { true }
    XCTAssertEqual(model.entries.map(\.id), [2, 1])
    let initialPublications = publications
    response = ActivityLogUpdate(entries: [], oldest_id: 1, newest_id: 2, reset: false)
    await model.refresh { true }
    XCTAssertEqual(publications, initialPublications)
    response = ActivityLogUpdate(entries: [entry(3)], oldest_id: 2, newest_id: 3, reset: false)
    await model.refresh { false }
    XCTAssertEqual(model.entries.map(\.id), [2, 1])
    XCTAssertEqual(model.pendingCount, 1)
    XCTAssertEqual(publications, initialPublications)
    shouldFail = true
    await model.refresh { true }
    XCTAssertTrue(model.failed)
    XCTAssertEqual(model.cursor, 3)
    XCTAssertEqual(model.entries.map(\.id), [2, 1])
    shouldFail = false
    await model.refresh { false }
    XCTAssertFalse(model.failed)
    model.showLatest()
    XCTAssertEqual(model.entries.map(\.id), [3, 2])
    XCTAssertEqual(model.pendingCount, 0)
    XCTAssertEqual(requested, [0, 2, 2, 3, 3])
    response = ActivityLogUpdate(entries: [], oldest_id: nil, newest_id: nil, reset: true)
    await model.refresh { true }
    XCTAssertTrue(model.entries.isEmpty)
    XCTAssertEqual(model.cursor, 0)
  }

  func testActivityLogChecksReadingPositionAfterFetchCompletes() async {
    var completion: CheckedContinuation<ActivityLogUpdate, Never>?
    var following = true
    let model = ActivityLogModel { _ in
      await withCheckedContinuation { completion = $0 }
    }
    let initial = Task { await model.refresh { true } }
    while completion == nil { await Task.yield() }
    completion?.resume(returning: ActivityLogUpdate(entries: [], oldest_id: nil, newest_id: nil, reset: false))
    await initial.value
    completion = nil
    let poll = Task { await model.refresh { following } }
    while completion == nil { await Task.yield() }
    following = false
    let entry = ActivityEntry(id: 1, timestamp: 1, code: "app_foreground", job_id: nil, bytes: nil, context: nil)
    completion?.resume(returning: ActivityLogUpdate(entries: [entry], oldest_id: 1, newest_id: 1, reset: false))
    let followed = await poll.value
    XCTAssertFalse(followed)
    XCTAssertTrue(model.entries.isEmpty)
    XCTAssertEqual(model.pendingCount, 1)
  }
}
