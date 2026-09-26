import SwiftUI

/// Shared sender guidance; storage and cloud policies belong to the receiver.
struct BackupHelp: View {
  var body: some View {
    Form {
      topic("help_start_title", "help_start_body")
      #if os(macOS)
        topic("mac_nav_library", "mac_help_library")
        topic("mac_nav_sources", "mac_help_folders")
        topic("mac_all_sources", "mac_global_pause_note")
      #else
        topic("help_scope_title", "help_scope_body")
      #endif
      Section {
        #if os(iOS)
          Text("background_explanation")
          Text("background_force_quit")
        #else
          Text("mac_running_note")
        #endif
      } header: { Text("background_section") }
      #if os(macOS)
        topic("help_receipt_title", "mac_help_receipt")
      #else
        topic("help_receipt_title", "help_receipt_body")
      #endif
      topic("help_recover_title", "help_recover_body")
      #if os(macOS)
        topic("help_permission_title", "mac_help_permissions")
      #else
        topic("help_permission_title", "help_permission_body")
      #endif
      topic("help_space_title", "help_space_body")
      topic("help_support_title", "help_support_body")
    }
    .formStyle(.grouped)
    .navigationTitle("help_title")
    #if os(iOS)
      .navigationBarTitleDisplayMode(.inline)
    #endif
    .accessibilityIdentifier("settings.help.content")
  }

  private func topic(_ title: LocalizedStringKey, _ body: LocalizedStringKey) -> some View {
    Section { Text(body).textSelection(.enabled) } header: { Text(title) }
  }
}
