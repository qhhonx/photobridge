import Foundation
import Photos
import SwiftUI

struct StorageSettings: Codable {
  var cache_budget_bytes: UInt64
  var receiver_budget_bytes: UInt64
  var min_free_bytes: UInt64
  var auto_reclaim: Bool
  var log_days: Int
  var log_limit: Int
}
struct StorageSnapshot: Decodable {
  var settings: StorageSettings
  let used_bytes: UInt64
  let free_bytes: UInt64
  let export_allowance: UInt64
  let reason: String?
  var limitingReason: String {
    free_bytes > settings.min_free_bytes
      && free_bytes - settings.min_free_bytes > settings.cache_budget_bytes
        - min(used_bytes, settings.cache_budget_bytes)
      ? "local_cache_budget" : "local_free_space"
  }
}
enum StoragePreferenceScope { case all, cache, logs }

struct StoragePreferences: View {
  @ObservedObject var model: BackupModel
  var scope: StoragePreferenceScope = .all
  @State private var logs = false
  var body: some View {
    controls
      .buttonStyle(.borderless)
      .pickerStyle(.menu)
      .disabled(model.savingStorage)
  }
  @ViewBuilder private var controls: some View {
    #if os(macOS)
      VStack(alignment: .leading, spacing: 14) { rows }
    #else
      // A Form must see separate rows. A VStack combines all controls into one
      // row and automatic picker/button actions can capture unrelated taps.
      rows
    #endif
  }
  @ViewBuilder private var rows: some View {
    Group {
      Text(
        scope == .cache
          ? "settings_cache" : scope == .logs ? "settings_log_retention" : "storage_settings"
      ).font(.headline).task { await model.refreshStorage() }
      if let snapshot = model.storage {
        if scope != .logs {
          LabeledContent("storage_cache_used", value: bytes(snapshot.used_bytes))
          LabeledContent("storage_disk_free", value: bytes(snapshot.free_bytes))
          Picker(
            "storage_cache_budget",
            selection: Binding(
              get: { Int(snapshot.settings.cache_budget_bytes >> 30) },
              set: { value in
                var settings = snapshot.settings
                settings.cache_budget_bytes = UInt64(value) << 30
                Task { await model.saveStorage(settings) }
              })
          ) { ForEach([1, 2, 5, 10, 20, 50], id: \.self) { Text("\($0) GB").tag($0) } }
          .accessibilityIdentifier("settings.cache_budget")
          Picker(
            "storage_min_free",
            selection: Binding(
              get: { Int(snapshot.settings.min_free_bytes >> 30) },
              set: { value in
                var settings = snapshot.settings
                settings.min_free_bytes = UInt64(value) << 30
                Task { await model.saveStorage(settings) }
              })
          ) { ForEach([1, 2, 5, 10], id: \.self) { Text("\($0) GB").tag($0) } }
          Toggle(
            "storage_auto_reclaim",
            isOn: Binding(
              get: { snapshot.settings.auto_reclaim },
              set: { value in
                var settings = snapshot.settings
                settings.auto_reclaim = value
                Task { await model.saveStorage(settings) }
              }))
          Text("storage_reclaim_note").font(.footnote).foregroundStyle(.secondary)
          Button("storage_reclaim_now") { Task { await model.reclaimCache() } }
            .accessibilityIdentifier("settings.reclaim_cache")
        }
        if scope == .all { Divider() }
        if scope != .cache {
          Picker(
            "logs_retention",
            selection: Binding(
              get: { snapshot.settings.log_days },
              set: { value in
                var settings = snapshot.settings
                settings.log_days = value
                Task { await model.saveStorage(settings) }
              })
          ) {
            ForEach([1, 7, 14, 30, 90], id: \.self) {
              Text(String(format: NSLocalizedString("logs_days", comment: ""), $0)).tag($0)
            }
          }
          Picker(
            "logs_limit",
            selection: Binding(
              get: { snapshot.settings.log_limit },
              set: { value in
                var settings = snapshot.settings
                settings.log_limit = value
                Task { await model.saveStorage(settings) }
              })
          ) { ForEach([1000, 5000, 10000, 20000], id: \.self) { Text("\($0)").tag($0) } }
        }
      } else if let error = model.storageError {
        Text(error).foregroundStyle(.orange)
        Button("retry_task") { Task { await model.refreshStorage() } }
      } else {
        ProgressView(model.ready ? "storage_loading" : "backup_initializing")
          .controlSize(.small)
      }
      if model.storage != nil, let error = model.storageError {
        Text(error).foregroundStyle(.orange)
      }
      if model.savingStorage { ProgressView("settings_saving").controlSize(.small) }
      if scope == .all {
        Button {
          logs = true
        } label: {
          Label("activity_log", systemImage: "list.bullet.rectangle")
        }.accessibilityIdentifier("settings.activity_log")
          .sheet(isPresented: $logs) { ActivityLogView() }
      }
    }
  }
  private func bytes(_ value: UInt64) -> String {
    ByteCountFormatter.string(fromByteCount: Int64(value), countStyle: .file)
  }
}
struct ActivityEntry: Decodable {
  let timestamp: Int64
  let code: String
  let job_id: Int64?
  let bytes: Int64?
  let context: ActivityContext?
}
struct ActivityContext: Decodable {
  let queued: Int?
  let running: Int?
  let waiting: Int?
  let failed: Int?
  let pending_imports: Int?
  let discovery_pending: Int?
  let paused: Bool?
  let active_requests: Int?
  let next_retry_at: Int64?
  let http_status: Int?
  let system_error: Int?
  let bytes_sent: Int64?
  let bytes_expected: Int64?
  let execution: String?
  let phase: String?
  let reason: String?
}
private struct ActivityContextView: View {
  let context: ActivityContext
  @State private var expanded = false
  var body: some View {
    VStack(alignment: .leading, spacing: 8) {
      // A DisclosureGroup inside a macOS List can become an outline-row
      // heading with no usable disclosure control. Keep this action explicit.
      Button { expanded.toggle() } label: {
        Label("logs_details", systemImage: expanded ? "chevron.down" : "chevron.right")
          .contentShape(Rectangle())
      }.buttonStyle(.borderless)
        .accessibilityIdentifier("activity.details")
        .accessibilityValue(Text(expanded ? "logs_details_expanded" : "logs_details_collapsed"))
      if expanded {
      VStack(alignment: .leading, spacing: 5) {
        if let execution = context.execution {
          Text(LocalizedStringKey("logs_execution_" + execution))
        }
        if let reason = context.reason {
          Text(LocalizedStringKey("logs_reason_" + reason))
        }
        if let queued = context.queued, let running = context.running,
          let waiting = context.waiting, let failed = context.failed {
          Text(String(format: NSLocalizedString("logs_queue_counts", comment: ""), queued, running, waiting, failed))
        }
        if let pending = context.pending_imports, let discovery = context.discovery_pending {
          Text(String(format: NSLocalizedString("logs_import_counts", comment: ""), pending, discovery))
        }
        if let paused = context.paused {
          Text(paused ? "logs_backup_paused" : "logs_backup_enabled")
        }
        if let active = context.active_requests {
          Text(String(format: NSLocalizedString("logs_active_requests", comment: ""), active))
        }
        if let phase = context.phase { Text(LocalizedStringKey("logs_phase_" + phase)) }
        if let sent = context.bytes_sent, let expected = context.bytes_expected {
          Text(String(format: NSLocalizedString("logs_request_bytes", comment: ""),
            ByteCountFormatter.string(fromByteCount: sent, countStyle: .file),
            ByteCountFormatter.string(fromByteCount: expected, countStyle: .file)))
        }
        if let retry = context.next_retry_at {
          Text("logs_next_retry") + Text(" ") + Text(Date(timeIntervalSince1970: Double(retry)), format: .dateTime)
        }
        if let status = context.http_status, status > 0 {
          Text(String(format: NSLocalizedString("logs_http_status", comment: ""), status))
        }
        if let error = context.system_error {
          Text(String(format: NSLocalizedString("logs_system_error", comment: ""), error))
        }
      }.frame(maxWidth: .infinity, alignment: .leading)
      }
    }.font(.caption).foregroundStyle(.secondary)
  }
}
struct ActivityLogView: View {
  @Environment(\.dismiss) private var dismiss
  @State private var entries: [ActivityEntry] = []
  @State private var export: URL?
  @State private var error = false
  var body: some View {
    NavigationStack {
      VStack(alignment: .leading, spacing: 12) {
        Text("logs_privacy_note").font(.footnote).foregroundStyle(.secondary)
        if error { Text("logs_load_failed").foregroundStyle(.orange) }
        List(Array(entries.enumerated()), id: \.offset) { _, entry in
          VStack(alignment: .leading, spacing: 5) {
            Text(Date(timeIntervalSince1970: Double(entry.timestamp)), format: .dateTime.year().month().day().hour().minute().second()).font(
              .caption
            ).foregroundStyle(.secondary)
            Text(LocalizedStringKey("event_" + entry.code))
            if let job = entry.job_id {
              Text(String(format: NSLocalizedString("logs_job_id", comment: ""), job)).font(
                .caption
              ).foregroundStyle(.secondary)
            }
            if let amount = entry.bytes {
              Text(ByteCountFormatter.string(fromByteCount: amount, countStyle: .file)).font(
                .caption
              ).foregroundStyle(.secondary)
            }
            if let context = entry.context { ActivityContextView(context: context) }
          }.frame(maxWidth: .infinity, alignment: .leading).textSelection(.enabled)
        }.listStyle(.plain)
      }.padding().navigationTitle("activity_log")
        .toolbar {
          ToolbarItem(placement: .confirmationAction) {
            Button("settings_done") { dismiss() }.accessibilityIdentifier("activity.done")
          }
          ToolbarItem(placement: .cancellationAction) {
            if let export {
              ShareLink("logs_export", item: export).accessibilityIdentifier("activity.export")
            } else {
              Button("logs_export") {}.disabled(true).accessibilityIdentifier("activity.export")
            }
          }
        }
    }
    #if os(macOS)
      .frame(width: 680, height: 540)
    #endif
    .task {
      do {
        let data = try await Bridge.call(["op": "activity_log", "receiver": false])
        entries = try JSONDecoder().decode([ActivityEntry].self, from: data)
        // A second window/export must not overwrite a report already being shared.
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent(
          "PhotoBridge-diagnostics-" + UUID().uuidString, isDirectory: true)
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        let url = directory.appendingPathComponent("PhotoBridge-diagnostics.json")
        try data.write(to: url, options: .atomic)
        export = url
      } catch { self.error = true }
    }
  }
}

