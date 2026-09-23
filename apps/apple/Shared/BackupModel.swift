import Combine
import Foundation
import Photos
import Security

#if os(iOS)
  import BackgroundTasks
  import UIKit
#endif

struct Pairing: Codable {
  let version: Int
  let receiverID: String
  let endpoint: String
  let certificate: String
  let token: String
  var certificateName: String? = nil
  enum CodingKeys: String, CodingKey {
    case version, endpoint, certificate, token
    case receiverID = "receiver_id"
    case certificateName = "certificate_name"
  }
}
struct BackupJob: Decodable, Identifiable, Equatable {
  struct Asset: Decodable, Equatable {
    struct Resource: Decodable, Equatable {
      let filename: String
      let size: UInt64
    }
    var metadata: [String: String]? = nil
    let kind: String
    let source_id: String
    let revision: String
    let resources: [Resource]
  }
  let id: Int64
  let asset: Asset
  let state: String
  let confirmedBytes: UInt64
  let errorCode: String?
  var attempts: Int? = nil
  var nextAttemptAt: Int64? = nil
  var stateChangedAt: Int64? = nil
  var sortValue: Int64? = nil
  var totalBytes: UInt64 { asset.resources.reduce(0) { $0 + $1.size } }
  enum CodingKeys: String, CodingKey {
    case id, asset, state, attempts
    case nextAttemptAt = "next_attempt_at"
    case stateChangedAt = "state_changed_at_ms"
    case sortValue = "sort_value"
    case confirmedBytes = "confirmed_bytes"
    case errorCode = "error_code"
  }
}

enum Bridge {
  struct Failure: Error, LocalizedError {
    let code: String
    var errorDescription: String? { NSLocalizedString("error_" + code, comment: "") }
  }
  // The native worker owns no queue logic. Rust schedules and persists tasks.
  static func call(_ command: [String: Any]) async throws -> Data {
    let input = try JSONSerialization.data(withJSONObject: command)
    let text = String(decoding: input, as: UTF8.self)
    return try await withCheckedThrowingContinuation { continuation in
      DispatchQueue.global(qos: .utility).async {
        do {
          let pointer = text.withCString { photobridge_call($0) }
          guard let pointer else { throw Failure(code: "internal") }
          let output = Data(String(cString: pointer).utf8)
          photobridge_free(pointer)
          let envelope = try JSONSerialization.jsonObject(with: output) as! [String: Any]
          guard envelope["ok"] as? Bool == true else {
            throw Failure(code: envelope["error"] as? String ?? "internal")
          }
          continuation.resume(
            returning: try JSONSerialization.data(
              withJSONObject: envelope["value"] ?? NSNull(), options: [.fragmentsAllowed]))
        } catch { continuation.resume(throwing: error) }
      }
    }
  }
}

