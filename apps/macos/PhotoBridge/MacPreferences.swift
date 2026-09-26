import SwiftUI

enum MacSettingsSection: String { case backup, cache, diagnostics, help }

/// The Settings window and sidebar destination expose the same categories.
struct MacPreferences: View {
  @ObservedObject var model: BackupModel
  @ObservedObject var folders: FolderSources = .shared
  var showFolders: (() -> Void)? = nil
  @Environment(\.openWindow) private var openWindow
  @AppStorage("macSettingsSection") private var section = MacSettingsSection.backup
  @State private var logs = false
  @AppStorage("macListThumbnails") private var thumbnails = true

  var body: some View {
    TabView(selection: $section) {
      Form {
        Section("device_identity") { DeviceIdentitySettings(model: model) }
        Section {
          Toggle("auto_backup_new", isOn: Binding(
            get: { model.autoBackup },
            set: { enabled in Task { await model.setAutoBackup(enabled) } }))
            .disabled(!model.ready || model.pairing == nil)
          Text("mac_library_automatic_note").foregroundStyle(.secondary)
          if model.pairing == nil {
            Label("mac_pair_first", systemImage: "externaldrive.badge.plus")
              .foregroundStyle(.secondary)
          }
          if model.discoveryPending > 0 {
            LabeledContent("discovery_pending", value: model.discoveryPending.formatted())
          }
          if model.historyUnavailable {
            Text("error_history_unavailable").foregroundStyle(.orange)
          }
        } header: { Text("mac_nav_library") }
        HistoricalImportSettings(model: model, title: "mac_library_existing", explanation: "mac_library_existing_note")
        Section("mac_nav_sources") {
          Text("mac_folder_settings_note").foregroundStyle(.secondary)
          ForEach(folders.sources) { source in
            Toggle(folders.displayName(source), isOn: Binding(get: { source.automaticActive },
              set: { folders.setAutomatic(source.id, $0) }))
              .help(folders.displayPath(source))
          }
          Button("mac_manage_folders") {
            if let showFolders { showFolders() }
            else {
              UserDefaults.standard.set("sources", forKey: "macRequestedDestination")
              openWindow(id: "main")
              NotificationCenter.default.post(name: Notification.Name("PhotoBridgeShowFolders"), object: nil)
            }
          }
        }
        Section("mac_all_sources") {
          Toggle("mac_global_pause", isOn: Binding(get: { model.paused },
            set: { value in Task { await model.setPaused(value) } }))
            .disabled(!model.ready || model.pairing == nil)
          Text("mac_global_pause_note").foregroundStyle(.secondary)
        }
        TransferConcurrencySettings(model: model)
        Section("list_display_settings") {
          Toggle("list_thumbnails", isOn: $thumbnails)
          Text("list_thumbnails_hint").foregroundStyle(.secondary)
        }
        UpdateSettings()
        Section {
          Label("mac_running_note", systemImage: "desktopcomputer")
            .foregroundStyle(.secondary)
        }
        Section {
          LabeledContent("settings_version", value:
            Bundle.main.object(forInfoDictionaryKey: "CFBundleShortVersionString") as? String ?? "—")
          Text("settings_about_description").foregroundStyle(.secondary)
        } header: { Text("PhotoBridge") }
      }.formStyle(.grouped)
        .tabItem { Label("backup_settings", systemImage: "arrow.triangle.2.circlepath") }
        .tag(MacSettingsSection.backup)

      Form { Section { StoragePreferences(model: model, scope: .cache) } }
        .formStyle(.grouped)
        .tabItem { Label("settings_cache", systemImage: "internaldrive") }
        .tag(MacSettingsSection.cache)

      Form {
        Section {
          Button { logs = true } label: {
            Label("activity_log", systemImage: "list.bullet.rectangle")
          }.accessibilityIdentifier("settings.activity_log")
          Text("logs_privacy_note").foregroundStyle(.secondary)
        }
        Section { StoragePreferences(model: model, scope: .logs) }
      }.formStyle(.grouped)
        .tabItem { Label("settings_diagnostics", systemImage: "waveform.path") }
        .tag(MacSettingsSection.diagnostics)

      BackupHelp()
        .tabItem { Label("help_title", systemImage: "questionmark.circle") }
        .tag(MacSettingsSection.help)
    }.sheet(isPresented: $logs) { ActivityLogView() }
  }
}
