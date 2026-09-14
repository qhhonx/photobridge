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
  var activityLabel: String? = nil
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
        if let activityLabel {
          Text(NSLocalizedString(activityLabel, comment: "") + " · " + (job.stateChangedAt.map {
            Date(timeIntervalSince1970: Double($0) / 1000).formatted(date: .abbreviated, time: .standard)
          } ?? NSLocalizedString("task_order_unknown", comment: "")))
            .font(.caption).foregroundStyle(.secondary)
        }
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
  @AppStorage("transferSortChoicesV2") private var savedOrders = "{}"
  private var ordering: TransferOrdering {
    get { (try? JSONDecoder().decode([String: TransferOrdering].self, from: Data(savedOrders.utf8)))?[filter] ?? .recommended(filter) }
    nonmutating set {
      var values = (try? JSONDecoder().decode([String: TransferOrdering].self, from: Data(savedOrders.utf8))) ?? [:]
      values[filter] = newValue
      if let data = try? JSONEncoder().encode(values) { savedOrders = String(decoding: data, as: UTF8.self) }
    }
  }
  init(model: BackupModel, filter: String = "all") {
    self.model = model
    _filter = State(initialValue: filter)
  }
  private var queryID: String {
    "\(filter)|\(ordering.sort)|\(ordering.descending)|\(model.queueRevision)|\(model.pairing?.receiverID ?? "")"
  }
  var body: some View {
    VStack(alignment: .leading, spacing: 20) {
      HStack {
        VStack(alignment: .leading, spacing: 6) {
          Text(LocalizedStringKey(filter == "all" ? "transfer_tasks" : filter == "scanned" ? "task_title_scanned" : "state_" + filter))
            .font(.title2.weight(.medium)).accessibilityIdentifier("transfers.title")
          Text(String(format: NSLocalizedString("task_status_count", comment: ""), model.transferCount(for: filter)))
            .font(.callout).foregroundStyle(.secondary).monospacedDigit()
            .accessibilityIdentifier("transfers.count")
        }
        Spacer()
        Picker("task_filter", selection: $filter) {
          Text("backup_all_tasks").tag("all")
          ForEach(["preparing", "running", "queued", "waiting", "paused", "failed", "received", "scanned"], id: \.self) {
            Text(LocalizedStringKey($0 == "scanned" ? "task_title_scanned" : "state_" + $0)).tag($0)
          }
        }.pickerStyle(.menu).labelsHidden().fixedSize()
          .accessibilityIdentifier("transfers.filter")
      }.frame(maxWidth: .infinity, alignment: .leading)
      Text(LocalizedStringKey("task_explanation_" + filter))
        .font(.callout).foregroundStyle(.secondary)
        .frame(maxWidth: .infinity, alignment: .leading)
        .accessibilityIdentifier("transfers.explanation")
      TransferSortControl(ordering: Binding(get: { ordering }, set: { ordering = $0 }), filter: filter)
      if filter == "preparing" || filter == "scanned" {
        SourceBrowser(model: model, history: filter == "scanned", descending: ordering.descending, sort: ordering.sort)
      } else {
        if browser.hasMore {
          Text(String(format: NSLocalizedString("task_loaded_count", comment: ""), browser.jobs.count))
            .font(.caption).foregroundStyle(.secondary)
        }
        ScrollView {
          LazyVStack(alignment: .leading, spacing: 0) {
            if browser.loading && browser.jobs.isEmpty {
              ProgressView().frame(maxWidth: .infinity).padding(.top, 36)
            } else if browser.failed {
              VStack(spacing: 12) {
                Text("tasks_load_failed").foregroundStyle(.secondary)
                Button("retry_task") {
                  Task { await refresh() }
                }
              }.frame(maxWidth: .infinity).padding(.top, 36)
            } else if browser.jobs.isEmpty {
              ContentUnavailableView(
                "tasks_empty_filtered", systemImage: "line.3.horizontal.decrease.circle"
              ).frame(maxWidth: .infinity).padding(.top, 36)
            }
            ForEach(browser.jobs) { job in
              TransferRow(job: job, progress: model.transferProgress[job.id], activityLabel: ordering.sort == "activity" ? "task_order_by_" + ordering.basis(filter) : nil) {
                Task { await model.retry(job.id) }
              }
              Divider()
            }
            if browser.hasMore {
              Button("load_more_tasks") { Task { await browser.loadMore() } }.padding(.vertical, 20)
            }
          }
        }
      }
    }.frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
    .task(id: queryID) {
      if filter != "preparing" && filter != "scanned" {
        await refresh()
      }
    }
  }
  private func refresh() async {
    let receiver = model.pairing?.receiverID
    await browser.refresh(filter: filter, receiver: receiver, descending: ordering.descending, sort: ordering.sort)
    guard !Task.isCancelled, receiver == model.pairing?.receiverID,
      !browser.failed, let summary = browser.summary else { return }
    model.summary = summary
  }
}

