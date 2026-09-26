import AppKit
import SwiftUI

struct FolderSourcesPage: View {
  @ObservedObject var folders: FolderSources
  @ObservedObject var backup: BackupModel
  private var selection: String? {
    get { folders.selectedSourceID }
    nonmutating set { folders.selectedSourceID = newValue }
  }
  @State private var adding: URL?
  @State private var removing: String?
  @State private var showHelp = false
  @State private var issuesPage = false
  @State private var rulesSource: FolderSource?
  var body: some View {
    VStack(alignment: .leading, spacing: 14) {
      if backup.paused {
        HStack {
          Label("folder_global_paused", systemImage: "pause.circle")
          Spacer()
          Button("mac_resume_all") { Task { await backup.setPaused(false) } }
            .disabled(!backup.ready || backup.pairing == nil)
        }.font(.callout).padding(12)
          .background(.quaternary.opacity(0.35), in: RoundedRectangle(cornerRadius: 8))
      }
      if let selection, let source = folders.sources.first(where: { $0.id == selection }) {
        Button { if issuesPage { issuesPage = false } else { self.selection = nil } } label: {
          Label(issuesPage ? "folder_open" : "sources_back", systemImage: "chevron.left")
        }
        if issuesPage { FolderIssuesPage(folders: folders, backup: backup, source: source) }
        else {
          FolderSourceDetail(folders: folders, backup: backup, source: source,
            openIssues: { issuesPage = true }, editRules: { rulesSource = source }).id(source.id)
        }
      } else {
        ScrollView {
          VStack(alignment: .leading, spacing: 14) {
            ForEach(folders.sources) { source in
              FolderSourceCard(folders: folders, backup: backup, source: source,
                open: { selection = source.id; issuesPage = false },
                openIssues: { selection = source.id; issuesPage = true },
                editRules: { rulesSource = source }, remove: { removing = source.id })
            }
            if folders.sources.isEmpty {
              Text("folder_sources_empty").foregroundStyle(.secondary).padding(.vertical, 24)
            }
          }
        }
      }
      if let error = folders.error { Text(error).font(.caption).foregroundStyle(.orange) }
    }.padding(20).frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
      .toolbar {
        ToolbarItemGroup(placement: .primaryAction) {
          Button { showHelp = true } label: { Label("folder_help", systemImage: "questionmark.circle") }
            .labelStyle(.iconOnly).help("folder_help")
          Button { choose() } label: { Label("folder_add", systemImage: "folder.badge.plus") }
            .labelStyle(.titleAndIcon).disabled(!backup.ready)
        }
      }
      .sheet(item: $rulesSource) { source in FolderRulesSheet(folders: folders, source: source) { rulesSource = nil } }
      .sheet(isPresented: $showHelp) {
        VStack(alignment: .leading, spacing: 16) {
          Text("folder_help").font(.title2)
          Text("folder_automatic").font(.headline)
          Text("folder_automatic_guide")
          Text("folder_backup_now").font(.headline)
          Text("folder_manual_guide")
          Text("folder_readonly_note").foregroundStyle(.secondary)
          HStack { Spacer(); Button("settings_done") { showHelp = false }.keyboardShortcut(.defaultAction) }
        }.padding(24).frame(width: 440)
      }
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
  let openIssues: () -> Void
  let editRules: () -> Void
  let remove: () -> Void
  var body: some View {
    VStack(alignment: .leading, spacing: 12) {
      HStack(alignment: .top, spacing: 12) {
        Image(systemName: "folder").font(.title2).foregroundStyle(.tint)
        VStack(alignment: .leading, spacing: 5) {
          Text(folders.displayName(source)).font(.headline).lineLimit(1)
          Text(folders.displayPath(source)).font(.caption).foregroundStyle(.secondary)
            .lineLimit(1).truncationMode(.middle).help(folders.displayPath(source))
        }
        Spacer()
        Menu {
          Button("folder_rules", action: editRules)
          Button("folder_check") { folders.check(source.id, userInitiated: true) }
          Button("folder_reveal") { folders.reveal(source) }
          if let checked = source.lastCheck {
            Text(NSLocalizedString("folder_last_check", comment: "") + " " + checked.formatted(date: .abbreviated, time: .shortened))
          }
          if let summary = folders.summaries[source.id], summary.unsupported > 0 {
            Text(String(format: NSLocalizedString("folder_unsupported", comment: ""), summary.unsupported))
          }
          Divider()
          Button("folder_remove", role: .destructive, action: remove)
        } label: { Label("folder_more", systemImage: "ellipsis") }
          .menuStyle(.borderlessButton).menuIndicator(.hidden).fixedSize()
      }
      HStack(spacing: 12) {
        FolderSourceStatus(folders: folders, backup: backup, source: source)
        if let summary = folders.summaries[source.id] {
          Text(String(format: NSLocalizedString("folder_count", comment: ""), summary.files,
            ByteCountFormatter.string(fromByteCount: Int64(summary.bytes), countStyle: .file)))
            .font(.caption).foregroundStyle(.secondary)
        }
        if !source.issues.isEmpty {
          Button(action: openIssues) { Text(String(format: NSLocalizedString("folder_issues", comment: ""), source.issues.count)) }
            .buttonStyle(.link).foregroundStyle(.orange)
        }
      }
      FolderSourceActions(folders: folders, backup: backup, source: source, open: open)
    }.padding(16).frame(maxWidth: .infinity, alignment: .leading)
      .background(.quaternary.opacity(0.35), in: RoundedRectangle(cornerRadius: 12))
  }
}
private struct FolderSourceStatus: View {
  @ObservedObject var folders: FolderSources
  @ObservedObject var backup: BackupModel
  let source: FolderSource
  private var key: String {
    let phase = folders.phases[source.id]
    if let phase, ["folder_scanning", "folder_scan_failed", "folder_offline"].contains(phase) { return phase }
    if folders.actionMessages[source.id] == "folder_check_finished",
      let checked = source.lastCheck, Date().timeIntervalSince(checked) < 15 { return "folder_check_finished" }
    if source.enabled || !(source.retryPaths ?? []).isEmpty {
      if backup.paused {
        return source.automaticActive ? "folder_auto_enabled" : source.manualActive ? "folder_manual_queued" : "folder_retry_waiting"
      }
      if backup.pairing == nil { return "folder_pair_first" }
      if let phase, !["folder_up_to_date", "folder_paused", "folder_manual_idle"].contains(phase) { return phase }
      return source.automaticActive ? "folder_auto_active" : "folder_manual_active"
    }
    return "folder_manual_idle"
  }
  var body: some View { Text(LocalizedStringKey(key)).font(.callout).foregroundStyle(.secondary) }
}
private struct FolderSourceActions: View {
  @ObservedObject var folders: FolderSources
  @ObservedObject var backup: BackupModel
  let source: FolderSource
  var open: (() -> Void)? = nil
  var body: some View {
    VStack(alignment: .leading, spacing: 6) {
      ViewThatFits(in: .horizontal) {
        HStack(spacing: 10) { buttons; Spacer(minLength: 12); automatic }
        VStack(alignment: .leading, spacing: 10) { HStack(spacing: 10) { buttons }; automatic }
      }
      if let message = folders.actionMessages[source.id],
        (message != "folder_global_wait" || backup.paused),
        !["folder_check_finished", "folder_check_running", "folder_backup_requested", "folder_pause_explanation", "folder_automatic_on", "folder_automatic_off"].contains(message) {
        Text(LocalizedStringKey(message)).font(.caption).foregroundStyle(.secondary)
          .accessibilityIdentifier("folder.action_feedback")
      }
    }
  }
  private var automatic: some View {
    Toggle("folder_automatic", isOn: Binding(get: { source.automaticActive },
      set: { folders.setAutomatic(source.id, $0) }))
      .toggleStyle(.switch).controlSize(.small).fixedSize()
      .help("folder_automatic_hint").disabled(folders.starting.contains(source.id))
  }
  @ViewBuilder private var buttons: some View {
    if let open {
      Button(action: open) { Label("folder_open", systemImage: "folder") }
    }
    if source.manualActive {
      Button { folders.pause(source.id) } label: { Label("folder_stop_manual", systemImage: "stop.circle") }
        .help("folder_pause_help").disabled(folders.starting.contains(source.id))
    } else {
      Button { Task { await folders.start(source.id) } } label: {
        Label(folders.starting.contains(source.id) ? "folder_starting" : "folder_backup_now", systemImage: "arrow.up.circle")
      }.help("folder_backup_help")
        .disabled(!backup.ready || backup.pairing == nil || folders.starting.contains(source.id))
    }
  }
}
private struct AddFolderSheet: View {
  let url: URL
  @ObservedObject var folders: FolderSources
  var close: () -> Void
  @State private var automatic = true
  @State private var existing = true
  @State private var include = ""
  @State private var exclude = ""
  @State private var adding = false
  @State private var error: String?
  var body: some View {
    VStack(alignment: .leading, spacing: 18) {
      Label("folder_add", systemImage: "folder.badge.plus").font(.title2.weight(.medium))
      Text(url.lastPathComponent).font(.headline)
      Text(url.path).font(.caption).foregroundStyle(.secondary).textSelection(.enabled)
      Text("folder_scan_note").foregroundStyle(.secondary)
      Toggle("folder_include_existing", isOn: $existing)
      Toggle("folder_automatic", isOn: $automatic)
      DisclosureGroup("folder_rules") { FolderRuleFields(include: $include, exclude: $exclude) }
      Text("folder_readonly_note").font(.caption).foregroundStyle(.secondary)
      if let error { Text(error).foregroundStyle(.orange) }
      HStack {
        Button("cancel", action: close).disabled(adding)
        Spacer()
        Button("folder_add") {
          Task {
            adding = true
            do { try await folders.add(url, automatic: automatic, existing: existing,
              include: FolderRuleFields.lines(include), exclude: FolderRuleFields.lines(exclude)); close() }
            catch let failure as Bridge.Failure where failure.code == "conflict" { self.error = NSLocalizedString("folder_overlap_error", comment: "") }
            catch let error as FolderRuleError { self.error = NSLocalizedString("folder_rules_invalid", comment: "") + " " + error.localizedDescription }
            catch { self.error = error.localizedDescription }
            adding = false
          }
        }.buttonStyle(.borderedProminent).disabled(adding || (!existing && !automatic))
      }
    }.padding(28).frame(width: 520).interactiveDismissDisabled(adding)
  }
}
private struct FolderSourceDetail: View {
  @ObservedObject var folders: FolderSources
  @ObservedObject var backup: BackupModel
  let source: FolderSource
  let openIssues: () -> Void
  let editRules: () -> Void
  @State private var entries: [FolderEntry] = []
  @State private var more = true
  @State private var loading = false
  @State private var error: String?
  @State private var visible = Set<String>()
  @AppStorage("macFolderListLayout") private var layout = "flat"
  @AppStorage("macFolderFileSort") private var sort = FolderFileSort.modifiedNewest
  @AppStorage("macFolderTreeDescending") private var treeDescending = false
  @State private var request = UUID()
  @StateObject private var tree = FolderTreeModel()
  var body: some View {
    VStack(alignment: .leading, spacing: 14) {
      HStack {
        VStack(alignment: .leading, spacing: 6) {
          Text(folders.displayName(source)).font(.title3.weight(.medium))
          Text(folders.displayPath(source)).font(.caption).foregroundStyle(.secondary)
            .lineLimit(1).truncationMode(.middle)
          FolderSourceStatus(folders: folders, backup: backup, source: source)
        }
        Spacer()
        if !source.issues.isEmpty {
          Button(action: openIssues) { Label(String(format: NSLocalizedString("folder_issues", comment: ""), source.issues.count), systemImage: "exclamationmark.triangle") }
            .foregroundStyle(.orange).accessibilityIdentifier("folder.issues")
        }
        Button("folder_rules", action: editRules)
      }
      FolderSourceActions(folders: folders, backup: backup, source: source)
      HStack {
        Menu {
          if layout == "folders" {
            Picker("folder_sort", selection: $treeDescending) {
              Text("folder_sort_name_asc").tag(false)
              Text("folder_sort_name_desc").tag(true)
            }
          } else {
            Picker("folder_sort", selection: $sort) {
              ForEach(FolderFileSort.allCases) { item in Text(LocalizedStringKey(item.title)).tag(item) }
            }
          }
        } label: {
          Label(LocalizedStringKey(layout == "folders"
            ? (treeDescending ? "folder_sort_name_desc" : "folder_sort_name_asc") : sort.title),
            systemImage: "arrow.up.arrow.down")
        }.menuStyle(.borderlessButton).fixedSize(horizontal: true, vertical: true)
          .help("folder_sort").accessibilityIdentifier("folder.sort")
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
    .task(id: "\(source.id)|\(folders.indexRevision)|\(layout)|\(sort.rawValue)|\(treeDescending)") {
      if layout == "folders" { await tree.refresh(source: source.id, folders: folders, descending: treeDescending) }
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
    guard !loading else { return }
    let expected = request
    loading = true
    defer { if request == expected { loading = false } }
    do {
      let page = try await folders.page(source.id, offset: entries.count, sort: sort)
      guard request == expected, !Task.isCancelled else { return }
      entries.append(contentsOf: page); more = page.count == 100; error = nil
    } catch { if request == expected, !Task.isCancelled { self.error = error.localizedDescription } }
  }
  private func reload() async {
    let expected = UUID(); request = expected
    let selectedSort = sort
    let limit = max(100, entries.count)
    entries = []; more = false; loading = true; error = nil
    defer { if request == expected { loading = false } }
    do {
      var result: [FolderEntry] = []
      for offset in stride(from: 0, to: limit, by: 100) {
        let page = try await folders.page(source.id, offset: offset, sort: selectedSort)
        guard request == expected, !Task.isCancelled else { return }
        result.append(contentsOf: page)
        if page.count < 100 { break }
      }
      entries = result; more = result.count > 0 && result.count % 100 == 0
    } catch { if request == expected, !Task.isCancelled { self.error = error.localizedDescription } }
  }
}

private struct FolderIssuesPage: View {
  @ObservedObject var folders: FolderSources
  @ObservedObject var backup: BackupModel
  let source: FolderSource
  var body: some View {
    VStack(alignment: .leading, spacing: 14) {
      Text("folder_issue_details").font(.title2)
      Text(folders.displayPath(source)).font(.caption).foregroundStyle(.secondary)
      Text("folder_issues_explanation").foregroundStyle(.secondary)
      if source.issues.isEmpty { Label("folder_no_issues", systemImage: "checkmark.circle").padding(.vertical, 24) }
      ScrollView {
        LazyVStack(alignment: .leading, spacing: 16) {
          ForEach(source.issues.keys.sorted(), id: \.self) { path in
            FolderIssueRow(folders: folders, backup: backup, source: source, path: path)
            Divider()
          }
        }
      }
    }.frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
  }
}
private struct FolderRuleFields: View {
  @Binding var include: String
  @Binding var exclude: String
  static func lines(_ text: String) -> [String] {
    text.components(separatedBy: .newlines).filter { !$0.trimmingCharacters(in: .whitespaces).isEmpty }
  }
  var body: some View {
    VStack(alignment: .leading, spacing: 10) {
      Text("folder_rules_note").font(.callout)
      Text("folder_rules_include").font(.headline)
      TextEditor(text: $include).font(.system(.body, design: .monospaced)).frame(height: 90)
        .overlay(RoundedRectangle(cornerRadius: 4).stroke(.quaternary))
        .accessibilityLabel("folder_rules_include")
      Text("folder_rules_exclude").font(.headline)
      TextEditor(text: $exclude).font(.system(.body, design: .monospaced)).frame(height: 90)
        .overlay(RoundedRectangle(cornerRadius: 4).stroke(.quaternary))
        .accessibilityLabel("folder_rules_exclude")
      Text(verbatim: NSLocalizedString("folder_rules_examples", comment: "")).font(.caption).foregroundStyle(.secondary)
    }
  }
}
private struct FolderRulesSheet: View {
  @ObservedObject var folders: FolderSources
  let source: FolderSource
  let close: () -> Void
  @State private var include: String
  @State private var exclude: String
  @State private var saving = false
  @State private var error: String?
  init(folders: FolderSources, source: FolderSource, close: @escaping () -> Void) {
    self.folders = folders; self.source = source; self.close = close
    _include = State(initialValue: (source.includePatterns ?? []).joined(separator: "\n"))
    _exclude = State(initialValue: (source.excludePatterns ?? []).joined(separator: "\n"))
  }
  var body: some View {
    VStack(alignment: .leading, spacing: 12) {
      Text("folder_rules").font(.title2)
      Text(folders.displayName(source)).foregroundStyle(.secondary)
      FolderRuleFields(include: $include, exclude: $exclude)
      Text("folder_rules_effect").font(.caption).foregroundStyle(.secondary)
      if let error { Text(error).foregroundStyle(.orange).font(.caption).textSelection(.enabled) }
      HStack {
        Button("cancel", action: close).disabled(saving)
        Spacer()
        Button("folder_rules_save") {
          Task {
            saving = true
            do { try await folders.saveRules(source.id, include: FolderRuleFields.lines(include), exclude: FolderRuleFields.lines(exclude)); close() }
            catch { self.error = NSLocalizedString("folder_rules_invalid", comment: "") + " " + error.localizedDescription }
            saving = false
          }
        }.disabled(saving).keyboardShortcut(.defaultAction)
      }
    }.padding(24).frame(width: 500).interactiveDismissDisabled(saving)
  }
}

private struct FolderIssueRow: View {
  @ObservedObject var folders: FolderSources
  @ObservedObject var backup: BackupModel
  let source: FolderSource
  let path: String
  @State private var dismissing = false
  private var issue: FolderIssue? { source.issueDetails?[path] }
  private var retrying: Bool { (source.retryPaths ?? []).contains(path) }
  var body: some View {
    HStack(alignment: .top, spacing: 16) {
      Image(systemName: "exclamationmark.triangle").foregroundStyle(.orange)
      VStack(alignment: .leading, spacing: 4) {
        Text((path as NSString).lastPathComponent).font(.callout.weight(.medium)).lineLimit(1)
        Text(path).font(.caption).foregroundStyle(.secondary).lineLimit(1)
          .truncationMode(.middle).help(path).textSelection(.enabled)
        Text(LocalizedStringKey("folder_issue_" + (issue?.reason ?? "unknown"))).font(.callout)
        if let detail = issue?.detail { Text(detail).font(.caption).foregroundStyle(.secondary).lineLimit(2).help(detail) }
      }.frame(maxWidth: .infinity, alignment: .leading)
      VStack(alignment: .trailing, spacing: 8) {
        Button("folder_reveal") { folders.reveal(source, relative: path) }
        Button(retrying ? "folder_retry_waiting" : "retry_task") { Task { await folders.retry(source.id, relative: path) } }
          .disabled(retrying || dismissing || !backup.ready || backup.pairing == nil)
        Button("folder_ignore_version") { Task { dismissing = true; await folders.dismissIssue(source.id, relative: path); dismissing = false } }
          .help("folder_ignore_note").disabled(dismissing)
      }.fixedSize()
    }.accessibilityIdentifier("folder.issue." + path)
  }
}

enum FolderFileSort: String, CaseIterable, Identifiable {
  case modifiedNewest = "modified_desc"
  case modifiedOldest = "modified_asc"
  case pathAscending = "path_asc"
  case pathDescending = "path_desc"
  case sizeLargest = "size_desc"
  case sizeSmallest = "size_asc"
  var id: String { rawValue }
  var title: String { "folder_sort_" + rawValue }
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
  private var descending = false
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
      let page = try await folders.children(source, directory: directory, offset: children[directory]?.count ?? 0, descending: descending)
      guard generation == expected, !Task.isCancelled else { return }
      children[directory, default: []].append(contentsOf: page.rows)
      if page.has_more { more.insert(directory) } else { more.remove(directory) }
    } catch {
      if generation == expected, !Task.isCancelled { errors[directory] = error.localizedDescription }
    }
  }
  func refresh(source: String, folders: FolderSources, descending: Bool) async {
    generation += 1
    self.descending = descending
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
