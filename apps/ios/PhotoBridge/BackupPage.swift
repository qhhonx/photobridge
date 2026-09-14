import SwiftUI

/// An overview of the durable queue. Full history is queried on its own page.
struct IOSBackupPage: View {
  @ObservedObject var model: BackupModel
  @State private var picker = false
  var body: some View {
    NavigationStack {
      List {
        Section {
          VStack(alignment: .leading, spacing: 16) {
            if let peer = model.peerDevice {
              Text(String(format: NSLocalizedString("device_sending_to", comment: ""), peer.name))
                .font(.headline)
            }
            HStack {
              BackupStatusIndicator(model: model)
              Spacer()
              if model.pairing != nil {
                Button { Task { await model.setPaused(!model.paused) } } label: {
                  Label(model.paused ? "resume_backup" : "pause_backup",
                    systemImage: model.paused ? "play" : "pause")
                }.buttonStyle(.bordered).disabled(!model.ready)
              }
            }
            if model.summary.total > 0 {
              Text(
                String(
                  format: NSLocalizedString("transfer_summary", comment: ""),
                  model.summary.received, model.summary.total)
              ).foregroundStyle(.secondary)

            } else {
              Text(model.pairing == nil ? "backup_pair_first" : "tasks_empty").foregroundStyle(
                .secondary)
            }
          }.padding(.vertical, 8)
        }
        Section {
          NavigationLink {
            TransferList(model: model).padding(.horizontal, 20).padding(.top, 12)
              .navigationTitle("transfer_tasks").navigationBarTitleDisplayMode(.inline)
          } label: {
            Label("backup_all_tasks", systemImage: "list.bullet")
          }.accessibilityIdentifier("backup.all_tasks")
          ForEach(["preparing", "running", "queued", "waiting", "paused", "failed", "received", "scanned"], id: \.self) { state in
            NavigationLink {
              TransferList(model: model, filter: state).padding(.horizontal, 20).padding(.top, 12)
                .navigationTitle(LocalizedStringKey(state == "scanned" ? "task_title_scanned" : "state_" + state))
                .navigationBarTitleDisplayMode(.inline)
            } label: {
              HStack {
                Label(LocalizedStringKey(state == "scanned" ? "task_title_scanned" : "state_" + state), systemImage: taskSymbol(state))
                Spacer()
                Text(model.transferCount(for: state).formatted()).monospacedDigit().foregroundStyle(.secondary)
              }
            }.accessibilityIdentifier("backup.filter.\(state)")
          }
        } header: {
          Text("transfer_tasks")
        }
      }
      .safeAreaPadding(.bottom, 12)
      .navigationTitle("nav_backup")
      .toolbar {
        ToolbarItem(placement: .topBarTrailing) {
          Button {
            Task { if await model.authorizePhotos() { picker = true } }
          } label: {
            Label("choose_photos", systemImage: "plus")
          }
          .disabled(model.pairing == nil || model.importing)
        }
      }
      .sheet(isPresented: $picker) {
        LibraryPicker { identifiers in
          picker = false
          Task { await model.importAssets(identifiers) }
        }
      }
    }
  }
}