@MainActor final class BackupModel: ObservableObject {
  static let shared = BackupModel()
  @Published var historicalImport: HistoricalImportStatus?
  @Published var historyError: String?
  @Published var changingHistory = false
  var scanningHistory = false
  var historyControlRevision = 0
  var historicalCursor: HistoricalScanCursor?
  @Published var autoBackup = false
  @Published var discoveryPending = 0
  @Published var historyUnavailable = false
  @Published var waitingForNetwork = false
  @Published var concurrentUploads = 4
  @Published var savingConcurrency = false
  @Published var transferProgress: [Int64: TransferProgress] = [:]
  @Published var receiverUnavailable = false
  private var activeExport: BoundedExportWriter?
  private lazy var availability = ReceiverAvailability(changed: { [weak self] in
    self?.receiverUnavailable = true
    self?.activeExport?.cancel()
  })
  func canPrepareForReceiver() async -> Bool {
    guard !paused, let target = pairing else { return false }
    let available = await availability.check(target)
    guard pairing?.receiverID == target.receiverID, pairing?.endpoint == target.endpoint else { return false }
    let recovered = receiverUnavailable && available
    receiverUnavailable = !available
    if recovered {
      _ = try? await Bridge.call(["op": "recover_connection", "receiver_id": target.receiverID])
    }
    if !available { activeExport?.cancel() }
    return available && !paused
  }
  #if os(iOS)
    func recoverBackgroundReceiverRoute() async {
      guard !paused, let saved = pairing, !(await canPrepareForReceiver()) else { return }
      let discovery = ReceiverDiscovery(model: self)
      if await discovery.recoverInBackground(saved) {
        _ = await canPrepareForReceiver()
      }
    }
  #endif
  @Published var importingSourceID: String?
  @Published var exportProgress: Double?
  @Published var storage: StorageSnapshot?
  @Published var storageError: String?
  @Published var savingStorage = false
  @Published var reclaimingCache = false
  @Published var cacheReclaimResult: String?
  @Published var pendingImports = 0
  @Published var preparationReason: String?
  private var lastStorageRefresh = Date.distantPast
  @Published var hasMoreJobs = false
  private var jobsLimit = 200
  @Published var queueRevision: Int64 = -1
  @Published var summary = SenderSummary()
  private var refreshing = false
  private var discovery: PhotoLibraryChanges?
  private lazy var receiverDiscovery = ReceiverDiscovery(model: self)
  private var opening: Task<Void, Never>?
  @Published var pairing: Pairing?
  @Published var pairingError: String?
  @Published var pairingInProgress = false
  @Published var deviceSnapshot: DeviceSnapshot?
  @Published var deviceError: String?
  @Published var savingDevice = false
  var exchangingDevice = false
  var lastDeviceExchange = Date.distantPast
  @Published var jobs: [BackupJob] = []
  @Published var message: String?
  @Published var paused = true
  @Published var importing = false
  @Published var ready = false
  private var opened = false
  private var queueOpened = false
  private var refreshTask: Task<Void, Never>?
  private var lastScheduleError: Int?
  let root: URL
  init(root: URL? = nil) {
    self.root = root ?? FileManager.default.urls(
      for: .applicationSupportDirectory, in: .userDomainMask)[0].appendingPathComponent(
        "PhotoBridge", isDirectory: true)
  }

