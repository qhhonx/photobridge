import AppKit
import CoreImage.CIFilterBuiltins
import Darwin
import SwiftUI

@main struct PhotoBridgeMacApp: App {
  @NSApplicationDelegateAdaptor(MacAppDelegate.self) private var delegate
  @StateObject private var model = BackupModel.shared
  @StateObject private var library = PhotoLibraryModel()
  var body: some Scene {
    WindowGroup {
      MacWorkspace(model: model, library: library)
        .frame(minWidth: 900, minHeight: 620)
        .task { await model.open(); await AppUpdater.shared.start() }
        .task { await library.open() }
        .onReceive(NotificationCenter.default.publisher(for: NSApplication.didBecomeActiveNotification)) { _ in
          Task { await library.open() }
        }
    }.defaultSize(width: 1180, height: 800)
      .commands {
        CommandGroup(after: .appInfo) {
          Button("updates_check") { AppUpdater.shared.check() }
        }
      }
    Settings {
      MacPreferences(model: model).frame(width: 620, height: 580)
    }
  }
}
final class MacAppDelegate: NSObject, NSApplicationDelegate {
  func applicationShouldTerminateAfterLastWindowClosed(_ sender: NSApplication) -> Bool { false }
  func applicationShouldHandleReopen(_ sender: NSApplication, hasVisibleWindows flag: Bool) -> Bool
  {
    if !flag { sender.windows.first?.makeKeyAndOrderFront(nil) }
    return true
  }
}

