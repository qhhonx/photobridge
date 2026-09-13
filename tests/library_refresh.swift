import Foundation

@main struct LibraryRefreshCheck {
  @MainActor static func main() async throws {
    let scheduler = LibraryRefreshScheduler()
    var starts = 0
    var active = 0
    var peak = 0
    // Keep delivering scroll events faster than the refresh delay. A trailing
    // debounce never starts in this interval; a bounded coalescer must progress.
    for _ in 0..<100 {
      scheduler.schedule {
        starts += 1
        active += 1
        peak = max(peak, active)
        try? await Task.sleep(nanoseconds: 120_000_000)
        active -= 1
      }
      try await Task.sleep(nanoseconds: 10_000_000)
    }
    precondition(starts >= 3, "Continuous scrolling must not starve refresh")
    precondition(starts < 20 && peak == 1, "Refreshes must coalesce and serialize")
    scheduler.cancel()
    try await Task.sleep(nanoseconds: 150_000_000)
    let stopped = starts
    try await Task.sleep(nanoseconds: 150_000_000)
    precondition(starts == stopped, "Dismantling must discard pending refresh")

    var cancelledRan = false
    scheduler.schedule { cancelledRan = true }
    scheduler.cancel()
    var latest = 0
    for value in 1...20 { scheduler.schedule { latest = value } }
    try await Task.sleep(nanoseconds: 250_000_000)
    precondition(!cancelledRan && latest == 20, "Only the latest pending snapshot may run")
    scheduler.cancel()
    print("PASS: continuous-scroll progress, serialized refresh, cancellation and latest snapshot")
  }
}
