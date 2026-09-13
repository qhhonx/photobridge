import XCTest
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