/// Filter before pagination so a failure beyond the first page stays discoverable.
@MainActor final class TaskBrowserModel: ObservableObject {
  @Published var jobs: [BackupJob] = []
  @Published var hasMore = false
  @Published var loading = false
  @Published var failed = false
  private(set) var summary: SenderSummary?
  private var filter = "all"
  private var receiver: String?
  private var descending = false
  private var sort = "added"
  private var limit = 200
  private var generation = 0
  func loadMore() async {
    guard !loading else { return }
    limit += 200
    await refresh(filter: filter, receiver: receiver, descending: descending, sort: sort)
  }
  func refresh(filter: String, receiver: String?, descending: Bool = false, sort: String = "added") async {
    if self.filter != filter || self.receiver != receiver || self.descending != descending || self.sort != sort {
      limit = 200
      jobs = []
      hasMore = false
      summary = nil
    }
    self.descending = descending
    self.sort = sort
    self.filter = filter
    self.receiver = receiver
    generation += 1
    let expected = generation
    guard let receiver else { jobs = []; hasMore = false; loading = false; return }
    loading = true
    failed = false
    defer { if generation == expected { loading = false } }
    var result: [BackupJob] = []
    var after: Int64 = 0
    var afterValue: Int64?
    do {
      while result.count < limit {
        var query: [String: Any] = ["op": "jobs", "after": after, "descending": descending, "sort": sort]
        if let afterValue { query["after_value"] = afterValue }
        if filter != "all" { query["state"] = filter }
        query["receiver_id"] = receiver
        let data = try await Bridge.call(query)
        let page = try JSONDecoder().decode([BackupJob].self, from: data)
        guard expected == generation, !Task.isCancelled else { return }
        result.append(contentsOf: page)
        guard page.count == 200, let last = page.last else { break }
        after = last.id
        afterValue = last.sortValue
      }
      let data = try await Bridge.call(["op": "sender_summary", "receiver_id": receiver])
      let summary = try JSONDecoder().decode(SenderSummary.self, from: data)
      guard expected == generation, !Task.isCancelled else { return }
      self.summary = summary
      if jobs != result { jobs = result }
      hasMore = result.count < summary.count(for: filter)
    } catch {
      guard expected == generation, !Task.isCancelled else { return }
      failed = true
    }
  }
}

