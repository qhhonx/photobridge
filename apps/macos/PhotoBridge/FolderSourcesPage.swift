import AppKit
import SwiftUI

struct FolderSourcesPage: View {
  @ObservedObject var folders: FolderSources
  @ObservedObject var backup: BackupModel
  var library: () -> Void
  private var selection: String? {
    get { folders.selectedSourceID }
    nonmutating set { folders.selectedSourceID = newValue }
  }
  @State private var adding: URL?
  @State private var removing: String?
  var body: some View {
    VStack(alignment: .leading, spacing: 20) {
      HStack {
        VStack(alignment: .leading, spacing: 6) {
          Text("sources_heading").font(.title2.weight(.medium))
          Text("sources_description").foregroundStyle(.secondary)
        }
        Spacer()
        Button { choose() } label: { Label("folder_add", systemImage: "folder.badge.plus") }
          .disabled(!backup.ready)
      }
      if let selection, let source = folders.sources.first(where: { $0.id == selection }) {
        Button { self.selection = nil } label: { Label("sources_back", systemImage: "chevron.left") }
        FolderSourceDetail(folders: folders, backup: backup, source: source).id(source.id)
      } else {
        ScrollView {
          VStack(alignment: .leading, spacing: 14) {
            HStack(spacing: 14) {
              Image(systemName: "photo.on.rectangle").font(.title2).foregroundStyle(.tint)
              VStack(alignment: .leading, spacing: 5) {
                Text("source_system_library").font(.headline)
                Text("source_system_description").font(.callout).foregroundStyle(.secondary)
              }
              Spacer()
              Button("nav_library", action: library)
            }.padding(18).background(.quaternary.opacity(0.35), in: RoundedRectangle(cornerRadius: 12))
            ForEach(folders.sources) { source in
              FolderSourceCard(folders: folders, backup: backup, source: source,
                open: { selection = source.id }, remove: { removing = source.id })
            }
            Text("folder_readonly_note").font(.caption).foregroundStyle(.secondary).padding(.top, 8)
          }
        }
      }
      if let error = folders.error { Text(error).font(.caption).foregroundStyle(.orange) }
    }.padding(28).frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
      .sheet(isPresented: Binding(get: { adding != nil }, set: { if !$0 { adding = nil } })) {
        if let adding { AddFolderSheet(url: adding, folders: folders) { self.adding = nil } }
      }
      .confirmationDialog("folder_remove", isPresented: Binding(get: { removing != nil }, set: { if !$0 { removing = nil } })) {
        Button("folder_remove", role: .destructive) { if let removing { Task { await folders.remove(removing) }; self.removing = nil } }
      } message: { Text("folder_remove_note") }
  }
  private func choose() {
    let panel = NSOpenPanel(); panel.canChooseDirectories = true; panel.canChooseFiles = false
    panel.allowsMultipleSelection = false; panel.prompt = NSLocalizedString("folder_choose", comment: "")
    panel.begin { result in if result == .OK { adding = panel.url } }
  }
}
private struct FolderSourceCard: View {
  @ObservedObject var folders: FolderSources
  @ObservedObject var backup: BackupModel
  let source: FolderSource
  let open: () -> Void
  let remove: () -> Void
  var body: some View {
    VStack(alignment: .leading, spacing: 16) {
      HStack(alignment: .top, spacing: 14) {
        Image(systemName: "folder").font(.title2).foregroundStyle(.tint).frame(width: 30)
        VStack(alignment: .leading, spacing: 6) {
          Text(String(format: NSLocalizedString("folder_source_title", comment: ""), folders.displayName(source))).font(.headline).lineLimit(1)
          Text(folders.displayPath(source)).font(.caption).foregroundStyle(.secondary)
            .lineLimit(1).truncationMode(.middle).help(folders.displayPath(source))
          Text(LocalizedStringKey(folders.phases[source.id] ?? "folder_scanning"))
            .font(.callout).foregroundStyle(.secondary)
          if let summary = folders.summaries[source.id] {
            Text(String(format: NSLocalizedString("folder_count", comment: ""), summary.files,
              ByteCountFormatter.string(fromByteCount: Int64(summary.bytes), countStyle: .file)))
              .font(.caption).foregroundStyle(.secondary)
            if summary.unsupported > 0 {
              Text(String(format: NSLocalizedString("folder_unsupported", comment: ""), summary.unsupported))
                .font(.caption).foregroundStyle(.secondary)
            }
          }
          if !source.issues.isEmpty {
            Button { open() } label: {
              Text(String(format: NSLocalizedString("folder_issues", comment: ""), source.issues.count))
            }.buttonStyle(.link).foregroundStyle(.orange)
          }
          if let checked = source.lastCheck {
            Text(NSLocalizedString("folder_last_check", comment: "") + " " + checked.formatted(date: .abbreviated, time: .shortened))
              .font(.caption).foregroundStyle(.secondary)
          }
        }.frame(maxWidth: .infinity, alignment: .leading)
        Menu {
          Button("folder_reveal") {
            if let url = folders.sourceURL(source) { NSWorkspace.shared.activateFileViewerSelecting([url]) }
          }
          Divider()
          Button("folder_remove", role: .destructive, action: remove)
        } label: { Label("folder_more", systemImage: "ellipsis") }
          .menuStyle(.borderlessButton).menuIndicator(.hidden).fixedSize()
          .help("folder_more")
      }
      FolderSourceActions(folders: folders, backup: backup, source: source, open: open)
      Divider()
      HStack(spacing: 16) {
        VStack(alignment: .leading, spacing: 4) {
          Text("folder_automatic").font(.callout)
          Text("folder_automatic_hint").font(.caption).foregroundStyle(.secondary)
        }
        Spacer(minLength: 12)
        Toggle("folder_automatic", isOn: Binding(get: { source.automatic },
          set: { folders.setAutomatic(source.id, $0) }))
          .labelsHidden().toggleStyle(.switch).controlSize(.small)
      }
    }.padding(18).frame(maxWidth: .infinity, alignment: .leading)
      .background(.quaternary.opacity(0.35), in: RoundedRectangle(cornerRadius: 12))
  }
}
private struct FolderSourceActions: View {
  @ObservedObject var folders: FolderSources
  @ObservedObject var backup: BackupModel
  let source: FolderSource
  var open: (() -> Void)? = nil
  var body: some View {
    VStack(alignment: .leading, spacing: 8) {
      ViewThatFits(in: .horizontal) {
        HStack(spacing: 12) { buttons }
        VStack(alignment: .leading, spacing: 10) { buttons }
      }
      if let message = folders.actionMessages[source.id] {
        Text(LocalizedStringKey(message)).font(.caption).foregroundStyle(.secondary)
          .accessibilityIdentifier("folder.action_feedback")
      }
      if backup.pairing == nil { Text("folder_pair_first").font(.caption).foregroundStyle(.secondary) }
    }
  }
  @ViewBuilder private var buttons: some View {
    if let open {
      Button(action: open) { Label("folder_open", systemImage: "folder") }
    }
    Button { folders.check(source.id, userInitiated: true) } label: {
      Label("folder_check", systemImage: "arrow.clockwise")
    }.disabled(!backup.ready || folders.starting.contains(source.id))
    Button { Task { await folders.start(source.id) } } label: {
      Label(folders.starting.contains(source.id) ? "folder_starting" : "folder_backup_now", systemImage: "arrow.up.circle")
    }.disabled(!backup.ready || backup.pairing == nil || folders.starting.contains(source.id))
    if source.enabled {
      Button { folders.pause(source.id) } label: {
        Label("folder_stop_preparing", systemImage: "pause.circle")
      }.disabled(folders.starting.contains(source.id))
    }
  }
}
private struct AddFolderSheet: View {
  let url: URL
  @ObservedObject var folders: FolderSources
  var close: () -> Void
  @State private var automatic = true
  @State private var existing = true
  @State private var error: String?
  var body: some View {
    VStack(alignment: .leading, spacing: 18) {
      Label("folder_add", systemImage: "folder.badge.plus").font(.title2.weight(.medium))
      Text(url.lastPathComponent).font(.headline)
      Text(url.path).font(.caption).foregroundStyle(.secondary).textSelection(.enabled)
      Text("folder_scan_note").foregroundStyle(.secondary)
      Toggle("folder_include_existing", isOn: $existing)
      Toggle("folder_automatic", isOn: $automatic)
      Text("folder_readonly_note").font(.caption).foregroundStyle(.secondary)
      if let error { Text(error).foregroundStyle(.orange) }
      HStack {
        Button("cancel", action: close)
        Spacer()
        Button("folder_add") {
          do { try folders.add(url, automatic: automatic, existing: existing); close() }
          catch { self.error = NSLocalizedString("folder_overlap_error", comment: "") }
        }.buttonStyle(.borderedProminent).disabled(!existing && !automatic)
      }
    }.padding(28).frame(width: 480)
  }
}
private struct FolderSourceDetail: View {
  @ObservedObject var folders: FolderSources
  @ObservedObject var backup: BackupModel
  let source: FolderSource
  @State private var entries: [FolderEntry] = []
  @State private var more = true
  @State private var loading = false
  @State private var error: String?
  @State private var visible = Set<String>()
  @State private var showIssues = false
  @AppStorage("macFolderListLayout") private var layout = "flat"
  @StateObject private var tree = FolderTreeModel()
  var body: some View {
    VStack(alignment: .leading, spacing: 14) {
      HStack {
        VStack(alignment: .leading, spacing: 6) {
          Text(folders.displayName(source)).font(.title3.weight(.medium))
          Text(folders.displayPath(source)).font(.caption).foregroundStyle(.secondary)
            .lineLimit(1).truncationMode(.middle)
          Text(LocalizedStringKey(folders.phases[source.id] ?? "folder_scanning")).foregroundStyle(.secondary)
        }
        Spacer()
      }
      FolderSourceActions(folders: folders, backup: backup, source: source)
      if !source.issues.isEmpty {
        Button { showIssues.toggle() } label: {
          Label("folder_issue_details", systemImage: showIssues ? "chevron.down" : "chevron.right")
        }.buttonStyle(.plain).accessibilityIdentifier("folder.issues")
        if showIssues {
          ScrollView {
            VStack(alignment: .leading, spacing: 8) {
              ForEach(source.issues.keys.sorted(), id: \.self) { path in
                Text(path).font(.caption).foregroundStyle(.secondary).lineLimit(2)
                  .truncationMode(.middle).help(path).textSelection(.enabled)
                  .frame(maxWidth: .infinity, alignment: .leading)
              }
            }
          }.frame(maxHeight: 140)
        }
      }
      HStack {
        Text(layout == "folders" ? "folder_tree_note" : "folder_detail_note")
          .font(.caption).foregroundStyle(.secondary)
        Spacer()
        Picker("folder_list_layout", selection: $layout) {
          Text("folder_list_flat").tag("flat")
          Text("folder_list_tree").tag("folders")
        }.pickerStyle(.segmented).labelsHidden().frame(width: 180)
      }
      ScrollView {
        LazyVStack(alignment: .leading, spacing: 0) {
          if layout == "folders" {
            ForEach(tree.rows) { row in
              VStack(alignment: .leading, spacing: 0) {
                if let child = row.child {
                  if child.is_directory {
                    Button {
                      Task { await tree.toggle(child.relative, source: source.id, folders: folders) }
                    } label: {
                      HStack(spacing: 10) {
                        Image(systemName: tree.expanded.contains(child.relative) ? "chevron.down" : "chevron.right").frame(width: 12)
                        Image(systemName: "folder").foregroundStyle(.tint)
                        Text((child.relative as NSString).lastPathComponent).lineLimit(1)
                        Spacer()
                      }.padding(.vertical, 12).contentShape(Rectangle())
                    }.buttonStyle(.plain).help(child.relative)
                  } else if let entry = child.entry {
                    FolderFileRow(source: source, entry: entry, nested: true)
                      .onAppear { visible.insert(entry.relative) }
                      .onDisappear { visible.remove(entry.relative) }
                  }
                } else if tree.loading.contains(row.directory) {
                  ProgressView().padding(.vertical, 12)
                } else {
                  if let message = tree.errors[row.directory] { Text(message).font(.caption).foregroundStyle(.orange) }
                  Button(tree.errors[row.directory] == nil ? "folder_load_more" : "retry_task") {
                    Task { await tree.load(row.directory, source: source.id, folders: folders) }
                  }.padding(.vertical, 12)
                }
                Divider()
              }.padding(.leading, CGFloat(min(row.depth, 10) * 18))
            }
            if tree.rows.isEmpty { Text("folder_empty").foregroundStyle(.secondary).padding(.vertical, 24) }
          } else {
            ForEach(entries) { entry in
              FolderFileRow(source: source, entry: entry)
                .onAppear { visible.insert(entry.relative) }
                .onDisappear { visible.remove(entry.relative) }
              Divider()
            }
            if more { Button("folder_load_more") { Task { await load() } }.padding(.vertical, 16).disabled(loading) }
            if entries.isEmpty {
              if loading { ProgressView().padding(.vertical, 24) }
              else { Text("folder_empty").foregroundStyle(.secondary).padding(.vertical, 24) }
            }
          }
        }
      }
      if let error { Text(error).foregroundStyle(.orange).font(.caption) }
    }
    .task(id: "\(source.id)|\(folders.indexRevision)|\(layout)") {
      if layout == "folders" { await tree.refresh(source: source.id, folders: folders) }
      else { await reload() }
    }
    .task(id: "\(folders.revision)|\(backup.queueRevision)|\(visible.sorted().joined(separator: "|"))") {
      try? await Task.sleep(nanoseconds: 150_000_000)
      guard !Task.isCancelled else { return }
      if let states = try? await folders.states(source.id, relatives: Array(visible.prefix(400))) {
        guard !Task.isCancelled else { return }
        for i in entries.indices where visible.contains(entries[i].relative) { entries[i].state = states[entries[i].relative] }
        tree.apply(states, visible: visible)
      }
    }
  }
  private func load() async {
    guard !loading else { return }; loading = true; defer { loading = false }
    do { let page = try await folders.page(source.id, offset: entries.count); entries.append(contentsOf: page); more = page.count == 100 }
    catch { self.error = error.localizedDescription }
  }
  private func reload() async {
    guard !loading else { return }; loading = true; defer { loading = false }
    do {
      var result: [FolderEntry] = []
      for offset in stride(from: 0, to: max(100, entries.count), by: 100) {
        let page = try await folders.page(source.id, offset: offset); result.append(contentsOf: page)
        if page.count < 100 { break }
      }
      entries = result; more = result.count > 0 && result.count % 100 == 0
    } catch { self.error = error.localizedDescription }
  }
}

