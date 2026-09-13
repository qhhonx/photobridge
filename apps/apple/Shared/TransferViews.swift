import Photos
import SwiftUI

struct AssetThumbnail: View {
  let sourceID: String
  var size: CGFloat = 56
  @State private var image: LibraryImage?
  @State private var request: PHImageRequestID = PHInvalidImageRequestID
  @State private var requestToken = UUID()
  private static let pipeline = ThumbnailPipeline()
  var body: some View {
    ZStack {
      Rectangle().fill(.quaternary)
      if let image {
        #if os(macOS)
          Image(nsImage: image).resizable().scaledToFill()
        #else
          Image(uiImage: image).resizable().scaledToFill()
        #endif
      } else {
        Image(systemName: "photo").foregroundStyle(.secondary)
      }
    }.frame(width: size, height: size).clipped().clipShape(RoundedRectangle(cornerRadius: 8))
      .task(id: sourceID) {
        Self.pipeline.cancel(request)
        image = nil
        requestToken = UUID()
        let token = requestToken
        guard
          let asset = PHAsset.fetchAssets(withLocalIdentifiers: [sourceID], options: photoLibraryFetchOptions())
            .firstObject
        else { return }
        request = Self.pipeline.request(asset, pixels: size * 2) { value, _ in
          guard requestToken == token else { return }
          if let value { image = value }
        }
      }.onDisappear {
        requestToken = UUID()
        Self.pipeline.cancel(request)
      }
  }
}

struct TransferRow: View {
  let job: BackupJob
  var progress: TransferProgress?
  private var displayedBytes: UInt64 {
    guard job.state == "running", let progress else { return job.confirmedBytes }
    return progress.displayedBytes(confirmed: job.confirmedBytes, total: job.totalBytes)
  }
  private var statusKey: String {
    guard job.state == "running" else { return "state_" + job.state }
    if progress?.waitingForNetwork == true { return "waiting_for_wifi" }
    guard let progress, progress.sent > 0 else { return "state_scheduled" }
    return progress.expected > 0 && progress.sent >= progress.expected ? "state_confirming" : "state_running"
  }
  var retry: () -> Void
  var body: some View {
    HStack(spacing: 14) {
      AssetThumbnail(sourceID: job.asset.source_id)
      VStack(alignment: .leading, spacing: 7) {
        HStack {
          Text(job.asset.resources.first?.filename ?? "").lineLimit(1)
          Spacer()
          Image(systemName: taskSymbol(job.state)).foregroundStyle(
            job.state == "received" ? .green : job.state == "failed" ? .orange : .secondary)
        }
        HStack(spacing: 8) {
          Label(LocalizedStringKey("library_filter_" + (job.asset.metadata?["burst_group_ref"] != nil ? "burst" : job.asset.kind)),
            systemImage: job.asset.metadata?["burst_group_ref"] != nil ? "square.stack" : job.asset.kind == "motion" ? "livephoto" : job.asset.kind == "video" ? "video" : "photo")
          if let milliseconds = job.asset.metadata?["created_at_ms"].flatMap(Double.init) {
            Text(Date(timeIntervalSince1970: milliseconds / 1000).formatted(date: .abbreviated, time: .shortened))
              .lineLimit(1)
          }
        }.font(.caption).foregroundStyle(.secondary)
        if job.state != "received" {
          ProgressView(value: Double(displayedBytes), total: Double(max(1, job.totalBytes)))
        }
        HStack {
          Text(
            LocalizedStringKey(statusKey))
          Spacer()
          Text(
            ByteCountFormatter.string(fromByteCount: Int64(displayedBytes), countStyle: .file)
              + " / "
              + ByteCountFormatter.string(fromByteCount: Int64(job.totalBytes), countStyle: .file)
          ).monospacedDigit()
        }.font(.caption).foregroundStyle(.secondary)
        if job.state == "waiting", let next = job.nextAttemptAt, next > 0 {
          Text(String(format: NSLocalizedString("task_retry_time", comment: ""),
            Date(timeIntervalSince1970: Double(next)).formatted(date: .omitted, time: .standard)))
            .font(.caption).foregroundStyle(.secondary)
        }
        if let error = job.errorCode {
          HStack {
            Text(LocalizedStringKey("error_" + error)).font(.caption).foregroundStyle(.secondary)
            Spacer()
            Button("retry_task", action: retry).font(.caption)
          }
        }
      }
    }.padding(.vertical, 10)
  }
}

