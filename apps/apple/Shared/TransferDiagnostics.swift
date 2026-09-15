import Foundation
import Network
#if os(iOS)
import UIKit
#endif

/// Samples only fixed flags and timing counters. Never collects SSIDs, URLs,
/// addresses, headers, photo names or certificate material.
@MainActor final class TransferDiagnostics {
  static let shared = TransferDiagnostics()
  private let monitor = NWPathMonitor()
  private var network: [String: Any] = [:]
  private init() {
    #if os(iOS)
    UIDevice.current.isBatteryMonitoringEnabled = true
    #endif
    monitor.pathUpdateHandler = { [weak self] path in
      let flags: [String: Any] = [
        "network_available": path.status == .satisfied,
        "network_wifi": path.usesInterfaceType(.wifi),
        "network_cellular": path.usesInterfaceType(.cellular),
        "network_expensive": path.isExpensive,
        "network_constrained": path.isConstrained,
      ]
      Task { @MainActor in self?.network = flags }
    }
    monitor.start(queue: DispatchQueue(label: "app.photobridge.diagnostics.network"))
  }
  static func milliseconds(_ date: Date) -> UInt64 {
    UInt64(max(0, (date.timeIntervalSince1970 * 1000).rounded()))
  }
  func snapshot() -> [String: Any] {
    var value = network
    value["observed_at_ms"] = Self.milliseconds(Date())
    value["low_power"] = ProcessInfo.processInfo.isLowPowerModeEnabled
    value["thermal_state"] = ProcessInfo.processInfo.thermalState.rawValue
    #if os(iOS)
    let device = UIDevice.current
    if device.batteryLevel >= 0 { value["battery_percent"] = Int((device.batteryLevel * 100).rounded()) }
    if device.batteryState != .unknown {
      value["charging"] = device.batteryState == .charging || device.batteryState == .full
    }
    #endif
    return value
  }
  static func metrics(_ metrics: URLSessionTaskMetrics) -> [String: Any] {
    var value: [String: Any] = ["metrics_available": true,
      "transaction_count": metrics.transactionMetrics.count,
      "metric_start_ms": milliseconds(metrics.taskInterval.start),
      "metric_end_ms": milliseconds(metrics.taskInterval.end)]
    // First attempted connection and last HTTP exchange distinguish retry/delay
    // from callback delivery. Absent dates remain absent, never zero durations.
    let transactions = metrics.transactionMetrics
    for (key, date) in [
      ("fetch_at_ms", transactions.compactMap(\.fetchStartDate).first),
      ("connect_at_ms", transactions.compactMap(\.connectStartDate).first),
      ("tls_at_ms", transactions.compactMap(\.secureConnectionStartDate).first),
      ("tls_end_at_ms", transactions.compactMap(\.secureConnectionEndDate).last),
      ("send_at_ms", transactions.compactMap(\.requestStartDate).first),
      ("sent_at_ms", transactions.compactMap(\.requestEndDate).last),
      ("response_at_ms", transactions.compactMap(\.responseStartDate).first),
      ("response_end_at_ms", transactions.compactMap(\.responseEndDate).last),
    ] { if let date { value[key] = milliseconds(date) } }
    if let last = transactions.last { value["connection_reused"] = last.isReusedConnection }
    return value
  }
}