/// Compact, always-present status; full messages belong in the detail sheet.
struct BackupStatusIndicator: View {
  @ObservedObject var model: BackupModel
  @State private var details = false
  private var title: String {
    if let message = model.message { return message }
    if model.pairing == nil { return NSLocalizedString("receiver_unpaired", comment: "") }
    if !model.ready { return NSLocalizedString("backup_initializing", comment: "") }
    if model.summary.failed > 0 {
      return String(format: NSLocalizedString("backup_status_failed_count", comment: ""), model.summary.failed)
    }
    if model.waitingForNetwork { return NSLocalizedString("waiting_for_wifi", comment: "") }
    if let reason = model.waitingReason { return NSLocalizedString("error_" + reason, comment: "") }
    return NSLocalizedString(model.paused ? "backup_paused" : "backup_status_ready", comment: "")
  }
  private var status: (key: String, symbol: String, color: Color) {
    if !model.ready { return model.message == nil
      ? ("backup_indicator_starting", "clock", .secondary)
      : ("backup_indicator_attention", "exclamationmark.circle", .orange) }
    if model.pairing == nil { return ("backup_indicator_unpaired", "link", .secondary) }
    if model.paused { return ("backup_indicator_paused", "pause.circle", .secondary) }
    if model.message != nil || model.summary.failed > 0 {
      return ("backup_indicator_attention", "exclamationmark.circle", .orange)
    }
    if model.waitingForNetwork || model.receiverUnavailable {
      return ("backup_indicator_network", "wifi.exclamationmark", .orange)
    }
    if model.summary.running > 0 || model.importing {
      return ("backup_indicator_active", "arrow.up.circle", .accentColor)
    }
    if model.waitingReason != nil { return ("backup_indicator_retry", "clock", .orange) }
    if model.pendingImports > 0 || model.summary.queued > 0 {
      return ("backup_indicator_waiting", "clock", .secondary)
    }
    return ("backup_indicator_ready", "checkmark.circle", .green)
  }
  var body: some View {
    Button { details = true } label: {
      HStack(spacing: 6) {
        Image(systemName: status.symbol)
        Text(LocalizedStringKey(status.key)).lineLimit(1)
        Image(systemName: "chevron.right").font(.system(size: 9, weight: .medium))
      }.font(.subheadline).foregroundStyle(status.color)
        .padding(.horizontal, 10).padding(.vertical, 6)
        .background(status.color.opacity(0.09), in: Capsule())
        .fixedSize(horizontal: true, vertical: false)
        .frame(height: 44).contentShape(Rectangle())
    }.buttonStyle(.plain).accessibilityIdentifier("backup.status_indicator")
      .accessibilityHint(Text("backup_status_details"))
      .sheet(isPresented: $details) {
        VStack(alignment: .leading, spacing: 20) {
          HStack {
            Text("backup_status_title").font(.title2)
            Spacer()
            Button("backup_status_close") { details = false }
          }
          ScrollView {
            VStack(alignment: .leading, spacing: 16) {
              Label(LocalizedStringKey(status.key), systemImage: status.symbol).foregroundStyle(status.color)
              Text(title).textSelection(.enabled)
              Text(String(format: NSLocalizedString("transfer_summary", comment: ""),
                model.summary.received, model.summary.total)).foregroundStyle(.secondary)
              if model.waitingForNetwork || model.receiverUnavailable { Text("waiting_for_wifi") }
              WaitingStatus(model: model)
            }.frame(maxWidth: .infinity, alignment: .leading)
          }
        }.padding(24).frame(minWidth: 280, idealWidth: 440, minHeight: 280)
      }
  }
}


