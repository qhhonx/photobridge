import Foundation
import Darwin

/// Bonjour supplies route candidates only. Rust verifies each against saved TLS
/// identity before the executor changes its route. No credentials are broadcast.
@MainActor final class ReceiverDiscovery: NSObject, @preconcurrency NetServiceBrowserDelegate,
  @preconcurrency NetServiceDelegate {
  private weak var model: BackupModel?
  private var browser: NetServiceBrowser?
  private var services: [NetService] = []
  private var receiverID: String?
  private var lastResolve = Date.distantPast
  private var probing = false
  private var nextProbe = Date.distantPast
  private var nextBrowse = Date.distantPast
  init(model: BackupModel) { self.model = model }

  func update(pairing: Pairing?, active: Bool) {
    guard active, let pairing else { stop(); return }
    if receiverID != pairing.receiverID { stop(); receiverID = pairing.receiverID }
    if browser == nil, Date() >= nextBrowse {
      let browser = NetServiceBrowser()
      self.browser = browser
      browser.delegate = self
      browser.searchForServices(ofType: "_photobridge._tcp.", inDomain: "local.")
    }
    if Date().timeIntervalSince(lastResolve) > 30 {
      lastResolve = Date()
      for service in services { service.stop(); service.resolve(withTimeout: 5) }
    }
  }
  private func stop() {
    browser?.stop(); browser?.delegate = nil; browser = nil
    for service in services { service.stop(); service.delegate = nil }
    services.removeAll(); receiverID = nil
  }
  func netServiceBrowser(_ browser: NetServiceBrowser, didFind service: NetService, moreComing: Bool) {
    guard let receiverID, services.count < 8,
      service.name.hasPrefix("PhotoBridge-" + receiverID.prefix(20)),
      !services.contains(where: { $0.name == service.name && $0.domain == service.domain }) else { return }
    services.append(service); service.delegate = self; service.resolve(withTimeout: 5)
  }
  func netServiceBrowser(_ browser: NetServiceBrowser, didRemove service: NetService, moreComing: Bool) {
    services.filter { $0.name == service.name && $0.domain == service.domain }.forEach { $0.stop(); $0.delegate = nil }
    services.removeAll { $0.name == service.name && $0.domain == service.domain }
  }
  func netServiceBrowser(_ browser: NetServiceBrowser, didNotSearch errorDict: [String: NSNumber]) {
    stop() // Foreground polling retries; never bypass local-network permission.
    nextBrowse = Date().addingTimeInterval(30)
  }
  func netServiceDidResolveAddress(_ service: NetService) {
    guard !probing, Date() >= nextProbe, let model, let saved = model.pairing,
      saved.receiverID == receiverID, let txt = service.txtRecordData(), txt.count <= 1024,
      NetService.dictionary(fromTXTRecord: txt)["id"] == Data(saved.receiverID.utf8),
      (1...65535).contains(service.port) else { return }
    let candidates = (service.addresses ?? []).prefix(8).compactMap { data -> String? in
      guard data.count >= MemoryLayout<sockaddr_in>.size else { return nil }
      return data.withUnsafeBytes { bytes -> String? in
        guard let address = bytes.bindMemory(to: sockaddr.self).baseAddress,
          address.pointee.sa_family == sa_family_t(AF_INET),
          Int(address.pointee.sa_len) <= data.count else { return nil }
        var host = [CChar](repeating: 0, count: Int(NI_MAXHOST))
        guard getnameinfo(address, socklen_t(data.count), &host, socklen_t(host.count), nil, 0, NI_NUMERICHOST) == 0 else { return nil }
        return "https://\(String(cString: host)):\(service.port)"
      }
    }.filter { $0 != saved.endpoint }
    guard !candidates.isEmpty else { return }
    probing = true; nextProbe = Date().addingTimeInterval(30)
    Task {
      defer { probing = false }
      for endpoint in candidates.prefix(2) {
        guard model.pairing?.receiverID == saved.receiverID, model.pairing?.endpoint == saved.endpoint else { break }
        await model.relocateReceiver(to: endpoint, expected: saved)
      }
    }
  }
}
