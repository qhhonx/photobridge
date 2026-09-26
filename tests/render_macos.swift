// Render production SwiftUI views with synthetic presentation state and a
// temporary Rust store. No Keychain, Photos authorization or network is opened.
import AppKit
import SwiftUI

@main struct MacLayoutCheck {
  @MainActor static var interactiveWindow: NSWindow?
  @MainActor static func main() throws {
    let output = URL(fileURLWithPath: CommandLine.arguments[1], isDirectory: true)
    try FileManager.default.createDirectory(at: output, withIntermediateDirectories: true)
    let store = FileManager.default.temporaryDirectory.appendingPathComponent("photobridge-layout-" + UUID().uuidString)
    let command = try JSONSerialization.data(withJSONObject: ["op": "open_sender", "root": store.path])
    let pointer = String(decoding: command, as: UTF8.self).withCString { photobridge_call($0) }!
    let response = Data(String(cString: pointer).utf8)
    photobridge_free(pointer)
    guard (try JSONSerialization.jsonObject(with: response) as? [String: Any])?["ok"] as? Bool == true else {
      fatalError("Unable to open isolated layout fixture store")
    }
    let event = try JSONSerialization.data(withJSONObject: ["op": "record_event", "receiver": false,
      "code": "dispatch_waiting", "context": ["queued": 20, "running": 0, "waiting": 5,
        "failed": 2, "paused": true, "active_requests": 0, "execution": "desktop", "reason": "paused"]])
    if let result = String(decoding: event, as: UTF8.self).withCString({ photobridge_call($0) }) {
      photobridge_free(result)
    }
    let app = NSApplication.shared
    app.setActivationPolicy(.accessory)
    Task { @MainActor in
      do {
        if CommandLine.arguments.contains("--burst-cells") {
          for dark in [false, true] {
            try await capture("burst-cells-" + (dark ? "dark" : "light"),
              view: AnyView(BurstCellsFixture()), output: output,
              size: NSSize(width: 600, height: 230), dark: dark)
          }
          try FileManager.default.removeItem(at: store)
          app.terminate(nil)
          return
        }
        if CommandLine.arguments.contains("--library-only") {
          let states: [(String, LibraryPlaceholderKind)] = [
            ("request", .permission(.notDetermined)), ("denied", .permission(.denied)),
            ("restricted", .permission(.restricted)), ("loading", .loading),
            ("empty", .empty(filtered: false)), ("filtered", .empty(filtered: true)),
          ]
          for (name, state) in states {
            for dark in [false, true] {
              try await capture("library-" + name + (dark ? "-dark" : "-light"),
                view: AnyView(LibraryPlaceholder(kind: state)), output: output,
                size: NSSize(width: 660, height: 520), dark: dark)
            }
          }
          try await capture("library-permission-pending",
            view: AnyView(LibraryPlaceholder(kind: .permission(.notDetermined), requestingAccess: true)),
            output: output, size: NSSize(width: 660, height: 520))
          try FileManager.default.removeItem(at: store)
          print("Rendered six library states in light and dark appearances; no photo access requested.")
          app.terminate(nil)
          return
        }
        if CommandLine.arguments.contains("--help-only") {
          for dark in [false, true] {
            try await capture("help-" + (dark ? "dark" : "light"),
              view: AnyView(BackupHelp()), output: output,
              size: NSSize(width: 620, height: 760), dark: dark)
          }
          try FileManager.default.removeItem(at: store)
          app.terminate(nil)
          return
        }
        let model = BackupModel(root: store.appendingPathComponent("ui"))
        await model.refreshDeviceStatus()
        let originalDeviceID = model.deviceSnapshot!.device.id
        let rejectedName = await model.renameDevice("\n")
        precondition(!rejectedName && model.deviceError != nil, "Invalid names must report an error")
        let savedName = await model.renameDevice("Moonlit Cedar")
        precondition(savedName && model.deviceSnapshot!.device.id == originalDeviceID,
          "Renaming must preserve the device identity")
        await model.refreshDeviceStatus()
        precondition(model.deviceSnapshot!.device.name == "Moonlit Cedar", "Name must persist")
        model.ready = true
        model.paused = true
        // This identity is display-only; the renderer never starts a sender.
        let paired = Pairing(version: 1, receiverID: "layout-fixture", endpoint: "https://receiver.invalid", certificate: "", token: "")
        model.deviceSnapshot = DeviceSnapshot(device: model.deviceSnapshot!.device,
          peers: [DeviceSnapshot.Peer(key: paired.receiverID,
            profile: DeviceProfile(id: String(repeating: "a", count: 64), name: "Amber Otter"),
            last_seen: Int64(Date().timeIntervalSince1970))])
        if CommandLine.arguments.contains("--folder-only") {
          model.pairing = paired
          let folder = store.appendingPathComponent("Documents")
          try FileManager.default.createDirectory(at: folder, withIntermediateDirectories: true)
          let bitmap = NSBitmapImageRep(bitmapDataPlanes: nil, pixelsWide: 64, pixelsHigh: 48,
            bitsPerSample: 8, samplesPerPixel: 4, hasAlpha: true, isPlanar: false,
            colorSpaceName: .deviceRGB, bytesPerRow: 0, bitsPerPixel: 0)!
          for x in 0..<64 { for y in 0..<48 { bitmap.setColor(x < 32 ? NSColor(deviceRed: 0.1, green: 0.4, blue: 0.9, alpha: 1) : NSColor(deviceRed: 1, green: 0.5, blue: 0.1, alpha: 1), atX: x, y: y) } }
          let png = bitmap.representation(using: .png, properties: [:])!
          try png.write(to: folder.appendingPathComponent("root-photo.png"))
          try FileManager.default.createDirectory(at: folder.appendingPathComponent("Trip/Day 1"), withIntermediateDirectories: true)
          for name in ["Trip/photo.png", "Trip/Day 1/photo.png"] { try png.write(to: folder.appendingPathComponent(name)) }
          let bookmark = try folder.bookmarkData(options: [.withSecurityScope, .securityScopeAllowOnlyReadAccess], includingResourceValuesForKeys: nil, relativeTo: nil)
          try FileManager.default.createDirectory(at: model.root, withIntermediateDirectories: true)
          let folders = FolderSources(backup: model)
          var source = FolderSource(id: "layout-folder", name: "Documents", bookmark: bookmark,
            automatic: true, enabled: true, receiver: paired.receiverID, lastCheck: Date(),
            issues: [String(repeating: "nested-folder/", count: 12) + "unreadable.jpg": "1"])
          source.issues["root-photo.png"] = "fixture-revision"
          source.issueDetails = ["root-photo.png": FolderIssue(reason: "unreadable_media", detail: nil)]
          let legacyData = try JSONEncoder().encode(source)
          var legacy = try JSONSerialization.jsonObject(with: legacyData) as! [String: Any]
          legacy.removeValue(forKey: "issueDetails"); legacy.removeValue(forKey: "retryPaths"); legacy.removeValue(forKey: "includePatterns"); legacy.removeValue(forKey: "excludePatterns")
          let restored = try JSONDecoder().decode(FolderSource.self, from: JSONSerialization.data(withJSONObject: legacy))
          precondition(restored.issues.count == 2 && restored.issueDetails == nil && restored.retryPaths == nil)
          folders.sources = [source]
          _ = try await Bridge.call(["op": "folder", "command": ["action": "begin", "source": source.id, "root": folder.path]])
          while true {
            let data = try await Bridge.call(["op": "folder", "command": ["action": "step"]])
            if !(try JSONDecoder().decode(FolderSummary.self, from: data)).scanning { break }
          }
          let rootChildren = try await folders.children(source.id, directory: "", offset: 0)
          precondition(rootChildren.rows.map(\.relative) == ["Trip", "root-photo.png"] && !rootChildren.has_more)
          let tripChildren = try await folders.children(source.id, directory: "Trip", offset: 0)
          precondition(tripChildren.rows.map(\.relative) == ["Trip/Day 1", "Trip/photo.png"])
          let preview: NSImage? = await withCheckedContinuation { continuation in
            FileThumbnailPipeline.shared.request(id: UUID(), source: source, relative: "root-photo.png", revision: "test", size: 56) {
              continuation.resume(returning: $0)
            }
          }
          precondition(preview != nil, "System thumbnail generation must succeed for a valid PNG")
          try FileManager.default.createSymbolicLink(at: folder.appendingPathComponent("link.png"), withDestinationURL: folder.appendingPathComponent("root-photo.png"))
          let linkedPreview: NSImage? = await withCheckedContinuation { continuation in
            FileThumbnailPipeline.shared.request(id: UUID(), source: source, relative: "link.png", revision: "test", size: 56) {
              continuation.resume(returning: $0)
            }
          }
          precondition(linkedPreview == nil, "Previews must not follow file symlinks")
          let systemDocument = FileManager.default.urls(for: .documentDirectory, in: .userDomainMask)[0]
          var localizedSource = source
          localizedSource.bookmark = try systemDocument.bookmarkData(options: [.withSecurityScope, .securityScopeAllowOnlyReadAccess], includingResourceValuesForKeys: nil, relativeTo: nil)
          precondition(folders.displayName(localizedSource) == systemDocument.lastPathComponent)
          folders.summaries[source.id] = try JSONDecoder().decode(FolderSummary.self,
            from: Data(#"{"files":152,"bytes":297061580,"unsupported":3925,"scanning":false}"#.utf8))
          folders.phases[source.id] = "folder_attention"
          folders.check(source.id, userInitiated: true)
          precondition(folders.actionMessages[source.id] == "folder_check_requested")
          folders.pause(source.id)
          precondition(!folders.sources[0].enabled && folders.phases[source.id] == "folder_paused")
          precondition(folders.actionMessages[source.id] == "folder_pause_explanation")
          model.pairing = nil
          await folders.start(source.id)
          precondition(!folders.sources[0].enabled && folders.actionMessages[source.id] == "folder_pair_first")
          model.pairing = paired
          await folders.start(source.id)
          precondition(folders.sources[0].manualActive && !folders.sources[0].issues.isEmpty && model.paused)
          precondition(folders.actionMessages[source.id] == "folder_global_wait" && folders.starting.isEmpty)
          var beganStart = false
          let pendingStart = Task { beganStart = true; await folders.start(source.id) }
          while !beganStart { await Task.yield() }
          folders.pause(source.id)
          await pendingStart.value
          precondition(!folders.sources[0].enabled && folders.actionMessages[source.id] == "folder_pause_explanation",
            "An in-flight start must not undo a later stop")
          folders.setAutomatic(source.id, true)
          precondition(folders.sources[0].automaticActive)
          folders.setAutomatic(source.id, false)
          precondition(!folders.sources[0].enabled && !folders.sources[0].automaticActive)
          await folders.retry(source.id, relative: "root-photo.png")
          precondition(!folders.sources[0].enabled && model.paused)
          precondition(folders.sources[0].retryPaths == ["root-photo.png"])
          precondition(folders.sources[0].issues.count == 2)
          let persisted = try JSONDecoder().decode([FolderSource].self, from: Data(contentsOf: model.root.appendingPathComponent("folder-sources.json")))
          precondition(persisted[0].retryPaths == ["root-photo.png"] && persisted[0].issueDetails?["root-photo.png"]?.reason == "unreadable_media")
          try await folders.saveRules(source.id, include: ["**/*.png"], exclude: ["Trip/**"])
          precondition(folders.sources[0].includePatterns == ["**/*.png"] && folders.sources[0].retryPaths?.isEmpty != false && model.paused)
          do { try await folders.saveRules(source.id, include: ["["], exclude: []); preconditionFailure("Invalid glob must not save") }
          catch { precondition(folders.sources[0].includePatterns == ["**/*.png"]) }
          await folders.dismissIssue(source.id, relative: "root-photo.png")
          precondition(folders.sources[0].issues.count == 1 && folders.sources[0].issueDetails?["root-photo.png"] == nil)
          let savedRules = try JSONDecoder().decode([FolderSource].self, from: Data(contentsOf: model.root.appendingPathComponent("folder-sources.json")))
          precondition(savedRules[0].excludePatterns == ["Trip/**"] && savedRules[0].issues.count == 1)
          try await folders.saveRules(source.id, include: [], exclude: [])
          // Reset only the synthetic dismissed record by forgetting and rebuilding its index.
          _ = try await Bridge.call(["op": "folder", "command": ["action": "forget", "source": source.id]])
          _ = try await Bridge.call(["op": "folder", "command": ["action": "begin", "source": source.id, "root": folder.path]])
          while true {
            let data = try await Bridge.call(["op": "folder", "command": ["action": "step"]])
            if !(try JSONDecoder().decode(FolderSummary.self, from: data)).scanning { break }
          }
          let desktop = store.appendingPathComponent("Desktop")
          try FileManager.default.createDirectory(at: desktop, withIntermediateDirectories: true)
          let desktopBookmark = try desktop.bookmarkData(options: [.withSecurityScope, .securityScopeAllowOnlyReadAccess], includingResourceValuesForKeys: nil, relativeTo: nil)
          let second = FolderSource(id: "layout-desktop", name: "Desktop", bookmark: desktopBookmark,
            automatic: true, enabled: true, receiver: paired.receiverID, lastCheck: Date())
          folders.sources = [source, second]
          folders.pause(source.id)
          // Persist fixture state for the workspace's normal folder-open path.
          try FileManager.default.createDirectory(at: model.root, withIntermediateDirectories: true)
          try JSONEncoder().encode(folders.sources).write(to: model.root.appendingPathComponent("folder-sources.json"))
          folders.check(source.id, userInitiated: true)
          if CommandLine.arguments.contains("--interactive-folder") {
            let window = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 1080, height: 740),
              styleMask: [.titled, .closable, .resizable], backing: .buffered, defer: false)
            window.title = "PhotoBridge Folder Layout — Synthetic Data"
            window.contentView = NSHostingView(rootView: MacWorkspace(model: model,
              library: PhotoLibraryModel(), initialDestination: .sources, folderSources: folders))
            interactiveWindow = window
            window.center(); window.makeKeyAndOrderFront(nil)
            app.activate(ignoringOtherApps: true)
            print("Isolated folder fixture ready.")
            return
          }
          for dark in [false, true] {
            try await capture("folder-sources-" + (dark ? "dark" : "light"),
              view: AnyView(FolderSourcesPage(folders: folders, backup: model)), output: output,
              size: NSSize(width: 800, height: 680), dark: dark)
          }
          folders.selectedSourceID = source.id
          UserDefaults.standard.set("flat", forKey: "macFolderListLayout")
          UserDefaults.standard.set(true, forKey: "macListThumbnails")
          try await capture("folder-thumbnail", view: AnyView(MacFileThumbnail(source: source,
            relative: "root-photo.png", revision: "test", size: 100)), output: output, size: NSSize(width: 140, height: 140))
          UserDefaults.standard.set(false, forKey: "macListThumbnails")
          try await capture("folder-thumbnail-disabled", view: AnyView(MacFileThumbnail(source: source,
            relative: "root-photo.png", revision: "test", size: 100)), output: output, size: NSSize(width: 140, height: 140))
          UserDefaults.standard.set(true, forKey: "macListThumbnails")
          try await capture("folder-detail", view: AnyView(FolderSourcesPage(folders: folders, backup: model)),
            output: output, size: NSSize(width: 800, height: 680))
          UserDefaults.standard.set("folders", forKey: "macFolderListLayout")
          try await capture("folder-tree", view: AnyView(FolderSourcesPage(folders: folders, backup: model)),
            output: output, size: NSSize(width: 800, height: 680))
          UserDefaults.standard.set("flat", forKey: "macFolderListLayout")
          folders.selectedSourceID = nil
          try await capture("folder-workspace", view: AnyView(MacWorkspace(model: model,
            library: PhotoLibraryModel(), initialDestination: .sources, folderSources: folders)),
            output: output, size: NSSize(width: 1080, height: 740))
          UserDefaults.standard.set("backup", forKey: "macSettingsSection")
          try await capture("folder-settings", view: AnyView(MacPreferences(model: model, folders: folders)),
            output: output, size: NSSize(width: 800, height: 900))
          print("Folder mode, global pause, retry persistence, legacy decoding, directory queries and system thumbnails passed; rendered both layouts and source settings.")
          app.terminate(nil)
          return
        }
        if CommandLine.arguments.contains("--interactive") {
          model.pairing = paired
          model.summary.total = 120
          model.summary.received = 93
          model.summary.queued = 20
          model.summary.waiting = 5
          model.summary.failed = 2
          let window = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 1080, height: 740),
            styleMask: [.titled, .closable, .resizable], backing: .buffered, defer: false)
          window.title = "PhotoBridge Layout Check — Synthetic Data"
          window.contentView = NSHostingView(rootView:
            MacWorkspace(model: model, library: PhotoLibraryModel(), initialDestination: .backup))
          interactiveWindow = window
          window.center()
          window.makeKeyAndOrderFront(nil)
          app.activate(ignoringOtherApps: true)
          print("Interactive fixture window ready. Store: \(store.path)")
          return
        }
        let overview = { AnyView(MacBackupPage(model: model, pair: {}, library: {}, sources: {}, showTransfers: { _ in })) }
        try await capture("backup-unpaired", view: overview(), output: output)
        model.pairing = paired
        try await capture("backup-empty", view: overview(), output: output)
        model.summary.total = 120
        model.summary.received = 93
        model.summary.queued = 20
        model.summary.waiting = 5
        model.summary.failed = 2
        model.summary.waiting_reason = "network"
        model.summary.next_retry_at = Int64(Date().addingTimeInterval(60).timeIntervalSince1970)
        model.paused = false
        model.pendingImports = 6
        try await capture("backup-waiting", view: overview(), output: output)
        try await capture("backup-waiting-dark", view: overview(), output: output, dark: true)
        model.importing = true
        model.exportProgress = 0.45
        try await capture("backup-preparing", view: overview(), output: output)
        model.importing = false
        for section in [MacSettingsSection.backup, .cache, .diagnostics] {
          UserDefaults.standard.set(section.rawValue, forKey: "macSettingsSection")
          try await capture("settings-" + section.rawValue,
            view: AnyView(MacPreferences(model: model)), output: output,
            size: NSSize(width: 620, height: 580))
        }
        UserDefaults.standard.set(MacSettingsSection.cache.rawValue, forKey: "macSettingsSection")
        try await capture("settings-cache-dark", view: AnyView(MacPreferences(model: model)),
          output: output, size: NSSize(width: 620, height: 580), dark: true)
        try await capture("settings-save-error", view: AnyView(MacPreferences(model: model)),
          output: output, size: NSSize(width: 620, height: 580), action: {
            let original = model.storage!.settings.cache_budget_bytes
            var invalid = model.storage!.settings
            invalid.cache_budget_bytes = 0
            await model.saveStorage(invalid)
            precondition(model.storageError != nil, "Invalid settings must report an error")
            precondition(model.storage!.settings.cache_budget_bytes == original,
              "Rejected settings must not change the stored budget")
          })
        var valid = model.storage!.settings
        valid.cache_budget_bytes = 2 << 30
        await model.saveStorage(valid)
        precondition(model.storageError == nil && model.storage!.settings.cache_budget_bytes == 2 << 30,
          "Valid settings must persist and clear the previous error")
        model.scanningHistory = true
        let deferredImport = await model.importAssets(["synthetic-scan-gate"], requestAuthorization: false)
        model.scanningHistory = false
        precondition(deferredImport && !model.importing, "Discovery during a scan must remain queued without requesting Photos")
        let queuedDuringScan = try await Bridge.call(["op": "pending_sources", "receiver_id": paired.receiverID])
        let queuedResult = try JSONSerialization.jsonObject(with: queuedDuringScan) as! [String: Any]
        precondition(queuedResult["count"] as! Int == 1)
        let started = try await Bridge.call(["op": "history_control", "receiver_id": paired.receiverID, "action": "start"])
        let run = try JSONDecoder().decode(HistoricalImportStatus.self, from: started).run
        _ = try await Bridge.call(["op": "history_batch", "receiver_id": paired.receiverID, "run": run,
          "sources": [["synthetic-history-one", "1"], ["synthetic-history-two", "1"]], "finished": false])
        _ = try await Bridge.call(["op": "history_control", "receiver_id": paired.receiverID, "action": "pause"])
        await model.refreshHistoricalImport()
        precondition(model.historicalImport?.checked == 2 && model.historicalImport?.pending == 2)
        try await capture("history-paused", view: AnyView(Form { HistoricalImportSettings(model: model) }.formStyle(.grouped)), output: output,
          size: NSSize(width: 620, height: 480))
        _ = try await Bridge.call(["op": "history_control", "receiver_id": paired.receiverID, "action": "resume"])
        _ = try await Bridge.call(["op": "history_batch", "receiver_id": paired.receiverID, "run": run, "sources": [], "finished": true])
        await model.refreshHistoricalImport()
        precondition(model.historicalImport?.state == "scanned" && model.historicalImport?.pending == 2,
          "Scan completion must not report pending preparation as complete")
        try await capture("history-scanned", view: AnyView(Form { HistoricalImportSettings(model: model) }.formStyle(.grouped)), output: output,
          size: NSSize(width: 620, height: 480))
        print("Rendered 12 fixture screens; settings, device rename and historical-scan state checks passed.")
        try FileManager.default.removeItem(at: store)
        app.terminate(nil)
      } catch {
        fputs("Layout rendering failed: \(error)\n", stderr)
        exit(1)
      }
    }
    app.run()
  }

  @MainActor static func capture(_ name: String, view: AnyView, output: URL,
    size: NSSize = NSSize(width: 660, height: 720), dark: Bool = false,
    action: (() async -> Void)? = nil) async throws {
    let host = NSHostingView(rootView: view
      .frame(width: size.width, height: size.height, alignment: .topLeading)
      .background(Color(nsColor: .windowBackgroundColor))
      .environment(\.colorScheme, dark ? .dark : .light))
    let window = NSWindow(contentRect: NSRect(origin: .zero, size: size),
      styleMask: [.borderless], backing: .buffered, defer: false)
    window.appearance = NSAppearance(named: dark ? .darkAqua : .aqua)
    window.contentView = host
    window.setFrameOrigin(NSPoint(x: -10000, y: -10000))
    window.orderBack(nil)
    defer { window.orderOut(nil); window.contentView = nil }
    try await Task.sleep(nanoseconds: 500_000_000)
    if let action {
      await action()
      try await Task.sleep(nanoseconds: 200_000_000)
    }
    host.layoutSubtreeIfNeeded()
    guard let bitmap = host.bitmapImageRepForCachingDisplay(in: host.bounds) else {
      throw CocoaError(.coderInvalidValue)
    }
    host.cacheDisplay(in: host.bounds, to: bitmap)
    guard let data = bitmap.representation(using: .png, properties: [:]) else {
      throw CocoaError(.coderInvalidValue)
    }
    try data.write(to: output.appendingPathComponent(name + ".png"))
  }
}


private struct BurstCellsFixture: NSViewRepresentable {
  final class Coordinator { var cells: [PhotoCell] = [] }
  func makeCoordinator() -> Coordinator { Coordinator() }
  func makeNSView(context: Context) -> NSView {
    let panel = NSView(frame: NSRect(x: 0, y: 0, width: 600, height: 230))
    for (index, state) in ["received", "partial", "failed"].enumerated() {
      let cell = PhotoCell()
      cell.view.frame = NSRect(x: 12 + index * 196, y: 16, width: 184, height: 198)
      cell.view.layer?.backgroundColor = [NSColor.systemTeal, .systemIndigo, .systemBrown][index].cgColor
      cell.setGroup(count: [8, 32, 1200][index], state: state)
      cell.isSelected = index == 1
      panel.addSubview(cell.view)
      cell.viewDidLayout()
      context.coordinator.cells.append(cell)
    }
    return panel
  }
  func updateNSView(_ view: NSView, context: Context) {}
}
