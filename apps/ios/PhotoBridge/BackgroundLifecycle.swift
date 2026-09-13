import BackgroundTasks
import UIKit

@MainActor final class AppDelegate: NSObject, UIApplicationDelegate {
  static let processingIdentifier = "app.photobridge.backup.processing"
  func application(
    _ application: UIApplication,
    didFinishLaunchingWithOptions launchOptions: [UIApplication.LaunchOptionsKey: Any]? = nil
  ) -> Bool {
    BGTaskScheduler.shared.register(forTaskWithIdentifier: Self.processingIdentifier, using: .main)
    { task in
      guard let task = task as? BGProcessingTask else {
        task.setTaskCompleted(success: false)
        return
      }
      Task { @MainActor in
        let work = Task {
          let model = BackupModel.shared
          await model.open()
          await BackgroundTransfer.shared.recordSnapshot("processing_wake")
          await BackgroundTransfer.shared.kick(reconcile: true)
          await model.prepareInBackground()
          await BackgroundTransfer.shared.recordSnapshot("processing_finished")
          model.scheduleBackgroundWork()
          task.setTaskCompleted(success: !Task.isCancelled)
        }
        task.expirationHandler = {
          work.cancel()
          Task { @MainActor in await BackgroundTransfer.shared.recordSnapshot("processing_expired") }
        }
      }
    }
    BackgroundTransfer.shared.connect()
    Task { await BackupModel.shared.open() }
    return true
  }
  func application(
    _ application: UIApplication, handleEventsForBackgroundURLSession identifier: String,
    completionHandler: @escaping () -> Void
  ) {
    guard identifier == BackgroundTransfer.identifier else {
      completionHandler()
      return
    }
    BackgroundTransfer.shared.eventsCompletion = completionHandler
    BackgroundTransfer.shared.connect()
    Task {
      await BackupModel.shared.open()
      await BackgroundTransfer.shared.recordSnapshot("background_session_wake")
    }
  }
}

/// Keep a granted execution window until work drains, is paused or expires.
/// Batches bound memory and preserve cancellation points, not the work per wake.
@MainActor enum BackgroundPreparationLoop {
  enum Step { case progressed, waiting, finished }
  static func run(
    allowed: () -> Bool,
    step: () async -> Step,
    wait: () async throws -> Void = { try await Task.sleep(nanoseconds: 1_000_000_000) }
  ) async {
    while !Task.isCancelled && allowed() {
      let result = await step()
      guard !Task.isCancelled, allowed() else { return }
      switch result {
      case .finished: return
      case .progressed: await Task.yield()
      case .waiting:
        do { try await wait() } catch { return }
      }
    }
  }
}

extension BackupModel {
  func prepareInBackground() async {
    let receiver = pairing?.receiverID
    await BackgroundPreparationLoop.run(allowed: {
      self.ready && !self.paused && self.pairing?.receiverID == receiver
        && receiver != nil && !BackgroundTransfer.shared.waitingForNetwork
        && UIApplication.shared.applicationState != .active
    }, step: {
      if self.importing || self.scanningHistory { return .waiting }
      await self.refresh()
      await self.refreshStorage()
      // Queue only a small runway of prepared originals. The OS owns uploads.
      if self.summary.queued >= 12 {
        await BackgroundTransfer.shared.kick()
        return .waiting
      }
      let before = (self.pendingImports, self.discoveryPending,
        self.historicalImport?.checked ?? 0, self.queueRevision)
      if let storage = self.storage, storage.export_allowance > 0 {
        await self.discoverPhotos()
        // Don't enumerate thousands of additional sources ahead of preparation.
        if self.pendingImports < 200 { await self.scanHistoricalImport() }
        await self.processPendingImports()
      }
      await BackgroundTransfer.shared.kick()
      await self.refresh()
      let after = (self.pendingImports, self.discoveryPending,
        self.historicalImport?.checked ?? 0, self.queueRevision)
      if before != after { return .progressed }
      // Wait for in-flight uploads to free cache; don't spin on delayed retries.
      if self.summary.running > 0 || self.summary.queued > 0 { return .waiting }
      return .finished
    })
  }
}
