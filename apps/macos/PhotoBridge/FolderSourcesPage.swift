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
              VStack(alignment: .leading, spacing: 12) {
                HStack(spacing: 14) {
                  Image(systemName: "externaldrive").font(.title2).foregroundStyle(.tint)
                  VStack(alignment: .leading, spacing: 5) {
                    Button(source.name) { selection = source.id }.buttonStyle(.plain).font(.headline)
                    Text(LocalizedStringKey(folders.phases[source.id] ?? "folder_scanning"))
                      .font(.callout).foregroundStyle(.secondary)
                    if let summary = folders.summaries[source.id] {
                      Text(String(format: NSLocalizedString("folder_count", comment: ""), summary.files,
                        ByteCountFormatter.string(fromByteCount: Int64(summary.bytes), countStyle: .file)))
                        .font(.caption).foregroundStyle(.secondary)
                    }
                    if !source.issues.isEmpty {
                      Text(String(format: NSLocalizedString("folder_issues", comment: ""), source.issues.count)).font(.caption).foregroundStyle(.orange)
                    }
                    if let count = folders.summaries[source.id]?.unsupported, count > 0 {
                      Text(String(format: NSLocalizedString("folder_unsupported", comment: ""), count)).font(.caption).foregroundStyle(.secondary)
                    }
                    if let checked = source.lastCheck {
                      Text(NSLocalizedString("folder_last_check", comment: "") + " " + checked.formatted(date: .abbreviated, time: .shortened))
                        .font(.caption).foregroundStyle(.secondary)
                    }
                  }
                  Spacer()
                  Menu {
                    Button("folder_open") { selection = source.id }
                    Button("folder_check") { folders.check(source.id) }
                    Button("folder_backup_now") { Task { await folders.start(source.id) } }
                    Button("folder_stop_preparing") { folders.pause(source.id) }
                    Divider()
                    Button("folder_remove", role: .destructive) { removing = source.id }
                  } label: { Image(systemName: "ellipsis.circle") }.menuStyle(.borderlessButton).frame(width: 24)
                }
                Toggle("folder_automatic", isOn: Binding(get: { source.automatic }, set: { folders.setAutomatic(source.id, $0) }))
                  .toggleStyle(.switch).controlSize(.small)
              }.padding(18).background(.quaternary.opacity(0.35), in: RoundedRectangle(cornerRadius: 12))
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
  var body: some View {
    VStack(alignment: .leading, spacing: 14) {
      HStack {
        VStack(alignment: .leading, spacing: 6) {
          Text(source.name).font(.title3.weight(.medium))
          Text(LocalizedStringKey(folders.phases[source.id] ?? "folder_scanning")).foregroundStyle(.secondary)
        }
        Spacer()
        Button("folder_check") { folders.check(source.id) }
        Button("folder_backup_now") { Task { await folders.start(source.id) } }
      }
      if !source.issues.isEmpty {
        DisclosureGroup("folder_issue_details") {
          ForEach(source.issues.keys.sorted(), id: \.self) { Text($0).font(.caption).foregroundStyle(.secondary) }
        }
      }
      Text("folder_detail_note").font(.caption).foregroundStyle(.secondary)
      ScrollView {
        LazyVStack(alignment: .leading, spacing: 0) {
          ForEach(entries) { entry in
            HStack(spacing: 14) {
              Image(systemName: entry.media_type.hasPrefix("video/") ? "video" : "photo")
                .font(.title3).foregroundStyle(.secondary).frame(width: 28)
              VStack(alignment: .leading, spacing: 5) {
                Text((entry.relative as NSString).lastPathComponent).lineLimit(1)
                Text(entry.relative).font(.caption).foregroundStyle(.secondary).lineLimit(1)
              }
              Spacer()
              Text(ByteCountFormatter.string(fromByteCount: Int64(entry.size), countStyle: .file))
                .font(.caption).foregroundStyle(.secondary)
              Image(systemName: taskSymbol(entry.state))
                .foregroundStyle(entry.state == "received" ? .green : .secondary)
                .help(LocalizedStringKey(entry.state.map { "state_" + $0 } ?? "folder_not_queued"))
            }.padding(.vertical, 12)
              .onAppear { visible.insert(entry.relative) }
              .onDisappear { visible.remove(entry.relative) }
            Divider()
          }
          if more { Button("folder_load_more") { Task { await load() } }.padding(.vertical, 16).disabled(loading) }
          if entries.isEmpty { Text("folder_empty").foregroundStyle(.secondary).padding(.vertical, 24) }
        }
      }
      if let error { Text(error).foregroundStyle(.orange).font(.caption) }
    }
    .task(id: "\(source.id)|\(folders.indexRevision)") { await reload() }
    .task(id: "\(folders.revision)|\(backup.queueRevision)|\(visible.sorted().joined(separator: "|"))") {
      try? await Task.sleep(nanoseconds: 150_000_000)
      guard !Task.isCancelled else { return }
      if let states = try? await folders.states(source.id, relatives: Array(visible.prefix(400))) {
        guard !Task.isCancelled else { return }
        for i in entries.indices where visible.contains(entries[i].relative) { entries[i].state = states[entries[i].relative] }
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