  func open() async {
    if ready { return }
    if let opening {
      await opening.value
      return
    }
    let work = Task { await initialize() }
    opening = work
    await work.value
    opening = nil
  }
  private func initialize() async {
    guard !opened else { return }
    opened = true
    _ = availability // Observe connectivity before asynchronous startup completes.
    do {
      try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
      #if os(iOS)
        try FileManager.default.setAttributes(
          [.protectionKey: FileProtectionType.completeUntilFirstUserAuthentication],
          ofItemAtPath: root.path)
      #endif
      var privateRoot = root
      var values = URLResourceValues()
      values.isExcludedFromBackup = true
      try privateRoot.setResourceValues(values)
      if !queueOpened {
        await refreshDeviceStatus()
        _ = try await Bridge.call([
          "op": "open_sender", "root": root.appendingPathComponent("queue").path,
        ])
        queueOpened = true
      }
      message = NSLocalizedString("pairing_unlock_wait", comment: "")
      if let data = try await Keychain.readAsync() {
        pairing = try JSONDecoder().decode(Pairing.self, from: data)
      }
      message = nil
      // Opening the UI never silently resumes an explicitly paused queue.
      let state = try await Bridge.call(["op": "sender_status"])
      paused =
        ((try JSONSerialization.jsonObject(with: state)) as? [String: Any])?["paused"] as? Bool
        ?? true
      concurrentUploads = ((try JSONSerialization.jsonObject(with: state)) as? [String: Any])?["concurrent_uploads"] as? Int ?? 4
      discovery = try PhotoLibraryChanges(root: root)
      try discovery?.matchReceiver(pairing?.receiverID)
      updateDiscoveryStatus()
      let arguments = ProcessInfo.processInfo.arguments
      if let flag = arguments.firstIndex(of: "--retire-delivered-cache"),
        arguments.indices.contains(flag + 1), let current = pairing?.receiverID {
        let result = try await Bridge.call(["op": "retire_delivered_cache",
          "previous_receiver": arguments[flag + 1], "current_receiver": current])
        try result.write(to: root.appendingPathComponent("cache-retirement-result.json"), options: .atomic)
      }
      ready = true
      await refreshPendingImportCount()
      // Initial lifecycle callbacks may arrive before storage is ready. Capture
      // the actual state now without inventing an earlier foreground transition.
      await BackgroundTransfer.shared.recordSnapshot("sender_ready")
      Task { await syncDeviceProfile() }
      await refreshStorage()
      await refreshHistoricalImport()
      if storage?.settings.auto_reclaim == true { await reclaimCache() }
      refreshTask = Task {
        while !Task.isCancelled {
          receiverDiscovery.update(pairing: pairing, active: Self.canPoll)
          if Self.canPoll {
            Task { await self.syncDeviceProfile() }
            await refresh()
            if !paused { _ = await canPrepareForReceiver() }
            await BackgroundTransfer.shared.kick()
            await discoverPhotos()
            await scanHistoricalImport()
            await processPendingImports()
          }
          try? await Task.sleep(nanoseconds: 3_000_000_000)
        }
      }
      if !paused { startWorker() }
      scheduleBackgroundWork()
    } catch {
      opened = false
      message = error.localizedDescription
    }
  }
  func pair(_ payload: String) async {
    guard !pairingInProgress else { return }
    pairingInProgress = true
    pairingError = nil
    defer { pairingInProgress = false }
    do {
      let parsed = try JSONDecoder().decode(Pairing.self, from: Data(payload.utf8))
      // Rust validates identity/certificate and establishes TLS before trust is saved.
      _ = try await Bridge.call([
        "op": "check_pairing",
        "pairing": try JSONSerialization.jsonObject(with: JSONEncoder().encode(parsed)),
      ])
      if let saved = pairing, saved.receiverID == parsed.receiverID, saved.endpoint != parsed.endpoint {
        // Keep saved trust when a QR code supplies a new route for the same peer.
        let restored = await relocateReceiver(to: parsed.endpoint, expected: saved)
        pairingError = restored ? nil : NSLocalizedString("error_network", comment: "")
        return
      }
      // Cancel requests signed for the old receiver before replacing its trust.
      if pairing?.receiverID != parsed.receiverID {
        await setPaused(true)
        try discovery?.setEnabled(false, receiverID: parsed.receiverID)
        updateDiscoveryStatus()
      }
      try await Keychain.saveAsync(try JSONEncoder().encode(parsed))
      pairing = parsed
      historyControlRevision += 1
      historicalImport = nil
      historicalCursor = nil
      historyError = nil
      await refreshHistoricalImport()
      Task { await self.syncDeviceProfile(force: true) }
      queueRevision = -1
      await refresh()
      if !paused { startWorker() }
    } catch { pairingError = error.localizedDescription }
  }
  @discardableResult func relocateReceiver(to endpoint: String, expected: Pairing) async -> Bool {
    guard pairing?.receiverID == expected.receiverID, pairing?.endpoint == expected.endpoint,
      endpoint != expected.endpoint else { return false }
    do {
      let checked = try await Bridge.call(["op": "relocate_pairing",
        "pairing": try JSONSerialization.jsonObject(with: JSONEncoder().encode(expected)), "endpoint": endpoint])
      let updated = try JSONDecoder().decode(Pairing.self, from: checked)
      await BackgroundTransfer.shared.replaceRoute(updated, expected: expected)
      return pairing?.receiverID == updated.receiverID && pairing?.endpoint == updated.endpoint
    } catch {
      // Unauthenticated discovery must not overwrite trust or turn a transient
      // route probe into a permanent transfer failure. Normal retry/QR remains.
      return false
    }
  }
  func setConcurrentUploads(_ limit: Int) async {
    guard !savingConcurrency else { return }
    savingConcurrency = true
    defer { savingConcurrency = false }
    do {
      _ = try await Bridge.call(["op": "set_transfer_concurrency", "limit": limit])
      concurrentUploads = limit
      if !paused { await BackgroundTransfer.shared.kick() }
    } catch { message = error.localizedDescription }
  }
  func setPaused(_ value: Bool) async {
    do {
      _ = try await Bridge.call(["op": "pause_sender", "paused": value])
      paused = value
      if value {
        await BackgroundTransfer.shared.cancelAll()
      } else {
        startWorker()
        await discoverPhotos()
      }
      scheduleBackgroundWork()
    } catch { message = error.localizedDescription }
  }
  private func startWorker() {
    guard pairing != nil, ready else { return }
    Task { await BackgroundTransfer.shared.kick() }
  }
  func updateDiscoveryStatus() {
    autoBackup = discovery?.state.enabled ?? false
    discoveryPending = discovery?.state.pending.count ?? 0
    historyUnavailable = discovery?.state.historyUnavailable ?? false
  }
  func setAutoBackup(_ enabled: Bool) async {
    guard ready, pairing != nil else { return }
    if enabled, !(await authorizePhotos()) { return }
    do {
      try discovery?.setEnabled(enabled, receiverID: pairing?.receiverID)
      updateDiscoveryStatus()
      scheduleBackgroundWork()
    } catch { message = error.localizedDescription }
  }
  func discoverPhotos() async {
    guard ready else { return }
    await discovery?.discoverAndExport(using: self)
  }
  func becameActive() async {
    await open()
    await BackgroundTransfer.shared.kick(reconcile: true)
    await discoverPhotos()
    await scanHistoricalImport()
    await processPendingImports()
    await refresh()
  }
  func scheduleBackgroundWork() {
    #if os(iOS)
      BGTaskScheduler.shared.cancel(taskRequestWithIdentifier: AppDelegate.processingIdentifier)
      guard ready, !paused, pairing != nil,
        autoBackup || historicalImport?.state == "scanning" || pendingImports > 0
          || jobs.contains(where: { ["queued", "running", "waiting"].contains($0.state) })
      else { return }
      let request = BGProcessingTaskRequest(identifier: AppDelegate.processingIdentifier)
      request.requiresNetworkConnectivity = true
      request.earliestBeginDate = Date().addingTimeInterval(15 * 60)
      do {
        try BGTaskScheduler.shared.submit(request)
        lastScheduleError = nil
      } catch {
        let code = (error as NSError).code
        if lastScheduleError != code {
          lastScheduleError = code
          Task { await BackgroundTransfer.shared.record("processing_schedule_rejected",
            context: ["system_error": code]) }
        }
      }
    #endif
  }
  private static var canPoll: Bool {
    #if os(iOS)
      return UIApplication.shared.applicationState == .active
    #else
      return true
    #endif
  }
  func loadMoreJobs() async {
    jobsLimit += 200
    queueRevision = -1
    await refresh()
  }
  func refresh() async {
    guard !refreshing else { return }
    refreshing = true
    defer { refreshing = false }
    waitingForNetwork = BackgroundTransfer.shared.waitingForNetwork
    if Date().timeIntervalSince(lastStorageRefresh) > 15 { await refreshStorage() }
    do {
      let revisionData = try await Bridge.call(["op": "sender_revision"])
      let revision = try JSONDecoder().decode(Int64.self, from: revisionData)
      guard revision != queueRevision else { return }
      var all: [BackupJob] = []
      var after: Int64 = 0
      repeat {
        let data = try await Bridge.call(["op": "jobs", "after": after])
        let page = try JSONDecoder().decode([BackupJob].self, from: data)
        all.append(contentsOf: page)
        guard page.count == 200, let last = page.last else { break }
        after = last.id
      } while all.count < jobsLimit
      hasMoreJobs = all.count >= jobsLimit
      if jobs != all { jobs = all }
      if let pairing {
        let data = try await Bridge.call([
          "op": "sender_summary", "receiver_id": pairing.receiverID,
        ])
        summary = try JSONDecoder().decode(SenderSummary.self, from: data)
      }
      queueRevision = revision
      waitingForNetwork = BackgroundTransfer.shared.waitingForNetwork
    } catch { message = error.localizedDescription }
  }
  func retry(_ id: Int64) async {
    do {
      _ = try await Bridge.call(["op": "retry", "id": id])
      await refresh()
      if !paused { startWorker() }
    } catch { message = error.localizedDescription }
  }
  func authorizePhotos() async -> Bool {
    let access = await PHPhotoLibrary.requestAuthorization(for: .readWrite)
    let allowed = access == .authorized || access == .limited
    if !allowed { message = NSLocalizedString("photos_permission_needed", comment: "") }
    return allowed
  }
  @discardableResult func importAssets(_ identifiers: [String], requestAuthorization: Bool = true)
    async -> Bool
  {
    guard let target = pairing, !importing else { return false }
    var succeeded = true
    importing = true
    defer {
      importing = false
      importingSourceID = nil
      exportProgress = nil
    }
    do {
      for start in stride(from: 0, to: identifiers.count, by: 10000) {
        _ = try await Bridge.call([
          "op": "schedule_sources", "receiver_id": target.receiverID,
          "sources": Array(identifiers[start..<min(start + 10000, identifiers.count)]),
        ])
      }
      let pending = try await Bridge.call([
        "op": "pending_sources", "receiver_id": target.receiverID,
      ])
      pendingImports =
        (try JSONSerialization.jsonObject(with: pending) as? [String: Any])?["count"] as? Int ?? 0
    } catch {
      message = error.localizedDescription
      return false
    }
    // Keep enumeration batches and preparation separate. If discovery/manual
    // selection arrives during a scan, its identifiers are already durable;
    // the regular preparation worker will consume them after the batch finishes.
    // This prevents a rescan from replacing source membership during export.
    if scanningHistory { return true }
    let authorization =
      requestAuthorization
      ? await PHPhotoLibrary.requestAuthorization(for: .readWrite)
      : PHPhotoLibrary.authorizationStatus(for: .readWrite)
    guard authorization == .authorized || authorization == .limited else {
      message = NSLocalizedString("photos_permission_needed", comment: "")
      return false
    }
    let options = photoLibraryFetchOptions()
    options.sortDescriptors = [NSSortDescriptor(key: "creationDate", ascending: false)]
    let assets = PHAsset.fetchAssets(withLocalIdentifiers: identifiers, options: options)
    let orderedAssets = (0..<assets.count).map { assets.object(at: $0) }.sorted {
      PhotoBackupOrder.precedes($0.localIdentifier, PhotoBackupOrder.timestamp($0.creationDate),
        $1.localIdentifier, PhotoBackupOrder.timestamp($1.creationDate))
    }
    do { try await scheduleBurstSiblings(of: assets, receiver: target.receiverID) }
    catch { message = error.localizedDescription; return false }
    if assets.count != identifiers.count {
      succeeded = false
      message = NSLocalizedString("photos_permission_needed", comment: "")
      let found = Set((0..<assets.count).map { assets.object(at: $0).localIdentifier })
      for missing in identifiers where !found.contains(missing) {
        _ = try? await Bridge.call([
          "op": "source_result", "receiver_id": target.receiverID, "source": missing,
          "complete": false,
        ])
      }
      preparationReason = "source_unavailable"
    }
    for index in 0..<assets.count {
      guard !Task.isCancelled, pairing?.receiverID == target.receiverID else { return false }
      if paused && !requestAuthorization { return false }
      guard await canPrepareForReceiver() else { return false }
      let sourceID = orderedAssets[index].localIdentifier
      let folder = root.appendingPathComponent("exports/" + UUID().uuidString, isDirectory: true)
      var retained = false
      defer { if !retained { try? FileManager.default.removeItem(at: folder) } }
      do {
        let asset = orderedAssets[index]
        // Check the source revision before downloading originals, including
        // manual selection and metadata replay after a historical-scan restart.
        let existingData = try await Bridge.call(["op": "source_states", "receiver_id": target.receiverID,
          "sources": [[sourceID, PhotoLibraryModel.revision(asset)]]])
        let existing = try JSONDecoder().decode([String: String].self, from: existingData)
        if let state = existing[sourceID], state != "failed" {
          _ = try await Bridge.call(["op": "source_result", "receiver_id": target.receiverID,
            "source": sourceID, "complete": true])
          continue
        }
        importingSourceID = asset.localIdentifier
        exportProgress = nil
        try FileManager.default.createDirectory(at: folder, withIntermediateDirectories: true)
        let available = PHAssetResource.assetResources(for: asset)
        let live = asset.mediaSubtypes.contains(.photoLive)
        let video = asset.mediaType == .video
        guard let primary = available.first(where: { $0.type == (video ? .video : .photo) }) else {
          throw Bridge.Failure(code: "source_unavailable")
        }
        var selected: [(PHAssetResource, String)] = [(primary, video ? "video" : "photo")]
        if live {
          guard let paired = available.first(where: { $0.type == .pairedVideo }) else {
            throw Bridge.Failure(code: "source_unavailable")
          }
          selected.append((paired, "paired_video"))
        }
        var resources: [[String: Any]] = []
        for (resource, role) in selected {
          let destination = folder.appendingPathComponent(role + "-" + resource.originalFilename)
          try await export(resource, to: destination)
          try Task.checkCancellation()
          guard pairing?.receiverID == target.receiverID else { return false }
          let ext = (resource.originalFilename as NSString).pathExtension.lowercased()
          let mime: [String: String] = [
            "heic": "image/heic", "heif": "image/heif", "jpg": "image/jpeg", "jpeg": "image/jpeg",
            "png": "image/png", "mov": "video/quicktime", "mp4": "video/mp4",
          ]
          guard let type = mime[ext] else { throw Bridge.Failure(code: "unsupported") }
          resources.append([
            "role": role, "filename": resource.originalFilename, "media_type": type,
            "path": destination.path,
          ])
        }
        guard await canPrepareForReceiver() else { return false }
        var metadata = try await burstFields(for: asset)
        metadata["favorite"] = String(asset.isFavorite)
        if let date = asset.creationDate {
          metadata["created_at_ms"] = String(Int64(date.timeIntervalSince1970 * 1000))
        }
        if let location = asset.location {
          metadata["latitude"] = String(location.coordinate.latitude)
          metadata["longitude"] = String(location.coordinate.longitude)
        }
        let added = try await Bridge.call([
          "op": "enqueue", "receiver_id": target.receiverID, "source_id": asset.localIdentifier,
          "revision": String(
            Int64(
              (asset.modificationDate ?? asset.creationDate ?? Date(timeIntervalSince1970: 0))
                .timeIntervalSince1970 * 1000)),
          "kind": live ? "motion" : video ? "video" : "photo", "metadata": metadata,
          "resources": resources,
        ])
        retained = true
        if let response = try? JSONSerialization.jsonObject(with: added) as? [String: Any],
          let sources = response["sources"] as? [String: String]
        {
          retained = sources.values.contains { $0.hasPrefix(folder.path + "/") }
        }
        _ = try await Bridge.call([
          "op": "source_result", "receiver_id": target.receiverID, "source": sourceID,
          "complete": true,
        ])
        preparationReason = nil
        message = nil
        await refresh()
        if !paused { startWorker() }
      } catch {
        succeeded = false
        message = error.localizedDescription
        let reason = (error as? Bridge.Failure)?.code ?? "source_unavailable"
        preparationReason = reason
        _ = try? await Bridge.call(["op": "record_event", "receiver": false, "code": reason])
        _ = try? await Bridge.call([
          "op": "source_result", "receiver_id": target.receiverID, "source": sourceID,
          "complete": false,
        ])
      }
    }
    if let data = try? await Bridge.call([
      "op": "pending_sources", "receiver_id": target.receiverID,
    ]),
      let pending = try? JSONSerialization.jsonObject(with: data) as? [String: Any]
    {
      pendingImports = pending["count"] as? Int ?? 0
    }
    if !paused { startWorker() }
    scheduleBackgroundWork()
    return succeeded
  }
  func refreshStorage() async {
    guard ready else { return }
    do {
      let data = try await Bridge.call(["op": "storage_status", "receiver": false])
      storage = try JSONDecoder().decode(StorageSnapshot.self, from: data)
      storageError = nil
      lastStorageRefresh = Date()
    } catch { storageError = error.localizedDescription }
  }
  func saveStorage(_ settings: StorageSettings) async {
    guard !savingStorage else { return }
    savingStorage = true
    storageError = nil
    defer { savingStorage = false }
    do {
      _ = try await Bridge.call([
        "op": "save_storage_settings", "receiver": false,
        "settings": JSONSerialization.jsonObject(with: JSONEncoder().encode(settings)),
      ])
      await refreshStorage()
      if settings.auto_reclaim { await reclaimCache() }
      await processPendingImports()
    } catch let error as Bridge.Failure where error.code == "invalid_input" {
      storageError = NSLocalizedString("storage_settings_invalid", comment: "")
    } catch { storageError = error.localizedDescription }
  }
  func reclaimCache(reportResult: Bool = false) async {
    guard !reclaimingCache else { return }
    reclaimingCache = true
    if reportResult { cacheReclaimResult = nil }
    defer { reclaimingCache = false }
    do {
      let data = try await Bridge.call(["op": "reclaim_sender_cache"])
      if reportResult {
        let result = try JSONSerialization.jsonObject(with: data) as? [String: Any]
        let bytes = (result?["reclaimed_bytes"] as? NSNumber)?.int64Value ?? 0
        cacheReclaimResult = bytes == 0 ? NSLocalizedString("storage_reclaim_none", comment: "")
          : String(format: NSLocalizedString("storage_reclaim_result", comment: ""),
            ByteCountFormatter.string(fromByteCount: bytes, countStyle: .file))
      }
      await refreshStorage()
    } catch { storageError = error.localizedDescription }
  }
  /// Durable work must be loaded even when storage/network gates prevent export.
  func refreshPendingImportCount() async {
    guard ready, let target = pairing else { return }
    do {
      let data = try await Bridge.call(["op": "pending_sources", "receiver_id": target.receiverID])
      guard pairing?.receiverID == target.receiverID else { return }
      let result = try JSONSerialization.jsonObject(with: data) as? [String: Any]
      if let count = result?["count"] as? Int { pendingImports = count }
    } catch { /* Preserve the last known count rather than reporting an empty queue. */ }
  }
  func updatePreparationStorageState() async {
    guard let storage else { return }
    if storage.export_allowance == 0 && (pendingImports > 0 || discoveryPending > 0) {
      let reason = storage.reason ?? storage.limitingReason
      if preparationReason != reason {
        preparationReason = reason
        await BackgroundTransfer.shared.recordSnapshot("preparation_storage_blocked")
      }
    } else if preparationReason == "local_free_space" || preparationReason == "local_cache_budget" {
      preparationReason = nil
    }
  }
  private var orderingPendingSources = false
  func processPendingImports() async {
    guard ready, !importing, !orderingPendingSources, let pairing else { return }
    orderingPendingSources = true
    defer { orderingPendingSources = false }
    do {
      // Persist capture dates once, including existing queues, before choosing a
      // batch. Read metadata off-main; never download originals for ordering.
      var result: [String: Any]?
      while true {
        guard !Task.isCancelled, self.pairing?.receiverID == pairing.receiverID else { return }
        let data = try await Bridge.call(["op": "pending_sources", "receiver_id": pairing.receiverID])
        guard self.pairing?.receiverID == pairing.receiverID else { return }
        result = try JSONSerialization.jsonObject(with: data) as? [String: Any]
        pendingImports = result?["count"] as? Int ?? 0
        let access = PHPhotoLibrary.authorizationStatus(for: .readWrite)
        guard !paused, access == .authorized || access == .limited else { return }
        guard await canPrepareForReceiver() else { return }
        let unordered = result?["unordered"] as? [String] ?? []
        if unordered.isEmpty { break }
        let dates = await PhotoBackupOrder.captureDates(unordered)
        try Task.checkCancellation()
        _ = try await Bridge.call(["op": "source_dates", "receiver_id": pairing.receiverID,
          "dates": unordered.map { [$0, dates[$0] ?? Int64.min] as [Any] }])
      }
      if pendingImports == 0 { preparationReason = nil }
      if let storage, storage.export_allowance == 0 {
        preparationReason = storage.reason
        return
      }
      if !paused, let sources = result?["sources"] as? [String], !sources.isEmpty {
        _ = await importAssets(sources, requestAuthorization: false)
      }
    } catch { message = error.localizedDescription }
  }
  var waitingReason: String? {
    if paused { return nil }
    if receiverUnavailable { return "receiver_unavailable" }
    if let preparationReason { return preparationReason }
    if waitingForNetwork { return "network" }
    if summary.running == 0 && summary.queued == 0 { return summary.waiting_reason }
    return nil
  }
  private func export(_ resource: PHAssetResource, to url: URL) async throws {
    await refreshStorage()
    guard let snapshot = storage else { throw Bridge.Failure(code: "storage") }
    guard snapshot.export_allowance > 0 else {
      throw Bridge.Failure(code: snapshot.reason ?? snapshot.limitingReason)
    }
    guard await canPrepareForReceiver() else { throw Bridge.Failure(code: "receiver_unavailable") }
    let writer = try BoundedExportWriter(url: url, snapshot: snapshot)
    activeExport = writer
    defer { activeExport = nil }
    let options = PHAssetResourceRequestOptions()
    options.isNetworkAccessAllowed = true
    options.progressHandler = { progress in Task { @MainActor in self.exportProgress = progress } }
    try await withTaskCancellationHandler {
      try await withCheckedThrowingContinuation {
        (continuation: CheckedContinuation<Void, Error>) in
        let id = PHAssetResourceManager.default().requestData(
          for: resource, options: options, dataReceivedHandler: { writer.consume($0) },
          completionHandler: { error in
            if let error = writer.finish(error) {
              continuation.resume(throwing: error)
            } else {
              continuation.resume()
            }
          })
        writer.bind(id)
      }
    } onCancel: {
      writer.cancel()
    }
  }

}

