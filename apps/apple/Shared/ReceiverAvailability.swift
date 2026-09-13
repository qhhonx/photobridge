import Foundation
import Network

/// Preparation requires an allowed network and a response from the pinned peer.
/// Wi-Fi alone does not establish that the receiver is on the current network.
@MainActor final class ReceiverAvailability {
  private let monitor: NWPathMonitor?
  private let changed: () -> Void
  private let probe: (Pairing) async -> Bool
  private var allowed = false
  private var generation = 0
  private var route = ""
  private var result = false
  private var expires = Date.distantPast
  private var pending: Task<Bool, Never>?

  init(monitorNetwork: Bool = true, changed: @escaping () -> Void = {},
    probe: @escaping (Pairing) async -> Bool = { pairing in
      do {
        _ = try await Bridge.call(["op": "check_pairing",
          "pairing": JSONSerialization.jsonObject(with: JSONEncoder().encode(pairing))])
        return true
      } catch { return false }
    }) {
    self.changed = changed; self.probe = probe
    monitor = monitorNetwork ? NWPathMonitor() : nil
    monitor?.pathUpdateHandler = { [weak self] path in
      let permitted = path.status == .satisfied
        && !path.usesInterfaceType(.cellular)
        && (path.usesInterfaceType(.wifi) || path.usesInterfaceType(.wiredEthernet))
      Task { @MainActor in self?.networkChanged(allowed: permitted) }
    }
    monitor?.start(queue: DispatchQueue(label: "app.photobridge.network"))
  }
  deinit { monitor?.cancel() }

  func networkChanged(allowed: Bool) {
    self.allowed = allowed
    generation += 1
    result = false; expires = .distantPast
    pending?.cancel(); pending = nil
    changed()
  }

  func check(_ pairing: Pairing) async -> Bool {
    guard allowed else { return false }
    let key = pairing.receiverID + pairing.endpoint + pairing.certificate + pairing.token
    if key != route {
      route = key; generation += 1; expires = .distantPast
      pending?.cancel(); pending = nil
    }
    if Date() < expires { return result }
    let version = generation
    let task: Task<Bool, Never>
    if let pending { task = pending } else {
      task = Task { await probe(pairing) }
      pending = task
    }
    let reachable = await task.value
    guard version == generation, allowed, route == key else { return false }
    pending = nil; result = reachable
    expires = Date().addingTimeInterval(reachable ? 3 : 10)
    return reachable
  }
}
