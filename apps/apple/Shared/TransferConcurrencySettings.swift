import SwiftUI

struct TransferConcurrencySettings: View {
  @ObservedObject var model: BackupModel
  var body: some View {
    Section {
      Picker("transfer_concurrency", selection: Binding(
        get: { model.concurrentUploads },
        set: { value in Task { await model.setConcurrentUploads(value) } })) {
        ForEach(1...4, id: \.self) { Text($0.formatted()).tag($0) }
      }.accessibilityIdentifier("settings.transfer_concurrency")
        .disabled(!model.ready || model.savingConcurrency)
    } header: {
      Text("transfer_settings")
    } footer: {
      Text("transfer_concurrency_note")
    }
  }
}
