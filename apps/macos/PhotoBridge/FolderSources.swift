import AppKit
import AVFoundation
import CoreServices
import ImageIO
import SwiftUI

struct FolderSource: Codable, Identifiable {
  let id: String
  var name: String
  var bookmark: Data
  var automatic: Bool
  var enabled: Bool
  var receiver: String?
  var lastCheck: Date?
  var issues: [String: String] = [:]
  var baselinePending: Bool = false
}
struct FolderSummary: Decodable {
  let files: UInt64
  let bytes: UInt64
  let unsupported: UInt64
  let scanning: Bool
}

@MainActor final class FolderSources: ObservableObject {
  static let shared = FolderSources(backup: .shared)
  @Published var sources: [FolderSource] = []
  @Published var selectedSourceID: String?
  @Published var summaries: [String: FolderSummary] = [:]
  @Published var phases: [String: String] = [:]
  @Published var error: String?
  @Published var revision = 0
  @Published var indexRevision = 0
  private var watches: [String: FolderWatch] = [:]
  private var access: [String: URL] = [:]
  private var dirty = Set<String>()
  private var scan: String?
  private var fullChecks = Set<String>()
  private var changedFolders: [String: Set<String>] = [:]
  private var loop: Task<Void, Never>?
  private var lastTurn = 0
  private var debounce: [String: Date] = [:]
  private let backup: BackupModel
  private var file: URL { backup.root.appendingPathComponent("folder-sources.json") }
  init(backup: BackupModel) { self.backup = backup }
  private func call(_ command: [String: Any]) async throws -> Data {
    try await Bridge.call(["op": "folder", "command": command])
  }
  func open() async {
    guard loop == nil else { return }
    while !backup.ready { try? await Task.sleep(nanoseconds: 500_000_000); if Task.isCancelled { return } }
    do {
      if FileManager.default.fileExists(atPath: file.path) {
        sources = try JSONDecoder().decode([FolderSource].self, from: Data(contentsOf: file))
      }
      for source in sources { check(source.id) }
      loop = Task { [weak self] in
        while !Task.isCancelled {
          await self?.tick()
          try? await Task.sleep(nanoseconds: 1_000_000_000)
        }
      }
    } catch { self.error = error.localizedDescription }
  }
  private func save() {
    do { try JSONEncoder().encode(sources).write(to: file, options: .atomic) }
    catch { self.error = error.localizedDescription }
  }
  func add(_ url: URL, automatic: Bool, existing: Bool) throws {
    let path = url.resolvingSymlinksInPath().standardizedFileURL.path
    let managed = backup.root.resolvingSymlinksInPath().path
    guard !path.hasPrefix(managed + "/"), !managed.hasPrefix(path + "/"), path != managed else {
      throw Bridge.Failure(code: "invalid")
    }
    for source in sources {
      var stale = false
      if let other = try? URL(resolvingBookmarkData: source.bookmark, options: [.withSecurityScope, .withoutUI, .withoutMounting], relativeTo: nil, bookmarkDataIsStale: &stale) {
        let otherPath = other.resolvingSymlinksInPath().standardizedFileURL.path
        guard path != otherPath, !path.hasPrefix(otherPath + "/"), !otherPath.hasPrefix(path + "/") else {
          throw Bridge.Failure(code: "conflict")
        }
      }
    }
    let bookmark = try url.resolvingSymlinksInPath().bookmarkData(options: [.withSecurityScope, .securityScopeAllowOnlyReadAccess], includingResourceValuesForKeys: nil, relativeTo: nil)
    let source = FolderSource(id: UUID().uuidString, name: url.lastPathComponent, bookmark: bookmark,
      automatic: automatic, enabled: existing || automatic, receiver: backup.pairing?.receiverID, lastCheck: nil, baselinePending: !existing)
    sources.append(source); check(source.id); save()
  }
  func check(_ id: String) { dirty.insert(id); fullChecks.insert(id); debounce[id] = .distantPast }
  func wake() {
    watches.removeAll()
    for (id, url) in access { url.stopAccessingSecurityScopedResource(); dirty.insert(id) }
    access.removeAll()
    for source in sources { check(source.id) }
  }
  func setAutomatic(_ id: String, _ value: Bool) {
    guard let index = sources.firstIndex(where: { $0.id == id }) else { return }
    sources[index].automatic = value
    if value { sources[index].enabled = true; check(id) }
    save()
  }
  func start(_ id: String) async {
    guard let i = sources.firstIndex(where: { $0.id == id }) else { return }
    sources[i].enabled = true; sources[i].receiver = backup.pairing?.receiverID
    sources[i].issues.removeAll(); sources[i].baselinePending = false;
    _ = try? await call(["action": "include_existing", "source": id]);
    _ = try? await call(["action": "retry_ignored", "source": id]); check(id); save()
    if backup.pairing != nil { await backup.setPaused(false) }
  }
  func pause(_ id: String) {
    guard let i = sources.firstIndex(where: { $0.id == id }) else { return }
    sources[i].enabled = false; save()
  }
  func remove(_ id: String) async {
    do {
      if scan == id { _ = try await call(["action": "cancel"]); scan = nil }
      if selectedSourceID == id { selectedSourceID = nil }
      sources.removeAll { $0.id == id }; dirty.remove(id); watches[id] = nil
      access.removeValue(forKey: id)?.stopAccessingSecurityScopedResource()
      _ = try await call(["action": "forget", "source": id]); save(); revision += 1
    } catch { self.error = error.localizedDescription }
  }
  private func url(_ source: FolderSource) throws -> URL {
    if let value = access[source.id], FileManager.default.fileExists(atPath: value.path) { return value }
    var stale = false
    let value = try URL(resolvingBookmarkData: source.bookmark, options: [.withSecurityScope, .withoutUI, .withoutMounting], relativeTo: nil, bookmarkDataIsStale: &stale)
    _ = value.startAccessingSecurityScopedResource()
    guard FileManager.default.fileExists(atPath: value.path) else {
      value.stopAccessingSecurityScopedResource(); throw Bridge.Failure(code: "source_unavailable")
    }
    access[source.id]?.stopAccessingSecurityScopedResource(); access[source.id] = value
    watches[source.id] = FolderWatch(url: value) { [weak self] paths, full in
      Task { @MainActor in
        guard let self, self.sources.first(where: { $0.id == source.id })?.automatic == true else { return }
        self.dirty.insert(source.id)
        if full { self.fullChecks.insert(source.id) }
        for path in paths {
          let parent = URL(fileURLWithPath: path).deletingLastPathComponent().path
          if parent == value.path { self.fullChecks.insert(source.id) }
          else if parent.hasPrefix(value.path + "/") { self.changedFolders[source.id, default: []].insert(String(parent.dropFirst(value.path.count + 1))) }
          else { self.fullChecks.insert(source.id) }
        }
        self.debounce[source.id] = Date().addingTimeInterval(3)
      }
    }
    if stale, let i = sources.firstIndex(where: { $0.id == source.id }) {
      sources[i].bookmark = try value.bookmarkData(options: [.withSecurityScope, .securityScopeAllowOnlyReadAccess], includingResourceValuesForKeys: nil, relativeTo: nil); save()
    }
    return value
  }
  private func tick() async {
    guard backup.ready else { return }
    if let id = scan {
      do {
        for _ in 0..<4 {
          let summary = try JSONDecoder().decode(FolderSummary.self, from: await call(["action": "step"]))
          summaries[id] = summary
          if !summary.scanning {
            scan = nil; revision += 1; indexRevision += 1
            if let i = sources.firstIndex(where: { $0.id == id }) {
              sources[i].lastCheck = Date()
              if sources[i].baselinePending {
                _ = try await call(["action": "baseline", "source": id, "receiver": "@baseline"])
                if let current = sources.firstIndex(where: { $0.id == id }) { sources[current].baselinePending = false }
              }
              save()
            }
            break
          }
        }
      } catch {
        phases[id] = "folder_scan_failed"; scan = nil; dirty.insert(id); debounce[id] = Date().addingTimeInterval(60)
      }
    }
    guard !sources.isEmpty else { return }
    lastTurn = (lastTurn + 1) % sources.count
    let source = sources[lastTurn]
    var currentEntry: FolderEntry?
    var companionEntry: FolderEntry?
    do {
      let root = try url(source)
      if scan == nil, dirty.contains(source.id) || (source.automatic && Date().timeIntervalSince(source.lastCheck ?? .distantPast) > (watches[source.id]?.active == true ? 3600 : 60)) {
        if Date() >= (debounce[source.id] ?? .distantPast) {
          let scopes = fullChecks.contains(source.id) || (changedFolders[source.id]?.count ?? 0) > 256 ? [] : Array(changedFolders[source.id] ?? [])
          _ = try await call(["action": "begin", "source": source.id, "root": root.path, "directories": scopes])
          fullChecks.remove(source.id); changedFolders[source.id] = nil
          scan = source.id; dirty.remove(source.id); phases[source.id] = "folder_scanning"
        }
      }
      if scan == source.id { return }
      guard source.enabled else { phases[source.id] = "folder_paused"; return }
      guard let receiver = backup.pairing?.receiverID else { phases[source.id] = "folder_pair_first"; return }
      guard source.receiver == receiver || source.receiver == nil else { phases[source.id] = "folder_receiver_changed"; return }
      if source.receiver == nil, let i = sources.firstIndex(where: { $0.id == source.id }) { sources[i].receiver = receiver; save() }
      if source.baselinePending {
        guard source.lastCheck != nil else { return }
        _ = try await call(["action": "baseline", "source": source.id, "receiver": "@baseline"])
        if let i = sources.firstIndex(where: { $0.id == source.id }) { sources[i].baselinePending = false; save() }
        return
      }
      guard !backup.paused else { phases[source.id] = "backup_paused"; return }
      guard backup.summary.queued + backup.summary.running + backup.summary.waiting < 16 else { phases[source.id] = "folder_queue_wait"; return }
      guard await backup.canPrepareForReceiver() else { phases[source.id] = "waiting_for_wifi"; return }
      let entries = try JSONDecoder().decode([FolderEntry].self, from: await call(["action": "candidates", "source": source.id, "receiver": receiver]))
      if entries.isEmpty, Date().timeIntervalSince(source.lastCheck ?? .distantPast) < 12 { phases[source.id] = "folder_settling"; return }
      guard let entry = entries.first(where: { source.issues[$0.relative] != $0.revision }) else {
        let pendingData = try await call(["action": "pending", "source": source.id, "receiver": receiver])
        let pending = (try JSONSerialization.jsonObject(with: pendingData) as? [String: Int])?["count"] ?? 0
        if pending > 0 { phases[source.id] = "folder_cache_wait"; return }
        phases[source.id] = source.issues.isEmpty ? "folder_up_to_date" : "folder_attention"
        if entries.isEmpty, !source.automatic, Date().timeIntervalSince(source.lastCheck ?? .distantPast) > 12, let i = sources.firstIndex(where: { $0.id == source.id }) { sources[i].enabled = false; save() }
        return
      }
      currentEntry = entry
      phases[source.id] = "folder_preparing"
      let prepared = try await FolderMedia.prepare(root: root, entry: entry)
      var primary = entry
      if prepared.primary != entry.relative {
        primary = try JSONDecoder().decode(FolderEntry.self, from: await call(["action": "entry", "source": source.id, "relative": prepared.primary]))
      }
      currentEntry = primary
      var command: [String: Any] = ["action": "prepare", "source": source.id, "name": source.name,
        "root": root.path, "relative": primary.relative, "source_id": primary.source_id,
        "revision": primary.revision, "receiver": receiver, "metadata": prepared.metadata]
      if let paired = prepared.paired {
        let companion = try JSONDecoder().decode(FolderEntry.self, from: await call(["action": "entry", "source": source.id, "relative": paired]))
        companionEntry = companion
        command["paired"] = [companion.relative, companion.revision]
      }
      // Recheck global/source pause and target after asynchronous media inspection.
      guard !backup.paused, backup.pairing?.receiverID == receiver,
        sources.first(where: { $0.id == source.id })?.enabled == true else { return }
      _ = try await call(command); revision += 1
      if let i = sources.firstIndex(where: { $0.id == source.id }) {
        sources[i].issues.removeValue(forKey: primary.relative)
        if let paired = prepared.paired { sources[i].issues.removeValue(forKey: paired) }
        save()
      }
      await backup.refresh(); await BackgroundTransfer.shared.kick()
    } catch let failure as Bridge.Failure {
      if failure.code == "capacity" {
        for entry in [currentEntry, companionEntry].compactMap({ $0 }) {
          _ = try? await call(["action": "defer", "source": source.id, "relative": entry.relative, "revision": entry.revision])
        }
      }
      if failure.code == "conflict" { dirty.insert(source.id); debounce[source.id] = Date().addingTimeInterval(12) }
      if failure.code == "unsupported" || failure.code == "invalid" { await ignore(source, entry: currentEntry) }
      phases[source.id] = failure.code == "capacity" ? "folder_cache_wait" : failure.code == "source_unavailable" ? "folder_offline" : "folder_attention"
    } catch {
      phases[source.id] = "folder_attention"
      await ignore(source, entry: currentEntry)
      self.error = error.localizedDescription
    }
  }
  private func ignore(_ source: FolderSource, entry: FolderEntry?) async {
    guard let entry, let i = sources.firstIndex(where: { $0.id == source.id }) else { return }
    sources[i].issues[entry.relative] = entry.revision; save()
    _ = try? await call(["action": "ignore", "source": source.id, "relative": entry.relative, "revision": entry.revision])
  }
  func states(_ id: String, relatives: [String]) async throws -> [String: String] {
    try JSONDecoder().decode([String: String].self, from: await call(["action": "states", "source": id, "receiver": backup.pairing?.receiverID ?? "", "relatives": relatives]))
  }
  func page(_ id: String, offset: Int) async throws -> [FolderEntry] {
    try JSONDecoder().decode([FolderEntry].self, from: await call(["action": "page", "source": id, "offset": offset, "receiver": backup.pairing?.receiverID ?? ""]))
  }
}
