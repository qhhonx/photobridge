import AppKit
import Combine
import SwiftUI
import Sparkle

@MainActor final class AppUpdater: NSObject, ObservableObject, SPUUpdaterDelegate {
  static let shared = AppUpdater()
  @Published private(set) var canCheck = false
  @Published private(set) var automaticChecks = true
  private var controller: SPUStandardUpdaterController!
  private var observations: [NSKeyValueObservation] = []
  private var started = false
  private let resumeKey = "resumeAfterAppUpdate"

  override init() {
    super.init()
    controller = SPUStandardUpdaterController(startingUpdater: false, updaterDelegate: self, userDriverDelegate: nil)
    observations = [
      controller.updater.observe(\.canCheckForUpdates, options: [.initial, .new]) { [weak self] value, _ in
        Task { @MainActor in self?.canCheck = value.canCheckForUpdates }
      },
      controller.updater.observe(\.automaticallyChecksForUpdates, options: [.initial, .new]) { [weak self] value, _ in
        Task { @MainActor in self?.automaticChecks = value.automaticallyChecksForUpdates }
      }
    ]
  }
  func start() async {
    guard !started else { return }
    started = true
    if UserDefaults.standard.bool(forKey: resumeKey), BackupModel.shared.ready {
      await BackupModel.shared.setPaused(false)
      if !BackupModel.shared.paused { UserDefaults.standard.removeObject(forKey: resumeKey) }
    }
    controller.startUpdater()
  }
  func check() { controller.checkForUpdates(nil) }
  func setAutomaticChecks(_ value: Bool) { controller.updater.automaticallyChecksForUpdates = value }
  func updater(_ updater: SPUUpdater, shouldPostponeRelaunchForUpdate item: SUAppcastItem,
    untilInvokingBlock installHandler: @escaping () -> Void) -> Bool {
    Task { @MainActor in
      let model = BackupModel.shared
      let resume = model.ready && !model.paused
      if model.ready { await model.setPaused(true) }
      // Do not replace the process while PhotoKit is writing an original.
      while model.importing || model.scanningHistory {
        try? await Task.sleep(nanoseconds: 200_000_000)
      }
      UserDefaults.standard.set(resume, forKey: resumeKey)
      installHandler()
    }
    return true
  }
}

struct UpdateSettings: View {
  @ObservedObject private var updater = AppUpdater.shared
  var body: some View {
    Section("updates_title") {
      Toggle("updates_automatic", isOn: Binding(get: { updater.automaticChecks }, set: updater.setAutomaticChecks))
      Button("updates_check", action: updater.check).disabled(!updater.canCheck)
      Text("updates_mac_note").foregroundStyle(.secondary)
      Link("official_website", destination: URL(string: "https://photobridge-app.vercel.app")!)
    }
  }
}