struct FolderChild: Decodable, Identifiable {
  let relative: String
  let is_directory: Bool
  var entry: FolderEntry?
  var id: String { relative }
}
struct FolderChildren: Decodable {
  var rows: [FolderChild]
  let has_more: Bool
}
@MainActor private final class FolderTreeModel: ObservableObject {
  struct Row: Identifiable {
    let child: FolderChild?
    let directory: String
    let depth: Int
    var id: String { child.map { "child:" + $0.relative } ?? "page:" + directory }
  }
  @Published var children: [String: [FolderChild]] = [:]
  @Published var expanded = Set<String>()
  @Published var more = Set<String>()
  @Published var loading = Set<String>()
  @Published var errors: [String: String] = [:]
  private var generation = 0
  var rows: [Row] {
    var result: [Row] = []
    func visit(_ directory: String, depth: Int) {
      for child in children[directory] ?? [] {
        result.append(Row(child: child, directory: directory, depth: depth))
        if child.is_directory, expanded.contains(child.relative) { visit(child.relative, depth: depth + 1) }
      }
      if more.contains(directory) || loading.contains(directory) || errors[directory] != nil {
        result.append(Row(child: nil, directory: directory, depth: depth))
      }
    }
    visit("", depth: 0)
    return result
  }
  func toggle(_ directory: String, source: String, folders: FolderSources) async {
    if expanded.contains(directory) { expanded.remove(directory); return }
    expanded.insert(directory)
    if children[directory] == nil { await load(directory, source: source, folders: folders) }
  }
  func load(_ directory: String, source: String, folders: FolderSources) async {
    guard !loading.contains(directory) else { return }
    loading.insert(directory); errors[directory] = nil
    let expected = generation
    defer { if generation == expected { loading.remove(directory) } }
    do {
      let page = try await folders.children(source, directory: directory, offset: children[directory]?.count ?? 0)
      guard generation == expected, !Task.isCancelled else { return }
      children[directory, default: []].append(contentsOf: page.rows)
      if page.has_more { more.insert(directory) } else { more.remove(directory) }
    } catch {
      if generation == expected, !Task.isCancelled { errors[directory] = error.localizedDescription }
    }
  }
  func refresh(source: String, folders: FolderSources) async {
    generation += 1
    let expected = generation
    loading.removeAll()
    // Refresh only the root and directories that are currently expanded.
    for directory in [""] + expanded.sorted() {
      let limit = max(100, children[directory]?.count ?? 0)
      children[directory] = nil; more.remove(directory); errors[directory] = nil
      for _ in stride(from: 0, to: limit, by: 100) {
        guard generation == expected, !Task.isCancelled else { return }
        await load(directory, source: source, folders: folders)
        if !more.contains(directory) { break }
      }
    }
    // Cached closed directories must be queried again after an index change.
    for directory in Array(children.keys) where !directory.isEmpty && !expanded.contains(directory) {
      children[directory] = nil; more.remove(directory); errors[directory] = nil
    }
  }
  func apply(_ states: [String: String], visible: Set<String>) {
    for directory in Array(children.keys) {
      guard var rows = children[directory] else { continue }
      for index in rows.indices where visible.contains(rows[index].relative) {
        let state = states[rows[index].relative]
        rows[index].entry?.state = state
      }
      children[directory] = rows
    }
  }
}

private struct FolderFileRow: View {
  let source: FolderSource
  let entry: FolderEntry
  var nested = false
  var body: some View {
    HStack(spacing: 14) {
      MacFileThumbnail(source: source, relative: entry.relative, revision: entry.revision,
        video: entry.media_type.hasPrefix("video/"), size: 44)
      VStack(alignment: .leading, spacing: 5) {
        Text((entry.relative as NSString).lastPathComponent).lineLimit(1).help(entry.relative)
        if !nested { Text(entry.relative).font(.caption).foregroundStyle(.secondary).lineLimit(1).truncationMode(.middle) }
      }
      Spacer(minLength: 12)
      Text(ByteCountFormatter.string(fromByteCount: Int64(entry.size), countStyle: .file))
        .font(.caption).foregroundStyle(.secondary)
      Image(systemName: taskSymbol(entry.state))
        .foregroundStyle(entry.state == "received" ? .green : .secondary)
        .help(LocalizedStringKey(entry.state.map { "state_" + $0 } ?? "folder_not_queued"))
    }.padding(.vertical, 10)
  }
}
