import SwiftUI

enum MacSettingsSection: String { case backup, cache, diagnostics, help }

/// The Settings window and sidebar destination expose the same categories.
struct MacPreferences: View {
  @ObservedObject var model: BackupModel
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
          Text("auto_backup_explanation").foregroundStyle(.secondary)
          if model.pairing == nil {
            Label("backup_pair_first", systemImage: "externaldrive.badge.plus")
              .foregroundStyle(.secondary)
          }
          if model.discoveryPending > 0 {
            LabeledContent("discovery_pending", value: model.discoveryPending.formatted())
          }
          if model.historyUnavailable {
            Text("error_history_unavailable").foregroundStyle(.orange)
          }
        } header: { Text("backup_settings") }
        TransferConcurrencySettings(model: model)
        HistoricalImportSettings(model: model)
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
