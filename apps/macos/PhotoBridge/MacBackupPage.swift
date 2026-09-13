import SwiftUI

struct MacBackupPage: View {
  @ObservedObject var model: BackupModel
  var pair: () -> Void
  var library: () -> Void
  var showTransfers: (String) -> Void

  var body: some View {
    ScrollView {
      VStack(alignment: .leading, spacing: 24) {
        VStack(alignment: .leading, spacing: 8) {
          Text("backup_overview_heading").font(.title2.weight(.medium))
          if let peer = model.peerDevice {
            Text(String(format: NSLocalizedString("device_sending_to", comment: ""), peer.name))
              .font(.headline)
          }
          Text("backup_overview_description").foregroundStyle(.secondary)
        }
        status
        if model.pairing != nil && model.ready {
          LazyVGrid(columns: [GridItem(.adaptive(minimum: 140), spacing: 12)], spacing: 12) {
            metric("preparing", value: model.pendingImports)
            metric("running", value: model.summary.running)
            metric("paused", value: model.summary.paused)
            metric("scanned", value: model.historicalImport?.checked ?? 0)
            metric("received", value: model.summary.received)
            metric("queued", value: model.summary.queued)
            metric("waiting", value: model.summary.waiting)
            metric("failed", value: model.summary.failed)
          }
          VStack(alignment: .leading, spacing: 0) {
            HStack {
              Button("backup_all_tasks") { showTransfers("all") }
              Spacer()
              Button("backup_choose_library", action: library)
            }
          }
        }
      }.frame(maxWidth: 960, alignment: .leading)
        .frame(maxWidth: .infinity, alignment: .leading).padding(28)
    }
  }

  private var status: some View {
    MacBackupSurface {
      VStack(alignment: .leading, spacing: 18) {
        HStack(alignment: .top, spacing: 16) {
          Image(systemName: model.pairing == nil ? "externaldrive.badge.plus" : "checkmark.shield")
            .font(.system(size: 30, weight: .regular)).foregroundStyle(.tint)
          VStack(alignment: .leading, spacing: 8) {
            Text(model.pairing == nil ? "receiver_unpaired" : model.paused ? "backup_paused" : "backup_enabled")
              .font(.title3.weight(.medium))
            if !model.ready {
              if let message = model.message {
                Text(message).foregroundStyle(.secondary)
                Button("retry_task") { Task { await model.open() } }
              } else { ProgressView("backup_initializing").controlSize(.small) }
            } else if model.pairing == nil {
              Text("backup_pair_first").foregroundStyle(.secondary)
              Button("pair_receiver_desktop", action: pair).buttonStyle(.borderedProminent)
            } else if model.summary.total == 0 {
              Text("tasks_empty").foregroundStyle(.secondary)
              Button("backup_choose_library", action: library).buttonStyle(.borderedProminent)
            } else {
              Text(String(format: NSLocalizedString("transfer_summary", comment: ""),
                model.summary.received, model.summary.total)).foregroundStyle(.secondary)
            }
          }
          Spacer(minLength: 0)
          if model.ready && model.pairing != nil {
            Button { Task { await model.setPaused(!model.paused) } } label: {
              Label(model.paused ? "resume_backup" : "pause_backup", systemImage: model.paused ? "play" : "pause")
            }.buttonStyle(.bordered)
          }
        }
        if model.pairing != nil && model.summary.total > 0 {
          ProgressView(value: Double(model.summary.received), total: Double(max(1, model.summary.total)))
            .accessibilityLabel("transfer_summary_progress")
        }
        if model.waitingReason != nil || model.pendingImports > 0 { WaitingStatus(model: model) }
        if model.waitingForNetwork {
          Label("waiting_for_wifi", systemImage: "wifi.exclamationmark").foregroundStyle(.secondary)
        }
        if model.ready, let message = model.message {
          Text(message).font(.callout).foregroundStyle(.secondary).textSelection(.enabled)
        }
      }
    }
  }

  private func metric(_ state: String, value: Int) -> some View {
    Button { showTransfers(state) } label: {
      VStack(alignment: .leading, spacing: 10) {
        Label(LocalizedStringKey(state == "scanned" ? "history_title" : "state_" + state), systemImage: taskSymbol(state))
          .font(.callout).foregroundStyle(.secondary)
          .frame(height: 34, alignment: .topLeading)
        Text(value.formatted()).font(.largeTitle).monospacedDigit()
      }.frame(maxWidth: .infinity, minHeight: 76, alignment: .leading).padding(16)
        .background(.background, in: RoundedRectangle(cornerRadius: 12))
        .overlay { RoundedRectangle(cornerRadius: 12).stroke(.quaternary, lineWidth: 1) }
        .contentShape(RoundedRectangle(cornerRadius: 12))
    }.buttonStyle(.plain).accessibilityIdentifier("backup.filter.\(state)")
  }
}

private struct MacBackupSurface<Content: View>: View {
  @ViewBuilder var content: () -> Content
  var body: some View {
    content().frame(maxWidth: .infinity, alignment: .leading).padding(22)
      .background(.background, in: RoundedRectangle(cornerRadius: 14))
      .overlay { RoundedRectangle(cornerRadius: 14).stroke(.quaternary, lineWidth: 1) }
  }
}