enum MacDestination: String, CaseIterable, Identifiable {
  case library, backup, transfers, receiver, settings
  var id: String { rawValue }
  var title: String { "nav_" + rawValue }
  var icon: String {
    switch self {
    case .library: "photo.on.rectangle"
    case .backup: "checkmark.shield"
    case .transfers: "arrow.up.arrow.down"
    case .receiver: "externaldrive.badge.wifi"
    case .settings: "gearshape"
    }
  }
}
struct MacWorkspace: View {
  @ObservedObject var model: BackupModel
  @ObservedObject var library: PhotoLibraryModel
  @State private var destination: MacDestination? = .library
  @State private var tileSize: Double = 170
  @State private var pairSheet = false
  @State private var transferFilter = "all"
  @StateObject private var activeTransfers = TaskBrowserModel()
  init(model: BackupModel, library: PhotoLibraryModel, initialDestination: MacDestination = .library) {
    self.model = model
    self.library = library
    _destination = State(initialValue: initialDestination)
  }
  var body: some View {
    NavigationSplitView {
      VStack(alignment: .leading, spacing: 0) {
        Label("PhotoBridge", systemImage: "photo.stack").font(.title3.weight(.medium)).padding(
          .horizontal, 20
        ).padding(.vertical, 28)
        List(selection: $destination) {
          Section("sidebar_library") {
            ForEach([MacDestination.library, .backup, .transfers]) { item in
              Label(LocalizedStringKey(item.title), systemImage: item.icon).tag(item)
            }
          }
          Section("sidebar_manage") {
            ForEach([MacDestination.receiver, .settings]) { item in
              Label(LocalizedStringKey(item.title), systemImage: item.icon).tag(item)
            }
          }
        }.listStyle(.sidebar)
        VStack(alignment: .leading, spacing: 8) {
          Label(
            model.pairing == nil
              ? "receiver_unpaired" : model.paused ? "backup_paused" : "backup_enabled",
            systemImage: model.paused ? "pause.circle" : "checkmark.shield")
          Text(model.peerDevice?.name ?? NSLocalizedString(model.pairing == nil ? "receiver_unpaired" : "receiver_paired", comment: "")).font(.caption)
            .foregroundStyle(.secondary)
        }.font(.callout).padding(20)
      }.navigationSplitViewColumnWidth(min: 195, ideal: 220, max: 270)
    } detail: {
      VStack(spacing: 0) {
        // Keep the native collection alive when switching destinations. No loss
        // of its reuse pool, selection or scroll position on tab changes.
        ZStack(alignment: .topLeading) {
          libraryPage.opacity(destination == .library ? 1 : 0).allowsHitTesting(
            destination == .library
          ).accessibilityHidden(destination != .library)
          if destination == .backup {
            MacBackupPage(model: model, pair: { pairSheet = true },
              library: { destination = .library }, showTransfers: showTransfers)
          }
          if destination == .transfers {
            TransferList(model: model, filter: transferFilter).id(transferFilter).padding(28)
          }
          if destination == .settings {
            MacPreferences(model: model).padding(20)
          }
          if destination == .receiver {
            ScrollView { ReceiverPage(model: model, pair: { pairSheet = true }).padding(32) }
          }
        }.frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
        if destination != .settings && destination != .receiver {
          Divider()
          footer.padding(.horizontal, 24).padding(.vertical, 14).background(.bar)
            .task(id: "\(model.queueRevision)|\(model.pairing?.receiverID ?? "")") {
              if model.ready, let receiver = model.pairing?.receiverID {
                await activeTransfers.refresh(filter: "running", receiver: receiver)
              }
            }
        }
      }
      .navigationTitle(LocalizedStringKey((destination ?? .library).title))
      .toolbar {
        ToolbarItemGroup(placement: .primaryAction) {
          if destination == .library {
            Image(systemName: "square.grid.3x3").foregroundStyle(.secondary)
            Slider(value: $tileSize, in: 100...260, step: 20).frame(width: 95).help(
              "thumbnail_size")
          }
          if destination == .library || destination == .transfers {
            Button {
              Task { await model.setPaused(!model.paused) }
            } label: {
              Label(
                model.paused ? "resume_backup" : "pause_backup",
                systemImage: model.paused ? "play.fill" : "pause.fill")
            }.disabled(!model.ready || model.pairing == nil)
          }
        }
      }
    }
    .sheet(isPresented: $pairSheet) { MacPairingSheet(model: model) }
  }
  private func showTransfers(_ filter: String) {
    transferFilter = filter
    destination = .transfers
  }
  private var libraryPage: some View {
    VStack(alignment: .leading, spacing: 0) {
      HStack(alignment: .center) {
        VStack(alignment: .leading, spacing: 7) {
          Text("library_heading").font(.title2.weight(.medium))
          if library.authorized, library.fetch != nil {
            Text(String(format: NSLocalizedString("library_count", comment: ""), library.total))
              .foregroundStyle(.secondary)
          }
        }
        Spacer()
        Picker(
          "media_type",
          selection: Binding(
            get: { library.filter }, set: { value in Task { await library.changeFilter(value) } })
        ) {
          ForEach(LibraryFilter.allCases) { Text(LocalizedStringKey($0.title)).tag($0) }
        }.labelsHidden().frame(width: 150).disabled(!library.authorized)
        Button {
          backupSelection()
        } label: {
          Text(
            String(
              format: NSLocalizedString("backup_selected", comment: ""), library.selection.count))
        }.buttonStyle(.borderedProminent).disabled(
          library.selection.isEmpty || model.importing || model.pairing == nil)
      }.padding(.horizontal, 28).padding(.top, 28).padding(.bottom, 14)
      if let placeholder = library.placeholder {
        LibraryPlaceholder(kind: placeholder,
          requestingAccess: library.requestingPermission,
          requestAccess: { Task { await library.open(requestPermission: true) } },
          clearFilter: { Task { await library.changeFilter(.all) } })
      } else {
        MacLibraryGrid(
          library: library, preparationRevision: "\(model.pendingImports)|\(model.importingSourceID ?? "")", queueRevision: model.queueRevision,
          receiverID: model.pairing?.receiverID, itemSize: tileSize)
      }
    }
  }
  private func backupSelection() {
    let tokens = library.selection
    Task {
      let ids = await library.selectedAssetIdentifiers(tokens)
      await model.setPaused(false)
      await model.importAssets(ids)
      library.selection.subtract(tokens)
    }
  }
  private var footer: some View {
    HStack(spacing: 14) {
      if let source = model.importingSourceID
        ?? (model.summary.running > 0 && model.pairing != nil
          ? activeTransfers.jobs.first?.asset.source_id : nil)
      {
        AssetThumbnail(sourceID: source, size: 44)
      } else {
        Image(systemName: "checkmark.shield").font(.title2).foregroundStyle(.blue).frame(width: 44)
      }
      VStack(alignment: .leading, spacing: 5) {
        if model.importing {
          Text("importing_originals")
          if let progress = model.exportProgress { ProgressView(value: progress).frame(width: 200) }
        } else {
          Text(
            model.pairing == nil
              ? "receiver_unpaired"
              : model.paused
                ? "backup_paused" : model.waitingForNetwork ? "waiting_for_wifi" : "backup_enabled")
        }
        WaitingStatus(model: model)
        Text(
          model.message
            ?? String(
              format: NSLocalizedString("transfer_summary", comment: ""), model.summary.received,
              model.summary.total)
        ).font(.caption).foregroundStyle(.secondary).lineLimit(1).help(model.message ?? "")
      }
      Spacer()
      if model.pairing == nil {
        Button("pair_receiver_desktop") { pairSheet = true }
      } else {
        Button("nav_transfers") { showTransfers("all") }.buttonStyle(.link)
      }
    }
  }
}