enum Keychain {
  private static let query: [String: Any] = [
    kSecClass as String: kSecClassGenericPassword,
    kSecAttrService as String: "app.photobridge.pairing", kSecAttrAccount as String: "receiver",
  ]
  // Security framework calls can wait for system authorization after an ad-hoc
  // build changes. Never make that wait block AppKit or SwiftUI's main thread.
  static func readAsync() async throws -> Data? {
    try await withCheckedThrowingContinuation { continuation in
      DispatchQueue.global(qos: .userInitiated).async {
        continuation.resume(with: Result { try read() })
      }
    }
  }
  static func saveAsync(_ data: Data) async throws {
    try await withCheckedThrowingContinuation { (continuation: CheckedContinuation<Void, Error>) in
      DispatchQueue.global(qos: .userInitiated).async {
        continuation.resume(with: Result { try save(data) })
      }
    }
  }
  static func read() throws -> Data? {
    var q = query
    q[kSecReturnData as String] = true
    q[kSecMatchLimit as String] = kSecMatchLimitOne
    var item: CFTypeRef?
    let status = SecItemCopyMatching(q as CFDictionary, &item)
    if status == errSecItemNotFound { return nil }
    guard status == errSecSuccess else { throw Bridge.Failure(code: "storage") }
    return item as? Data
  }
  static func save(_ data: Data) throws {
    let update: [String: Any] = [kSecValueData as String: data]
    var status = SecItemUpdate(query as CFDictionary, update as CFDictionary)
    if status == errSecItemNotFound {
      var q = query
      q[kSecValueData as String] = data
      q[kSecAttrAccessible as String] = kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly
      status = SecItemAdd(q as CFDictionary, nil)
    }
    guard status == errSecSuccess else { throw Bridge.Failure(code: "storage") }
  }
}

struct SenderSummary: Decodable {
  var waiting_reason: String?
  var next_retry_at: Int64?
  var total = 0
  var received = 0
  var waiting = 0
  var failed = 0
  var queued = 0
  var running = 0
  var paused = 0
  var confirmed_bytes: Int64 = 0
  func count(for state: String) -> Int {
    switch state {
    case "received": return received
    case "waiting": return waiting
    case "failed": return failed
    case "queued": return queued
    case "running": return running
    case "paused": return paused
    default: return total
    }
  }
}

extension BackupModel {
  func transferCount(for state: String) -> Int {
    switch state {
    case "preparing": return pendingImports
    case "scanned": return historicalImport?.checked ?? 0
    default: return summary.count(for: state)
    }
  }
}
