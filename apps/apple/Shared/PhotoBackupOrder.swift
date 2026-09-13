import Foundation
import Photos

/// Capture time, not modification time or opaque PhotoKit identifier order.
enum PhotoBackupOrder {
  static func timestamp(_ date: Date?) -> Int64 {
    date.map { Int64($0.timeIntervalSince1970 * 1000) } ?? Int64.min
  }

  static func precedes(_ lhsID: String, _ lhsDate: Int64, _ rhsID: String, _ rhsDate: Int64) -> Bool {
    lhsDate == rhsDate ? lhsID < rhsID : lhsDate > rhsDate
  }

  static func captureDates(_ identifiers: [String]) async -> [String: Int64] {
    await Task.detached(priority: .utility) {
      var dates: [String: Int64] = [:]
      for start in stride(from: 0, to: identifiers.count, by: 200) {
        let batch = Array(identifiers[start..<min(start + 200, identifiers.count)])
        let assets = PHAsset.fetchAssets(withLocalIdentifiers: batch, options: photoLibraryFetchOptions())
        assets.enumerateObjects { asset, _, _ in
          dates[asset.localIdentifier] = timestamp(asset.creationDate)
        }
      }
      return dates
    }.value
  }
}
