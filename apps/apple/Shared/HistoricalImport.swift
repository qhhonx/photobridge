import Foundation
import Photos
import SwiftUI

struct HistoricalImportStatus: Decodable, Equatable {
  let run: Int64
  let state: String
  let checked: Int
  let pending: Int
}

/// A process-local PhotoKit snapshot, never a durable offset into a mutable library.
/// On restart enumerate metadata again; Rust remembers membership and queued revisions.
@MainActor final class HistoricalScanCursor {
  let receiver: String
  let run: Int64
  let assets: PHFetchResult<PHAsset>
  var offset = 0
  init(receiver: String, run: Int64, assets: PHFetchResult<PHAsset>) {
    self.receiver = receiver; self.run = run; self.assets = assets
  }
}

extension BackupModel {
  func refreshHistoricalImport() async {
    guard ready, let target = pairing else { return }
    let version = historyControlRevision
    do {
      let data = try await Bridge.call(["op": "history_status", "receiver_id": target.receiverID])
      let status = try JSONDecoder().decode(HistoricalImportStatus?.self, from: data)
      guard pairing?.receiverID == target.receiverID, version == historyControlRevision else { return }
      if historicalImport != status { historicalImport = status }
      historyError = nil
    } catch { historyError = NSLocalizedString("history_operation_failed", comment: "") }
  }
  func controlHistoricalImport(_ action: String) async {
    guard ready, let target = pairing, !changingHistory else { return }
    changingHistory = true
    historyControlRevision += 1
    defer { changingHistory = false }
    if action != "pause", !(await authorizePhotos()) {
      historyError = NSLocalizedString("photos_permission_needed", comment: "")
      return
    }
    do {
      let data = try await Bridge.call(["op": "history_control", "receiver_id": target.receiverID,
        "action": action])
      guard pairing?.receiverID == target.receiverID else { return }
      historicalImport = try JSONDecoder().decode(HistoricalImportStatus.self, from: data)
      historyError = nil
      scheduleBackgroundWork()
      Task { await scanHistoricalImport() }
    } catch { historyError = NSLocalizedString("history_operation_failed", comment: "") }
  }
  func scanHistoricalImport() async {
    guard ready, !importing, !scanningHistory, !changingHistory, let target = pairing else { return }
    scanningHistory = true
    defer { scanningHistory = false }
    await refreshHistoricalImport()
    guard !paused, let status = historicalImport, status.state == "scanning" else { return }
    let access = PHPhotoLibrary.authorizationStatus(for: .readWrite)
    guard access == .authorized || access == .limited else {
      historyError = NSLocalizedString("photos_permission_needed", comment: "")
      historicalCursor = nil
      return
    }
    guard await canPrepareForReceiver() else { return }
    let version = historyControlRevision
    do {
      if historicalCursor?.receiver != target.receiverID || historicalCursor?.run != status.run {
        let snapshot = await Task.detached(priority: .utility) {
          let options = photoLibraryFetchOptions()
          options.sortDescriptors = [NSSortDescriptor(key: "creationDate", ascending: false)]
          options.predicate = NSPredicate(format: "mediaType == %d OR mediaType == %d",
            PHAssetMediaType.image.rawValue, PHAssetMediaType.video.rawValue)
          return PHAsset.fetchAssets(with: options)
        }.value
        guard !Task.isCancelled, !paused, version == historyControlRevision,
          pairing?.receiverID == target.receiverID else { return }
        historicalCursor = HistoricalScanCursor(receiver: target.receiverID, run: status.run, assets: snapshot)
      }
      guard let cursor = historicalCursor else { return }
      let start = cursor.offset
      let end = min(start + 200, cursor.assets.count)
      let assets = cursor.assets
      let sources = await Task.detached(priority: .utility) {
        (start..<end).map { index -> [String] in
          let asset = assets.object(at: index)
          let revision = String(Int64((asset.modificationDate ?? asset.creationDate ?? Date(timeIntervalSince1970: 0)).timeIntervalSince1970 * 1000))
          return [asset.localIdentifier, revision]
        }
      }.value
      guard !Task.isCancelled, !paused, version == historyControlRevision,
        pairing?.receiverID == target.receiverID else { return }
      let data = try await Bridge.call(["op": "history_batch", "receiver_id": target.receiverID,
        "run": status.run, "sources": sources, "finished": end == assets.count])
      guard version == historyControlRevision, pairing?.receiverID == target.receiverID else { return }
      historicalImport = try JSONDecoder().decode(HistoricalImportStatus.self, from: data)
      cursor.offset = end // Advance only after the batch and pending work commit together.
      if historicalImport?.state == "scanned" { historicalCursor = nil }
      historyError = nil
    } catch {
      guard version == historyControlRevision else { return }
      historyError = NSLocalizedString("history_operation_failed", comment: "")
    }
  }
}

