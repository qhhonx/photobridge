import Foundation
import Photos

extension BackupModel {
  /// Every preparation path expands accessible siblings without an existing task.
  /// Existing failed tasks stay visible for retry and are not recursively re-added.
  func scheduleBurstSiblings(of assets: PHFetchResult<PHAsset>, receiver: String) async throws {
    let groups = await Task.detached(priority: .utility) {
      Set((0..<assets.count).compactMap { assets.object(at: $0).burstIdentifier })
    }.value
    for group in groups {
      try Task.checkCancellation()
      guard pairing?.receiverID == receiver else { throw CancellationError() }
      let members = await Task.detached(priority: .utility) {
        PHAsset.fetchAssets(withBurstIdentifier: group, options: photoLibraryFetchOptions())
      }.value
      for start in stride(from: 0, to: members.count, by: 200) {
        let end = min(start + 200, members.count)
        let sources = await Task.detached(priority: .utility) {
          (start..<end).map { index -> [String] in
            let asset = members.object(at: index)
            return [asset.localIdentifier, String(Int64((asset.modificationDate ?? asset.creationDate ?? Date(timeIntervalSince1970: 0)).timeIntervalSince1970 * 1000))]
          }
        }.value
        try Task.checkCancellation()
        guard pairing?.receiverID == receiver else { throw CancellationError() }
        let data = try await Bridge.call(["op": "source_states", "receiver_id": receiver, "sources": sources])
        let known = try JSONDecoder().decode([String: String].self, from: data)
        let needed = sources.filter { known[$0[0]] == nil }.map { $0[0] }
        if !needed.isEmpty {
          _ = try await Bridge.call(["op": "schedule_sources", "receiver_id": receiver, "sources": needed])
        }
      }
    }
  }
  func burstFields(for asset: PHAsset) async throws -> [String: String] {
    guard let group = asset.burstIdentifier, !group.isEmpty else { return [:] }
    let data = try await Bridge.call(["op": "burst_metadata", "identifier": group, "primary": asset.representsBurst])
    return try JSONDecoder().decode([String: String].self, from: data)
  }
}
