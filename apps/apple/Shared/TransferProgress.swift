import Foundation

/// Volatile transport progress never changes durable receiver acknowledgements.
struct TransferProgress: Equatable {
  var sent: Int64
  var expected: Int64
  var baseline: UInt64
  var phase: String
  var waitingForNetwork: Bool = false

  func displayedBytes(confirmed: UInt64, total: UInt64) -> UInt64 {
    let sent = UInt64(max(0, sent))
    let estimate: UInt64
    if phase == "bundle", expected > 0 {
      // Multipart framing is included in URLSession's byte count.
      estimate = UInt64(Double(total) * min(1, Double(sent) / Double(expected)))
    } else if phase == "upload" {
      estimate = baseline + min(sent, total > baseline ? total - baseline : 0)
    } else { estimate = confirmed }
    return min(total, max(confirmed, estimate))
  }
}

struct TaskProgressTracker {
  struct Entry {
    var jobID: Int64
    var generation: Int64
    var progress: TransferProgress
  }
  private(set) var entries: [Int: Entry] = [:]
  mutating func update(taskID: Int, attempt: NativeAttempt, sent: Int64, expected: Int64,
    phase: String, baseline: UInt64) {
    let previous = entries[taskID]
    let same = previous?.generation == attempt.generation && previous?.jobID == attempt.jobID
    entries[taskID] = Entry(jobID: attempt.jobID, generation: attempt.generation,
      progress: TransferProgress(sent: max(same ? previous?.progress.sent ?? 0 : 0, sent),
        expected: expected, baseline: same ? previous?.progress.baseline ?? baseline : baseline,
        phase: phase, waitingForNetwork: same && sent <= (previous?.progress.sent ?? 0)
          ? previous?.progress.waitingForNetwork ?? false : false))
  }
  mutating func waiting(taskID: Int) { entries[taskID]?.progress.waitingForNetwork = true }
  mutating func remove(taskID: Int) { entries.removeValue(forKey: taskID) }
  var jobs: [Int64: TransferProgress] {
    var result: [Int64: TransferProgress] = [:]
    for entry in entries.values { result[entry.jobID] = entry.progress }
    return result
  }
}
