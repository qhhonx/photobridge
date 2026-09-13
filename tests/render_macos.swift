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
        let overview = { AnyView(MacBackupPage(model: model, pair: {}, library: {}, showTransfers: { _ in })) }
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