/// PhotoKit streams into an app-owned file; enforce the Rust-provided budget
/// before writing each chunk, without buffering the complete original in memory.
final class BoundedExportWriter: @unchecked Sendable {
  private let lock = NSLock()
  private let file: FileHandle
  private let allowance: UInt64
  private let reason: String
  private let path: String
  private let reserve: UInt64
  private var written: UInt64 = 0
  private var checked: UInt64 = 0
  private var failure: Error?
  private var request: PHAssetResourceDataRequestID?
  init(url: URL, snapshot: StorageSnapshot) throws {
    FileManager.default.createFile(atPath: url.path, contents: nil)
    file = try FileHandle(forWritingTo: url)
    path = url.deletingLastPathComponent().path
    allowance = snapshot.export_allowance
    reason = snapshot.limitingReason
    reserve = snapshot.settings.min_free_bytes
  }
  func bind(_ id: PHAssetResourceDataRequestID) {
    lock.lock()
    request = id
    let cancelled = failure != nil
    lock.unlock()
    if cancelled { PHAssetResourceManager.default().cancelDataRequest(id) }
  }
  func consume(_ data: Data) {
    lock.lock()
    if failure == nil {
      do {
        guard UInt64(data.count) <= allowance - min(written, allowance) else {
          throw Bridge.Failure(code: reason)
        }
        if written - checked >= 4 << 20 || written == 0 {
          let attributes = try FileManager.default.attributesOfFileSystem(forPath: path)
          let free = (attributes[.systemFreeSize] as? NSNumber)?.uint64Value ?? 0
          guard free > reserve + UInt64(data.count) + (8 << 20) else {
            throw Bridge.Failure(code: "local_free_space")
          }
          checked = written
        }
        try file.write(contentsOf: data)
        written += UInt64(data.count)
      } catch { failure = error }
    }
    let cancel = failure != nil ? request : nil
    lock.unlock()
    if let cancel { PHAssetResourceManager.default().cancelDataRequest(cancel) }
  }
  func cancel() {
    lock.lock()
    failure = CancellationError()
    let id = request
    lock.unlock()
    if let id { PHAssetResourceManager.default().cancelDataRequest(id) }
  }
  func finish(_ error: Error?) -> Error? {
    lock.lock()
    defer { lock.unlock() }
    do { try file.close() } catch { if failure == nil { failure = error } }
    return failure ?? error
  }
}

struct WaitingStatus: View {
  @ObservedObject var model: BackupModel
  var body: some View {
    VStack(alignment: .leading, spacing: 5) {
      if let reason = model.waitingReason {
        Label(LocalizedStringKey("error_" + reason), systemImage: "clock").foregroundStyle(.orange)
        if let retry = model.summary.next_retry_at, retry > Int64(Date().timeIntervalSince1970) {
          Text(
            String(
              format: NSLocalizedString("retry_scheduled", comment: ""),
              Date(timeIntervalSince1970: Double(retry)).formatted(date: .omitted, time: .shortened)
            ))
        }
      }
      if model.pendingImports > 0 {
        Text(
          String(format: NSLocalizedString("imports_pending", comment: ""), model.pendingImports))
      }
    }.font(.caption)
  }
}