/// Pairing has its own messages; unrelated transfer errors never appear here.
struct ReceiverStatusIndicator: View {
  @ObservedObject var model: BackupModel
  @State private var details = false
  private var status: (key: String, symbol: String, color: Color) {
    if model.pairingInProgress { return ("receiver_indicator_pairing", "link", .accentColor) }
    if model.pairingError != nil { return ("receiver_indicator_attention", "exclamationmark.circle", .orange) }
    if model.pairing == nil { return ("backup_indicator_unpaired", "link", .secondary) }
    if model.receiverUnavailable || model.waitingForNetwork {
      return ("receiver_indicator_waiting", "wifi.exclamationmark", .orange)
    }
    return ("receiver_indicator_paired", "link", .accentColor)
  }
  var body: some View {
    Button { details = true } label: {
      HStack(spacing: 6) {
        Image(systemName: status.symbol)
        Text(LocalizedStringKey(status.key)).lineLimit(1)
        Image(systemName: "chevron.right").font(.system(size: 9, weight: .medium))
      }.font(.subheadline).foregroundStyle(status.color)
        .padding(.horizontal, 10).padding(.vertical, 6)
        .background(status.color.opacity(0.09), in: Capsule())
        .fixedSize(horizontal: true, vertical: false)
        .frame(height: 44).contentShape(Rectangle())
    }.buttonStyle(.plain).accessibilityIdentifier("receiver.status_indicator")
      .accessibilityHint(Text("backup_status_details"))
      .sheet(isPresented: $details) {
        VStack(alignment: .leading, spacing: 20) {
          HStack {
            Text("receiver_connection_details").font(.title2)
            Spacer()
            Button("backup_status_close") { details = false }
          }
          ScrollView {
            VStack(alignment: .leading, spacing: 16) {
              Label(LocalizedStringKey(status.key), systemImage: status.symbol).foregroundStyle(status.color)
              if let error = model.pairingError { Text(error).textSelection(.enabled) }
              Text(model.pairing == nil ? "receiver_pair_instructions" : "receiver_indicator_explanation")
              if let pairing = model.pairing {
                LabeledContent("receiver_address", value: pairing.endpoint).textSelection(.enabled)
              }
              Text("receiver_local_network").foregroundStyle(.secondary)
            }.frame(maxWidth: .infinity, alignment: .leading)
          }
        }.padding(24).frame(minWidth: 280, idealWidth: 440, minHeight: 280)
      }
  }
}


struct TransferOrdering: Codable, Equatable {
  var sort: String
  var descending: Bool
  static func recommended(_ filter: String) -> Self {
    switch filter {
    case "waiting": return Self(sort: "retry", descending: false)
    case "running": return Self(sort: "activity", descending: false)
    case "failed": return Self(sort: "activity", descending: true)
    case "all": return Self(sort: "added", descending: true)
    default: return Self(sort: "capture", descending: true)
    }
  }
  static func options(_ filter: String) -> [String] {
    ["capture", "added"] + (["running", "received", "failed"].contains(filter) ? ["activity"] : filter == "waiting" ? ["retry"] : [])
  }
  func basis(_ filter: String) -> String {
    if sort == "activity" { return filter == "received" ? "received" : filter == "failed" ? "failed" : "started" }
    if sort == "added" && filter == "scanned" { return "scan" }
    return sort
  }
}

struct TransferSortControl: View {
  @Binding var ordering: TransferOrdering
  let filter: String
  var body: some View {
    VStack(alignment: .leading, spacing: 5) {
      HStack(spacing: 12) {
        Text(LocalizedStringKey("task_order_" + ordering.basis(filter) + (ordering.descending ? "_desc" : "_asc")))
          .font(.subheadline).foregroundStyle(.secondary)
          .accessibilityIdentifier("transfers.sort_description")
        Spacer(minLength: 0)
        Menu {
          ForEach(TransferOrdering.options(filter), id: \.self) { value in
            Button {
              ordering = TransferOrdering(sort: value, descending: value != "retry")
            } label: {
              Label(LocalizedStringKey("task_order_by_" + TransferOrdering(sort: value, descending: false).basis(filter)), systemImage: ordering.sort == value ? "checkmark" : "line.3.horizontal.decrease")
            }.accessibilityIdentifier("transfers.sort_by_" + value)
          }
          Divider()
          Button { ordering.descending = false } label: {
            Label("task_sort_ascending", systemImage: ordering.descending ? "arrow.up" : "checkmark")
          }.accessibilityIdentifier("transfers.sort_ascending")
          Button { ordering.descending = true } label: {
            Label("task_sort_descending", systemImage: ordering.descending ? "checkmark" : "arrow.down")
          }.accessibilityIdentifier("transfers.sort_descending")
          Divider()
          Button("task_order_recommended") { ordering = .recommended(filter) }
            .accessibilityIdentifier("transfers.sort_recommended")
        } label: {
          Label("task_order_button", systemImage: "arrow.up.arrow.down")
        }.fixedSize().accessibilityIdentifier("transfers.sort")
      }
      Text(LocalizedStringKey("task_order_hint_" + filter)).font(.caption).foregroundStyle(.secondary)
    }.frame(maxWidth: .infinity, alignment: .leading)
  }
}
