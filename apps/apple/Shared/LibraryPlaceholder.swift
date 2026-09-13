import Photos
import SwiftUI
#if os(macOS)
  import AppKit
#else
  import UIKit
#endif

enum LibraryPlaceholderKind {
  case loading
  case permission(PHAuthorizationStatus)
  case empty(filtered: Bool)
}

/// Shared first-run and empty-state content. Actions follow current permission
/// state; a denied request cannot display the system authorization dialog again.
struct LibraryPlaceholder: View {
  let kind: LibraryPlaceholderKind
  var requestingAccess = false
  var requestAccess: () -> Void = {}
  var clearFilter: () -> Void = {}
  @State private var settingsUnavailable = false

  var body: some View {
    ScrollView {
      VStack(spacing: 16) {
        switch kind {
        case .loading:
          ProgressView().controlSize(.large)
          Text("library_loading").foregroundStyle(.secondary)
        case .permission(let access):
          Image(systemName: access == .restricted ? "lock.shield" : "photo.on.rectangle")
            .font(.system(size: 40)).foregroundStyle(.secondary).accessibilityHidden(true)
          Text("photos_access_title").font(.title2.weight(.medium))
          Text(permissionDescription(access)).foregroundStyle(.secondary)
          if access == .denied {
            Button("photos_open_settings") {
              Task { settingsUnavailable = !(await openPhotoSettings()) }
            }.buttonStyle(.borderedProminent).accessibilityIdentifier("photos.settings")
          } else if access == .notDetermined {
            Button("photos_allow", action: requestAccess).buttonStyle(.borderedProminent)
              .disabled(requestingAccess)
              .accessibilityIdentifier("photos.request")
            if requestingAccess {
              ProgressView("photos_waiting_permission").controlSize(.small)
                .accessibilityIdentifier("photos.permission_pending")
            }
          }
          if access == .denied, settingsUnavailable {
            Text("photos_settings_unavailable").foregroundStyle(.secondary)
          }
        case .empty(let filtered):
          Image(systemName: "photo").font(.system(size: 40)).foregroundStyle(.secondary)
            .accessibilityHidden(true)
          Text(filtered ? "library_empty" : "library_empty_all").font(.title2.weight(.medium))
          Text(filtered ? "library_empty_filter_description" : "library_empty_description")
            .foregroundStyle(.secondary)
          if filtered {
            Button("library_show_all", action: clearFilter).buttonStyle(.bordered)
              .accessibilityIdentifier("library.clear_filter")
          }
        }
      }
      .multilineTextAlignment(.center)
      .frame(maxWidth: 440).padding(.horizontal, 24).padding(.vertical, 48)
      .frame(maxWidth: .infinity, alignment: .top)
    }.accessibilityIdentifier("library.placeholder")
  }

  private func permissionDescription(_ access: PHAuthorizationStatus) -> LocalizedStringKey {
    if access == .restricted { return "photos_access_restricted" }
    if access == .denied {
      #if os(macOS)
        return "photos_access_denied_mac"
      #else
        return "photos_access_denied"
      #endif
    }
    return "photos_access_description"
  }

  @MainActor private func openPhotoSettings() async -> Bool {
    #if os(macOS)
      if let url = URL(string: "x-apple.systempreferences:com.apple.preference.security?Privacy_Photos"),
        NSWorkspace.shared.open(url) { return true }
      guard let application = NSWorkspace.shared.urlForApplication(
        withBundleIdentifier: "com.apple.systempreferences") else { return false }
      return NSWorkspace.shared.open(application)
    #else
      guard let url = URL(string: UIApplication.openSettingsURLString) else { return false }
      return await UIApplication.shared.open(url)
    #endif
  }
}
