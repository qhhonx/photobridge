import Foundation
import Photos

/// PhotoKit-specific discovery state. The token and pending native identifiers
/// are atomically persisted together before any originals are exported to Rust.
@MainActor final class PhotoLibraryChanges: NSObject, PHPhotoLibraryChangeObserver {
  struct Pending: Codable {
    let id: String
    var nextAttempt: Date = .distantPast
    var createdAtMS: Int64? = nil
  }
  struct State: Codable {
    var enabled = false
    var receiverID: String?
    var token: Data?
    var pending: [Pending] = []
    var historyUnavailable = false
  }
  private let file: URL
  private(set) var state = State()
  private var scanning = false
  private var revision = 0
  init(root: URL) throws {
    file = root.appendingPathComponent("photo-library-changes.json")
    super.init()
    if FileManager.default.fileExists(atPath: file.path) {
      state = try JSONDecoder().decode(State.self, from: Data(contentsOf: file))
    }
    PHPhotoLibrary.shared().register(self)
  }
  deinit { PHPhotoLibrary.shared().unregisterChangeObserver(self) }
  private func persist(_ next: State) throws {
    #if os(iOS)
      let options: Data.WritingOptions = [
        .atomic, .completeFileProtectionUntilFirstUserAuthentication,
      ]
    #else
      let options: Data.WritingOptions = [.atomic]
    #endif
    try JSONEncoder().encode(next).write(to: file, options: options)
    state = next
  }
  func setEnabled(_ enabled: Bool, receiverID: String?) throws {
    var next = state
    if next.receiverID != receiverID {
      next = State()
      next.receiverID = receiverID
    }
    if enabled {
      // Start from now, including newly imported photos with old capture dates.
      next.token = try NSKeyedArchiver.archivedData(
        withRootObject: PHPhotoLibrary.shared().currentChangeToken, requiringSecureCoding: true)
      next.historyUnavailable = false
    }
    next.enabled = enabled
    try persist(next)
    revision += 1
  }
  func matchReceiver(_ receiverID: String?) throws {
    if state.receiverID != receiverID { try setEnabled(false, receiverID: receiverID) }
  }
  nonisolated func photoLibraryDidChange(_ changeInstance: PHChange) {
    Task { @MainActor in await BackupModel.shared.discoverPhotos() }
  }
  func discoverAndExport(using model: BackupModel) async {
    guard state.enabled, !scanning, !model.paused, !model.importing,
      model.pairing?.receiverID == state.receiverID
    else { return }
    let authorization = PHPhotoLibrary.authorizationStatus(for: .readWrite)
    guard authorization == .authorized || authorization == .limited else {
      model.message = NSLocalizedString("photos_permission_needed", comment: "")
      return
    }
    scanning = true
    defer {
      scanning = false
      model.updateDiscoveryStatus()
    }
    let version = revision
    do {
      if !state.historyUnavailable, let savedToken = state.token {
        do {
          // PhotoKit fetches can block; keep change-history work off the UI thread.
          let changes = try await Task.detached(priority: .utility) {
            () -> (Data?, Set<String>, Set<String>) in
            guard
              let token = try NSKeyedUnarchiver.unarchivedObject(
                ofClass: PHPersistentChangeToken.self, from: savedToken)
            else {
              throw Bridge.Failure(code: "history_unavailable")
            }
            let result = try PHPhotoLibrary.shared().fetchPersistentChanges(since: token)
            var latest: PHPersistentChangeToken?
            var inserted = Set<String>()
            var deleted = Set<String>()
            for change in result {
              let details = try change.changeDetails(for: .asset)
              inserted.formUnion(details.insertedLocalIdentifiers)
              inserted.subtract(details.deletedLocalIdentifiers)
              deleted.formUnion(details.deletedLocalIdentifiers)
              deleted.subtract(details.insertedLocalIdentifiers)
              latest = change.changeToken
            }
            return (
              try latest.map {
                try NSKeyedArchiver.archivedData(withRootObject: $0, requiringSecureCoding: true)
              },
              inserted, deleted
            )
          }.value
          guard version == revision, !Task.isCancelled else { return }
          if let token = changes.0 {
            var next = state
            next.pending.removeAll { changes.2.contains($0.id) }
            let known = Set(next.pending.map(\.id))
            next.pending.append(
              contentsOf: changes.1.subtracting(known).map { Pending(id: $0) })
            next.token = token
            try persist(next)
          }
        } catch {
          guard version == revision else { return }
          var next = state
          next.historyUnavailable = true
          try persist(next)
          model.message = NSLocalizedString("error_history_unavailable", comment: "")
        }
      }
      // Resolve new and legacy pending identifiers once; identifier order has
      // no relationship to capture time. Keep retry deadlines during reordering.
      let missingDates = state.pending.filter { $0.createdAtMS == nil }.map(\.id)
      if !missingDates.isEmpty {
        let dates = await PhotoBackupOrder.captureDates(missingDates)
        guard version == revision, !Task.isCancelled else { return }
        var next = state
        for index in next.pending.indices where next.pending[index].createdAtMS == nil {
          next.pending[index].createdAtMS = dates[next.pending[index].id] ?? Int64.min
        }
        next.pending.sort {
          PhotoBackupOrder.precedes($0.id, $0.createdAtMS ?? Int64.min, $1.id, $1.createdAtMS ?? Int64.min)
        }
        try persist(next)
      }
      // Small bounded batches. A crash before removing an identifier safely
      // re-exports it; the Rust asset identity prevents duplicate transfer.
      let due = state.pending.filter { $0.nextAttempt <= Date() }.prefix(5)
      for pending in due {
        guard version == revision, state.enabled, !model.paused, !Task.isCancelled else { break }
        let completed = await model.importAssets([pending.id], requestAuthorization: false)
        guard version == revision, !Task.isCancelled else { break }
        var next = state
        if completed {
          next.pending.removeAll { $0.id == pending.id }
        } else if let index = next.pending.firstIndex(where: { $0.id == pending.id }) {
          next.pending[index].nextAttempt = Date().addingTimeInterval(300)
        }
        try persist(next)
      }
    } catch { model.message = error.localizedDescription }
  }
}
