import Foundation

/// Coalesce scroll events without starving work during a continuous gesture.
/// At most one refresh runs; events during it request one subsequent refresh.
@MainActor final class LibraryRefreshScheduler {
  private var worker: Task<Void, Never>?
  private var pending: (@MainActor () async -> Void)?
  private var generation = 0

  func schedule(_ action: @escaping @MainActor () async -> Void) {
    pending = action
    guard worker == nil else { return }
    let expected = generation
    worker = Task { [weak self] in
      while !Task.isCancelled {
        do { try await Task.sleep(nanoseconds: 80_000_000) }
        catch { break }
        guard let self, self.generation == expected, let action = self.pending else { break }
        self.pending = nil
        await action()
        guard self.generation == expected else { return }
        if self.pending == nil { break }
      }
      guard let self, self.generation == expected else { return }
      self.worker = nil
    }
  }

  func cancel() {
    generation += 1
    pending = nil
    worker?.cancel()
    worker = nil
  }

  deinit { worker?.cancel() }
}
