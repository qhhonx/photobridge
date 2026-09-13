import Foundation
import Photos

/// Incremental metadata index. Presentation groups never change transfer identities.
struct LibraryPresentationIndex: Sendable {
  struct Metadata {
    let id: String
    let burst: String?
    let representative: Bool
    var key: String { burst.map { "burst:" + $0 } ?? id }
  }
  struct Row: Sendable {
    let key: String
    var rawIndex: Int
  }
  private(set) var rows: [Row] = []
  private(set) var positions: [String: Int] = [:]
  private(set) var cursor = 0

  /// Fill enough display rows, preserving a burst's first position even when its
  /// members cross a page boundary or have noncontiguous edited capture dates.
  @discardableResult mutating func load(sourceCount: Int, targetRows: Int,
    minimumCursor: Int = 0, shouldContinue: () -> Bool = { !Task.isCancelled },
    read: (Int) -> Metadata) -> Bool {
    var changedCover = false
    while cursor < sourceCount && (rows.count < targetRows || cursor < minimumCursor) && shouldContinue() {
      let item = read(cursor)
      if let position = positions[item.key] {
        if item.representative && rows[position].rawIndex != cursor {
          rows[position].rawIndex = cursor
          changedCover = true
        }
      } else {
        positions[item.key] = rows.count
        rows.append(Row(key: item.key, rawIndex: cursor))
      }
      cursor += 1
    }
    return changedCover
  }
}

struct LibrarySource: Sendable {
  let id: String
  let revision: String
}
struct LibraryGroup: Sendable {
  let sources: [LibrarySource]
  let complete: Bool
  var count: Int? { complete ? sources.count : nil }

  func state(_ states: [String: String]) -> String? {
    guard complete, !sources.isEmpty else { return nil }
    let known = sources.compactMap { states[$0.id] }
    // An active or failed member takes precedence over a completed cover photo.
    for state in ["failed", "preparing", "running", "waiting", "paused", "queued", "scheduled"] {
      if known.contains(state) { return state }
    }
    if known.count == sources.count && known.allSatisfy({ $0 == "received" }) { return "received" }
    return known.contains("received") ? "partial" : nil
  }
}

extension LibraryPresentationIndex {
  @discardableResult mutating func load(_ fetch: PHFetchResult<PHAsset>, targetRows: Int,
    minimumCursor: Int = 0) -> Bool {
    load(sourceCount: fetch.count, targetRows: targetRows, minimumCursor: minimumCursor) { index in
      let asset = fetch.object(at: index)
      return Metadata(id: asset.localIdentifier, burst: asset.burstIdentifier,
        representative: asset.representsBurst)
    }
  }
}

@MainActor func librarySourceStates(_ groups: [String: LibraryGroup], receiver: String?) async -> [String: String]? {
  guard let receiver else { return [:] }
  var sources: [String: String] = [:]
  for group in groups.values {
    for source in group.sources { sources[source.id] = source.revision }
  }
  let entries = sources.map { [$0.key, $0.value] }
  var states: [String: String] = [:]
  for start in stride(from: 0, to: entries.count, by: 400) {
    guard !Task.isCancelled else { return nil }
    let batch = Array(entries[start..<min(start + 400, entries.count)])
    if let data = try? await Bridge.call(["op": "source_states", "include_pending": true, "receiver_id": receiver, "sources": batch]),
      let result = try? JSONDecoder().decode([String: String].self, from: data) {
      states.merge(result, uniquingKeysWith: { _, new in new })
    } else { return nil }
  }
  return states
}
