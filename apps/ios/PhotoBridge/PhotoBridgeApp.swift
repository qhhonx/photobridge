import SwiftUI

@main struct PhotoBridgeApp: App {
  @UIApplicationDelegateAdaptor(AppDelegate.self) private var appDelegate
  @StateObject private var model = BackupModel.shared
  @StateObject private var library = PhotoLibraryModel()
  @Environment(\.scenePhase) private var scenePhase
  var body: some Scene {
    WindowGroup {
      TabView {
        IOSLibraryPage(library: library, model: model).tabItem {
          Label("nav_library", systemImage: "photo.on.rectangle")
        }
        IOSBackupPage(model: model).tabItem {
          Label("nav_backup", systemImage: "arrow.triangle.2.circlepath")
        }
        IOSReceiverPage(model: model).tabItem {
          Label("nav_receiver", systemImage: "externaldrive.badge.wifi")
        }
        IOSSettingsPage(model: model).tabItem {
          Label("nav_settings", systemImage: "gearshape")
        }
      }.task { await model.open() }
        .onChange(of: scenePhase) { _, phase in
          if phase == .active {
            BackgroundTransfer.shared.enteredForeground()
            Task { await library.open() }
            Task { await model.becameActive() }
          }
          if phase == .background {
            BackgroundTransfer.shared.enteredBackground()
            model.scheduleBackgroundWork()
          }
        }
    }
  }
}