struct TransferList: View {
  @ObservedObject var model: BackupModel
  @StateObject private var browser = TaskBrowserModel()
  @State private var filter: String
  init(model: BackupModel, filter: String = "all") {
    self.model = model
    _filter = State(initialValue: filter)
  }
  private var queryID: String {
    "\(filter)|\(model.queueRevision)|\(model.pairing?.receiverID ?? "")"
  }
  var body: some View {
    VStack(alignment: .leading, spacing: 20) {
      HStack {
        VStack(alignment: .leading, spacing: 6) {
          Text("transfer_tasks").font(.title2.weight(.medium))
          Text(
            String(
              format: NSLocalizedString("transfer_summary", comment: ""), model.summary.received,
              model.summary.total)
          ).font(.callout).foregroundStyle(.secondary)
        }
        Spacer()
        Picker("task_filter", selection: $filter) {
          Text("library_filter_all").tag("all")
          ForEach(["preparing", "running", "queued", "waiting", "paused", "failed", "received", "scanned"], id: \.self) {
            Text(LocalizedStringKey($0 == "scanned" ? "history_title" : "state_" + $0)).tag($0)
          }
        }.frame(maxWidth: 190).accessibilityIdentifier("transfers.filter")
      }
      if filter == "preparing" || filter == "scanned" {
        SourceBrowser(model: model, history: filter == "scanned")
      } else {
        ScrollView {
          LazyVStack(alignment: .leading, spacing: 0) {
            if browser.loading && browser.jobs.isEmpty {
              ProgressView().frame(maxWidth: .infinity).padding(.top, 36)
            } else if browser.failed {
              VStack(spacing: 12) {
                Text("tasks_load_failed").foregroundStyle(.secondary)
                Button("retry_task") {
                  Task { await browser.refresh(filter: filter, receiver: model.pairing?.receiverID) }
                }
              }.frame(maxWidth: .infinity).padding(.top, 36)
            } else if browser.jobs.isEmpty {
              ContentUnavailableView(
                "tasks_empty_filtered", systemImage: "line.3.horizontal.decrease.circle"
              ).frame(maxWidth: .infinity).padding(.top, 36)
            }
            ForEach(browser.jobs) { job in
              TransferRow(job: job, progress: model.transferProgress[job.id]) {
                Task { await model.retry(job.id) }
              }
              Divider()
            }
            if browser.hasMore {
              Button("load_more_tasks") { Task { await browser.loadMore() } }.padding(.vertical, 20)
            }
            BackupReceiptExplanation().padding(.bottom, 12)
          }
        }
      }
    }.task(id: queryID) {
      if filter != "preparing" && filter != "scanned" {
        await browser.refresh(filter: filter, receiver: model.pairing?.receiverID)
      }
    }
  }
}

/// Filter before pagination so a failure beyond the first page stays discoverable.
@MainActor final class TaskBrowserModel: ObservableObject {
  @Published var jobs: [BackupJob] = []
  @Published var hasMore = false
  @Published var loading = false
  @Published var failed = false
  private var filter = "all"
  private var receiver: String?
  private var limit = 200
  private var generation = 0
  func loadMore() async {
    guard !loading else { return }
    limit += 200
    await refresh(filter: filter, receiver: receiver)
  }
  func refresh(filter: String, receiver: String?) async {
    if self.filter != filter || self.receiver != receiver {
      limit = 200
      jobs = []
      hasMore = false
    }
    self.filter = filter
    self.receiver = receiver
    generation += 1
    let expected = generation
    loading = true
    failed = false
    defer { if generation == expected { loading = false } }
    var result: [BackupJob] = []
    var after: Int64 = 0
    do {
      while result.count < limit {
        var query: [String: Any] = ["op": "jobs", "after": after]
        if filter != "all" { query["state"] = filter }
        if let receiver { query["receiver_id"] = receiver }
        let data = try await Bridge.call(query)
        let page = try JSONDecoder().decode([BackupJob].self, from: data)
        guard expected == generation, !Task.isCancelled else { return }
        result.append(contentsOf: page)
        guard page.count == 200, let last = page.last else { break }
        after = last.id
      }
      jobs = result
      hasMore = result.count >= limit
    } catch {
      guard expected == generation, !Task.isCancelled else { return }
      failed = true
    }
  }
}

/// Keep explanatory text with its content, above the tab bar and outside row hit targets.
struct BackupReceiptExplanation: View {
  var body: some View {
    Text("receipt_explanation")
      .font(.footnote).foregroundStyle(.secondary)
      .lineSpacing(3).fixedSize(horizontal: false, vertical: true)
      .frame(maxWidth: .infinity, alignment: .leading)
      .padding(.vertical, 12)
      .accessibilityIdentifier("backup.receipt_explanation")
  }
}