struct HistoricalImportSettings: View {
  @ObservedObject var model: BackupModel
  var body: some View {
    Section {
      if let status = model.historicalImport {
        LabeledContent("history_scan_status", value: NSLocalizedString("history_state_" + status.state, comment: ""))
        Text(String(format: NSLocalizedString("history_counts", comment: ""), status.checked, status.pending))
          .foregroundStyle(.secondary)
        if status.state == "scanning" {
          Button("history_pause") { Task { await model.controlHistoricalImport("pause") } }
        } else if status.state == "paused" {
          Button("history_resume") { Task { await model.controlHistoricalImport("resume") } }
        } else {
          Button("history_scan_again") { Task { await model.controlHistoricalImport("start") } }
        }
      } else {
        Button("history_start") { Task { await model.controlHistoricalImport("start") } }
      }
      if model.paused, model.historicalImport?.state == "scanning" {
        Text("history_backup_paused").foregroundStyle(.secondary)
        Button("resume_backup") { Task { await model.setPaused(false); await model.scanHistoricalImport() } }
      }
      if let error = model.historyError {
        Text(error).foregroundStyle(.orange)
        Button("retry_task") { Task { await model.refreshHistoricalImport(); await model.scanHistoricalImport() } }
      }
    } header: { Text("history_title") } footer: { Text("history_explanation") }
      .disabled(!model.ready || model.pairing == nil || model.changingHistory)
      .task { await model.refreshHistoricalImport() }
  }
}

/// A compact overview; detailed scan controls live with backup preferences.
struct HistoricalImportProgress: View {
  @ObservedObject var model: BackupModel
  var body: some View {
    if let status = model.historicalImport, status.state != "scanned" || status.pending > 0 {
      VStack(alignment: .leading, spacing: 8) {
        Label("history_title", systemImage: "photo.stack").font(.headline)
        Text(model.paused && status.state == "scanning" ? "history_backup_paused" : LocalizedStringKey("history_state_" + status.state))
        Text(String(format: NSLocalizedString("history_counts", comment: ""), status.checked, status.pending))
          .font(.callout).foregroundStyle(.secondary)
        if let error = model.historyError { Text(error).font(.callout).foregroundStyle(.orange) }
      }.frame(maxWidth: .infinity, alignment: .leading)
    }
  }
}

/// Preparation is durable work, but not yet a transfer job. Keep its browser separate.
struct SourceBrowser: View {
  @ObservedObject var model: BackupModel
  let history: Bool
  @StateObject private var browser = SourceBrowserModel()
  private var queryID: String {
    "\(history)|\(model.pairing?.receiverID ?? "")|\(model.queueRevision)|\(model.pendingImports)|\(model.importingSourceID ?? "")|\(model.historicalImport?.run ?? 0)|\(model.historicalImport?.checked ?? 0)|\(model.historicalImport?.state ?? "")"
  }
  var body: some View {
    VStack(alignment: .leading, spacing: 12) {
      if history {
        Text(LocalizedStringKey("history_state_" + (model.historicalImport?.state ?? "not_started")))
          .font(.callout).foregroundStyle(.secondary)
        Text("history_list_explanation").font(.caption).foregroundStyle(.secondary)
      }
      Text(String(format: NSLocalizedString("source_list_count", comment: ""), browser.items.count, browser.total))
        .font(.caption).foregroundStyle(.secondary)
      ScrollView {
        LazyVStack(alignment: .leading, spacing: 0) {
          if browser.loading && browser.items.isEmpty {
            ProgressView().frame(maxWidth: .infinity).padding(24)
          } else if browser.failed {
            Button("tasks_load_failed") { Task { await refresh() } }
              .frame(maxWidth: .infinity).padding(24)
          } else if browser.items.isEmpty {
            ContentUnavailableView("tasks_empty_filtered", systemImage: "photo.stack")
              .frame(maxWidth: .infinity).padding(.top, 24)
          }
          ForEach(browser.items) { item in
            SourceBrowserRow(item: item, active: model.importingSourceID == item.source,
              progress: model.exportProgress)
            Divider()
          }
          if browser.hasMore {
            Button("load_more_tasks") { Task { await browser.loadMore() } }.padding(.vertical, 20)
          }
          BackupReceiptExplanation().padding(.bottom, 12)
        }
      }
    }.task(id: queryID) { await refresh() }
  }
  private func refresh() async {
    await browser.refresh(receiver: model.pairing?.receiverID, history: history,
      run: model.historicalImport?.run)
  }
}

