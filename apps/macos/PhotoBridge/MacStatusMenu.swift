import AppKit
import SwiftUI

/// A compact snapshot of the same persistent queue used by the main window.
/// Never start another worker when the menu opens.
@MainActor struct MacMenuStatus {
  let key: String
  let symbol: String

  init(model: BackupModel) {
    if !model.ready {
      key = model.message == nil ? "backup_initializing" : "state_failed"
      symbol = "photo.stack"
    } else if model.pairing == nil {
      key = "receiver_unpaired"
      symbol = "photo.stack"
    } else if model.paused {
      key = "backup_paused"
      symbol = "pause.circle"
    } else if let reason = model.waitingReason {
      key = "error_" + reason
      symbol = "clock"
    } else if model.importing {
      key = "importing_originals"
      symbol = "arrow.up.circle"
    } else if model.summary.running > 0 {
      key = "state_running"
      symbol = "arrow.up.circle"
    } else if model.summary.queued > 0 {
      key = "state_queued"
      symbol = "clock"
    } else if model.summary.waiting > 0 {
      key = "state_waiting"
      symbol = "clock"
    } else if model.summary.failed > 0 {
      key = "state_failed"
      symbol = "exclamationmark.circle"
    } else {
      key = "menu_backup_ready"
      symbol = "photo.stack"
    }
  }
}

struct MacStatusMenu: View {
  @ObservedObject var model: BackupModel
  @ObservedObject private var updater = AppUpdater.shared
  @Environment(\.openWindow) private var openWindow
  @State private var changingPause = false

  var body: some View {
    Text(LocalizedStringKey(MacMenuStatus(model: model).key))
    if let peer = model.peerDevice, model.pairing != nil {
      Text(String(format: NSLocalizedString("device_sending_to", comment: ""), peer.name))
    }
    if model.ready, model.pairing != nil {
      Text(String(format: NSLocalizedString("transfer_summary", comment: ""),
        model.summary.received, model.summary.total))
    }
    Divider()
    Button("menu_open") {
      openWindow(id: "main")
      NSApp.activate(ignoringOtherApps: true)
    }.keyboardShortcut("o", modifiers: .command)
    Button(model.paused ? "resume_backup" : "pause_backup") {
      guard !changingPause else { return }
      changingPause = true
      Task {
        await model.setPaused(!model.paused)
        changingPause = false
      }
    }.disabled(!model.ready || model.pairing == nil || changingPause)
    Divider()
    SettingsLink { Text("nav_settings") }
    Button("updates_check", action: updater.check).disabled(!updater.canCheck)
    Divider()
    Button("menu_quit") { NSApp.terminate(nil) }.keyboardShortcut("q")
  }
}