struct ReceiverPage: View {
  @ObservedObject var model: BackupModel
  let pair: () -> Void
  var body: some View {
    VStack(alignment: .leading, spacing: 26) {
      Text("receiver_heading").font(.title2.weight(.medium))
      HStack(spacing: 20) {
        Image(systemName: "externaldrive.badge.wifi").font(.system(size: 38)).foregroundStyle(.blue)
        VStack(alignment: .leading, spacing: 8) {
          Text(model.pairing == nil ? "receiver_unpaired" : "receiver_paired").font(.headline)
          if let pairing = model.pairing {
            if let peer = model.peerDevice { Text(peer.name).font(.title2) }
            Text(pairing.endpoint).font(.callout.monospaced()).foregroundStyle(.secondary)
          }
          Text("receiver_direct_description").foregroundStyle(.secondary)
        }
        Spacer()
        Button(
          model.pairing == nil ? "pair_receiver_desktop" : "pair_another_receiver", action: pair)
      }.padding(24).background(.quaternary.opacity(0.4), in: RoundedRectangle(cornerRadius: 14))
      Text("receipt_explanation").font(.callout).foregroundStyle(.secondary)
      Spacer()
    }.frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
  }
}
struct MacPairingSheet: View {
  @ObservedObject var model: BackupModel
  @Environment(\.dismiss) private var dismiss
  @StateObject private var exchange = DesktopPairingModel()
  var body: some View {
    VStack(spacing: 18) {
      Text("pair_receiver_desktop").font(.title2.weight(.medium))
      if let profile = model.deviceSnapshot?.device { Text(profile.name).font(.headline) }
      Text("desktop_pair_instructions").foregroundStyle(.secondary).multilineTextAlignment(.center)
      if let image = exchange.image {
        Image(nsImage: image).interpolation(.none).resizable().frame(width: 300, height: 300)
          .padding(16).background(.white, in: RoundedRectangle(cornerRadius: 12))
          .accessibilityLabel(Text("desktop_pair_qr"))
        Text("desktop_pair_expiry").font(.caption).foregroundStyle(.secondary)
      } else if let error = exchange.error {
        ContentUnavailableView(LocalizedStringKey(error), systemImage: "wifi.exclamationmark")
        Button("desktop_pair_regenerate") { exchange.begin(model: model, finished: { dismiss() }) }
      } else {
        ProgressView().frame(height: 300)
      }
      Button("cancel") {
        exchange.stop()
        dismiss()
      }.keyboardShortcut(.cancelAction)
    }.padding(30).frame(width: 450)
      .onAppear { exchange.begin(model: model, finished: { dismiss() }) }
      .onDisappear { exchange.stop() }
  }
}
@MainActor final class DesktopPairingModel: ObservableObject {
  @Published var image: NSImage?
  @Published var error: String?
  private var worker: Task<Void, Never>?
  func begin(model: BackupModel, finished: @escaping () -> Void) {
    worker?.cancel()
    image = nil
    error = nil
    worker = Task {
      var sessionID: String?
      defer {
        if let sessionID {
          Task {
            _ = try? await Bridge.call(["op": "stop_desktop_pairing", "session_id": sessionID])
          }
        }
      }
      do {
        guard let ip = Self.localIPv4() else {
          error = "desktop_pair_no_network"
          return
        }
        let data = try await Bridge.call(["op": "start_desktop_pairing", "ip": ip])
        let invite = try JSONSerialization.jsonObject(with: data) as? [String: Any]
        sessionID = (invite?["connection"] as? [String: Any])?["receiver_id"] as? String
        guard !Task.isCancelled else { return }
        let filter = CIFilter.qrCodeGenerator()
        filter.message = data
        filter.correctionLevel = "M"
        guard let output = filter.outputImage,
          let cg = CIContext().createCGImage(output, from: output.extent)
        else {
          error = "desktop_pair_failed"
          return
        }
        image = NSImage(cgImage: cg, size: NSSize(width: cg.width, height: cg.height))
        while !Task.isCancelled {
          try await Task.sleep(nanoseconds: 800_000_000)
          let status = try await Bridge.call(["op": "desktop_pairing_status"])
          let envelope = try JSONSerialization.jsonObject(with: status) as! [String: Any]
          if let pairing = envelope["pairing"] as? [String: Any] {
            image = nil
            let encoded = try JSONSerialization.data(withJSONObject: pairing)
            let identity = pairing["receiver_id"] as? String
            await model.pair(String(decoding: encoded, as: UTF8.self))
            guard !Task.isCancelled else { return }
            if model.pairing?.receiverID == identity {
              finished()
            } else {
              error = "desktop_pair_failed"
            }
            return
          }
          if envelope["expired"] as? Bool == true {
            image = nil
            error = "desktop_pair_expired"
            return
          }
        }
      } catch {
        if !Task.isCancelled {
          image = nil
          self.error = "desktop_pair_failed"
        }
      }
    }
  }
  func stop() {
    worker?.cancel()
    worker = nil
    image = nil
  }
  private static func localIPv4() -> String? {
    var pointer: UnsafeMutablePointer<ifaddrs>?
    guard getifaddrs(&pointer) == 0, let first = pointer else { return nil }
    defer { freeifaddrs(pointer) }
    var candidates: [(String, String)] = []
    var next: UnsafeMutablePointer<ifaddrs>? = first
    while let entry = next {
      defer { next = entry.pointee.ifa_next }
      guard let address = entry.pointee.ifa_addr, address.pointee.sa_family == UInt8(AF_INET),
        (entry.pointee.ifa_flags & UInt32(IFF_UP)) != 0,
        (entry.pointee.ifa_flags & UInt32(IFF_LOOPBACK)) == 0
      else { continue }
      var host = [CChar](repeating: 0, count: Int(NI_MAXHOST))
      if getnameinfo(
        address, socklen_t(address.pointee.sa_len), &host, socklen_t(host.count), nil, 0,
        NI_NUMERICHOST) == 0
      {
        let name = String(cString: entry.pointee.ifa_name)
        if name.hasPrefix("en") { candidates.append((name, String(cString: host))) }
      }
    }
    return candidates.sorted { $0.0 < $1.0 }.first?.1
  }
}