struct SourceBrowserItem: Decodable, Identifiable, Equatable {
  let cursor: Int64
  let source: String
  let revision: String
  let retry_at: Int64?
  let state: String?
  var id: String { source }
}

@MainActor final class SourceBrowserModel: ObservableObject {
  @Published var items: [SourceBrowserItem] = []
  @Published var total = 0
  @Published var loading = false
  @Published var failed = false
  @Published var hasMore = false
  private var receiver: String?
  private var history = false
  private var run: Int64?
  private var limit = 100
  private var generation = 0
  private struct Page: Decodable {
    let items: [SourceBrowserItem]
    let total: Int
    let run: Int64?
  }
  func loadMore() async {
    guard !loading else { return }
    limit += 100
    await refresh(receiver: receiver, history: history, run: run)
  }
  func refresh(receiver: String?, history: Bool, run: Int64?) async {
    if self.receiver != receiver || self.history != history || (history && self.run != run) {
      items = []; total = 0; hasMore = false; limit = 100
    }
    self.receiver = receiver; self.history = history; self.run = run
    generation += 1
    let expected = generation
    guard let receiver else { items = []; total = 0; hasMore = false; loading = false; return }
    loading = true; failed = false
    defer { if generation == expected { loading = false } }
    do {
      var result: [SourceBrowserItem] = []
      var cursor: Int64 = 0
      var total = 0
      var firstRun: Int64?
      repeat {
        let data = try await Bridge.call(["op": "browse_sources", "receiver_id": receiver,
          "history": history, "after": cursor])
        let page = try JSONDecoder().decode(Page.self, from: data)
        guard expected == generation, !Task.isCancelled else { return }
        if cursor == 0 { firstRun = page.run }
        // A new scan cannot be joined to pages from the previous scan.
        if history && firstRun != page.run { return }
        result.append(contentsOf: page.items)
        total = page.total
        guard page.items.count == 100, let last = page.items.last else { break }
        cursor = last.cursor
      } while result.count < limit
      if items != result { items = result }
      self.total = total
      hasMore = result.count < total && result.count >= limit
    } catch {
      guard expected == generation, !Task.isCancelled else { return }
      failed = true
    }
  }
}

private struct SourceBrowserRow: View {
  let item: SourceBrowserItem
  let active: Bool
  let progress: Double?
  @State private var title = ""
  @State private var kind = "photo"
  @State private var captured: Date?
  private var state: String { active ? "preparing" : item.state ?? "preparing" }
  var body: some View {
    HStack(spacing: 14) {
      AssetThumbnail(sourceID: item.source)
      VStack(alignment: .leading, spacing: 7) {
        Text(title.isEmpty ? NSLocalizedString("source_photo_unavailable", comment: "") : title).lineLimit(1)
        HStack {
          Label(LocalizedStringKey("library_filter_" + kind), systemImage:
            kind == "burst" ? "square.stack" : kind == "motion" ? "livephoto" : kind == "video" ? "video" : "photo")
          if let captured { Text(captured.formatted(date: .abbreviated, time: .shortened)) }
        }.font(.caption).foregroundStyle(.secondary)
        Label(active ? "importing_originals" : LocalizedStringKey("state_" + state),
          systemImage: taskSymbol(state)).font(.caption).foregroundStyle(.secondary)
        if active {
          if let progress { ProgressView(value: progress) } else { ProgressView().controlSize(.small) }
        } else if let retry = item.retry_at, retry > Int64(Date().timeIntervalSince1970) {
          Text(String(format: NSLocalizedString("task_retry_time", comment: ""),
            Date(timeIntervalSince1970: Double(retry)).formatted(date: .omitted, time: .standard)))
            .font(.caption).foregroundStyle(.secondary)
        }
      }
      Spacer(minLength: 0)
    }.padding(.vertical, 10)
      .task(id: item.source) {
        let source = item.source
        let details = await Task.detached(priority: .utility) { () -> (String, String, Date?)? in
          guard let asset = PHAsset.fetchAssets(withLocalIdentifiers: [source], options: photoLibraryFetchOptions()).firstObject else { return nil }
          return (PHAssetResource.assetResources(for: asset).first?.originalFilename ?? "",
            asset.burstIdentifier != nil ? "burst" : asset.mediaSubtypes.contains(.photoLive) ? "motion" : asset.mediaType == .video ? "video" : "photo",
            asset.creationDate)
        }.value
        guard !Task.isCancelled, let details else { return }
        title = details.0; kind = details.1; captured = details.2
      }
  }
}
