import SwiftUI

struct IOSReceiverPage: View {
  @ObservedObject var model: BackupModel
  @State private var scanner = false
  var body: some View {
    NavigationStack {
      List {
        Section {
          VStack(alignment: .leading, spacing: 16) {
            Image(systemName: "externaldrive.badge.wifi").font(.largeTitle).foregroundStyle(.tint)
            if let peer = model.peerDevice { Text(peer.name).font(.title2) }
            ReceiverStatusIndicator(model: model)
            Text(
              model.pairing == nil ? "receiver_pair_instructions" : "receiver_direct_description"
            )
            .foregroundStyle(.secondary)
          }.padding(.vertical, 12)
          Button {
            scanner = true
          } label: {
            Label(
              model.pairing == nil ? "pair_receiver" : "pair_another_receiver",
              systemImage: "qrcode.viewfinder")
          }.accessibilityIdentifier("receiver.scan")
        } footer: {
          Text("receiver_local_network")
        }
        if let pairing = model.pairing {
          Section {
            LabeledContent("receiver_address", value: pairing.endpoint)
              .font(.callout).textSelection(.enabled)
          } header: {
            Text("receiver_connection_details")
          }
        }
        Section { Text("receipt_explanation").foregroundStyle(.secondary) }
      }
      .navigationTitle("nav_receiver")
      .sheet(isPresented: $scanner) {
        PairingScanner { payload in
          scanner = false
          Task { await model.pair(payload) }
        }
      }
    }
  }
}
