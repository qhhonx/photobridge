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
            Label(
              model.pairing == nil
                ? "receiver_unpaired" : model.paused ? "backup_paused" : "backup_enabled",
              systemImage: model.pairing == nil
                ? "externaldrive.badge.plus" : model.paused ? "pause.circle" : "checkmark.shield"
            ).font(.title3.weight(.medium))
            if model.summary.total > 0 {
              Text(
                String(
                  format: NSLocalizedString("transfer_summary", comment: ""),
                  model.summary.received, model.summary.total)
              ).foregroundStyle(.secondary)
              ProgressView(
                value: Double(model.summary.received), total: Double(max(1, model.summary.total))
              )
              .accessibilityLabel("transfer_summary_progress")
            } else {
              Text(model.pairing == nil ? "backup_pair_first" : "tasks_empty").foregroundStyle(
                .secondary)
            }
            if model.pairing != nil {
              Button {
                Task { await model.setPaused(!model.paused) }
              } label: {
                HStack(spacing: 8) {
                  Image(systemName: model.paused ? "play" : "pause").renderingMode(.template)
                  Text(model.paused ? "resume_backup" : "pause_backup")
                }.foregroundStyle(.white)
              }.buttonStyle(.borderedProminent).disabled(!model.ready)
            }
          }.padding(.vertical, 8)
          if model.waitingReason != nil || model.pendingImports > 0 {
            WaitingStatus(model: model)
          }
          if model.waitingForNetwork {
            Label("waiting_for_wifi", systemImage: "wifi.exclamationmark").foregroundStyle(
              .secondary)
          }
          if let message = model.message {
            Text(message).font(.callout).foregroundStyle(.secondary).textSelection(.enabled)
          }
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
                .navigationTitle(LocalizedStringKey(state == "scanned" ? "history_title" : "state_" + state))
                .navigationBarTitleDisplayMode(.inline)
            } label: {
              HStack {
                Label(LocalizedStringKey(state == "scanned" ? "history_title" : "state_" + state), systemImage: taskSymbol(state))
                Spacer()
                Text(count(state).formatted()).monospacedDigit().foregroundStyle(.secondary)
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
  private func count(_ state: String) -> Int {
    switch state {
    case "preparing": model.pendingImports
    case "scanned": model.historicalImport?.checked ?? 0
    case "paused": model.summary.paused
    case "running": model.summary.running
    case "queued": model.summary.queued
    case "waiting": model.summary.waiting
    case "failed": model.summary.failed
    default: model.summary.received
    }
  }
}
