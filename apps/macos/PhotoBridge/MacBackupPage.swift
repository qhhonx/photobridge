import SwiftUI

struct MacBackupPage: View {
  @ObservedObject var model: BackupModel
  var pair: () -> Void
  var library: () -> Void
  var sources: () -> Void
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
              Button("nav_sources", action: sources)
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
          VStack(alignment: .leading, spacing: 8) {
            BackupStatusIndicator(model: model)
            if !model.ready {
              if model.message != nil {
                Button("retry_task") { Task { await model.open() } }
              } else { Text("backup_initializing").foregroundStyle(.secondary) }
            } else if model.pairing == nil {
              Text("mac_pair_first").foregroundStyle(.secondary)
              Button("pair_receiver_desktop", action: pair).buttonStyle(.borderedProminent)
            } else if model.summary.total == 0 {
              Text("tasks_empty").foregroundStyle(.secondary)
              Button("nav_sources", action: sources).buttonStyle(.borderedProminent)
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
      }
    }
  }

  private func metric(_ state: String, value: Int) -> some View {
    Button { showTransfers(state) } label: {
      VStack(alignment: .leading, spacing: 10) {
        Label(LocalizedStringKey(state == "scanned" ? "task_title_scanned" : "state_" + state), systemImage: taskSymbol(state))
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
