import Combine
import Foundation
import Photos

#if os(macOS)
  import AppKit
  typealias LibraryImage = NSImage
#else
  import UIKit
  typealias LibraryImage = UIImage
#endif

// Use identical membership for browsing, export, thumbnails and scroll anchors.
// PhotoKit otherwise returns only representative burst frames by default.
func photoLibraryFetchOptions() -> PHFetchOptions {
  let options = PHFetchOptions()
  options.includeAllBurstAssets = true
  return options
}

enum LibraryFilter: String, CaseIterable, Identifiable {
  case all, photo, motion, video, burst
  var id: String { rawValue }
  var title: String { "library_filter_" + rawValue }
  var symbol: String {
    switch self {
    case .all: "square.grid.2x2"
    case .photo: "photo"
    case .motion: "livephoto"
    case .video: "video"
    case .burst: "square.stack"
    }
  }
}

@MainActor final class PhotoLibraryModel: NSObject, ObservableObject, PHPhotoLibraryChangeObserver {
  @Published private(set) var total = 0
  @Published private(set) var loaded = 0
  @Published private(set) var generation = 0
  @Published private(set) var contentGeneration = 0
  @Published private(set) var loading = false
  @Published private(set) var authorized = false
  @Published private(set) var authorizationStatus: PHAuthorizationStatus = .notDetermined
  @Published private(set) var requestingPermission = false
  @Published var filter: LibraryFilter = .all
  @Published var selection: Set<String> = []
  @Published var message: String?
  private(set) var fetch: PHFetchResult<PHAsset>?
  private var presentation = LibraryPresentationIndex()
  private var loadingPage = false
  private var cancelMetadata: (() -> Void)?
  private var groups: [String: LibraryGroup] = [:]
  private var groupOrder: [String] = []
  private var requestGeneration = 0
  private var observerRegistered = false
  private var changesTask: Task<Void, Never>?
  var scrollOffset = CGPoint.zero
  var scrollAnchorID: String?
  var scrollAnchorInset: CGFloat = 0
  private(set) var scrollResetGeneration = 0
  let thumbnails = ThumbnailPipeline()

  var placeholder: LibraryPlaceholderKind? {
    if !authorized { return .permission(authorizationStatus) }
    if fetch == nil { return .loading }
    if total == 0 { return .empty(filtered: filter != .all) }
    return nil
  }

  func open(requestPermission: Bool = false) async {
    guard !requestingPermission else { return }
    if requestPermission { requestingPermission = true }
    defer { if requestPermission { requestingPermission = false } }
    let access =
      requestPermission
      ? await PHPhotoLibrary.requestAuthorization(for: .readWrite)
      : PHPhotoLibrary.authorizationStatus(for: .readWrite)
    guard applyAuthorization(access) else { return }
    if !observerRegistered {
      PHPhotoLibrary.shared().register(self)
      observerRegistered = true
    }
    if fetch == nil { await reload() }
  }
  @discardableResult func applyAuthorization(_ access: PHAuthorizationStatus) -> Bool {
    authorizationStatus = access
    let allowed = access == .authorized || access == .limited
    if !allowed {
      // Invalidate suspended metadata fetches before clearing visible state.
      requestGeneration += 1
      cancelMetadata?(); cancelMetadata = nil
      fetch = nil
      presentation = LibraryPresentationIndex()
      groups.removeAll(); groupOrder.removeAll()
      loadingPage = false
      total = 0
      loaded = 0
      loading = false
      selection.removeAll()
      scrollOffset = .zero
      scrollAnchorID = nil
      scrollAnchorInset = 0
      scrollResetGeneration += 1
      thumbnails.clear()
      generation += 1
    }
    authorized = allowed
    return allowed
  }
  func changeFilter(_ next: LibraryFilter) async {
    guard filter != next else { return }
    filter = next
    selection.removeAll()
    scrollOffset = .zero
    scrollAnchorID = nil
    scrollAnchorInset = 0
    scrollResetGeneration += 1
    await reload(resetWindow: true)
  }
  func reload(resetWindow: Bool = false) async {
    guard authorized else { return }
    requestGeneration += 1
    cancelMetadata?(); cancelMetadata = nil
    let expected = requestGeneration
    loading = true
    let selectedFilter = filter
    let targetRows = resetWindow ? 240 : max(240, loaded)
    let previousCursor = resetWindow ? 0 : presentation.cursor
    let anchor = resetWindow ? nil : scrollAnchorID
    let selected = selection
    let work = Task.detached(priority: .userInitiated) {
      let options = photoLibraryFetchOptions()
      options.sortDescriptors = [NSSortDescriptor(key: "creationDate", ascending: false)]
      switch selectedFilter {
      case .all:
        options.predicate = NSPredicate(
          format: "mediaType == %d OR mediaType == %d", PHAssetMediaType.image.rawValue,
          PHAssetMediaType.video.rawValue)
      case .photo:
        options.predicate = NSPredicate(
          format: "mediaType == %d AND (mediaSubtype & %d) == 0", PHAssetMediaType.image.rawValue,
          PHAssetMediaSubtype.photoLive.rawValue)
      case .motion:
        options.predicate = NSPredicate(
          format: "(mediaSubtype & %d) != 0", PHAssetMediaSubtype.photoLive.rawValue)
      case .burst:
        options.predicate = NSPredicate(format: "burstIdentifier != nil")
      case .video:
        options.predicate = NSPredicate(format: "mediaType == %d", PHAssetMediaType.video.rawValue)
      }
      let fetch = PHAsset.fetchAssets(with: options)
      let anchorIndex = Self.rawIndex(of: anchor, in: fetch).map { $0 + 1 } ?? 0
      var index = LibraryPresentationIndex()
      index.load(fetch, targetRows: targetRows, minimumCursor: max(previousCursor, anchorIndex))
      return (fetch, index, Set(Self.resolveTokens(selected, in: fetch).keys))
    }
    cancelMetadata = { work.cancel() }
    let result = await work.value
    guard expected == requestGeneration, authorized else { return }
    cancelMetadata = nil
    fetch = result.0
    presentation = result.1
    total = result.0.count
    loaded = presentation.rows.count
    groups.removeAll(); groupOrder.removeAll()
    // Preserve selected off-screen items while removing deleted/revoked sources.
    selection.formIntersection(result.2.union(selection.subtracting(selected)))
    loadingPage = false
    loading = false
    generation += 1
  }
  func loadMore() {
    guard let fetch, !loading, !loadingPage, presentation.cursor < fetch.count else { return }
    loadingPage = true
    let expected = requestGeneration
    let current = presentation
    Task {
      guard expected == requestGeneration, authorized else { return }
      let work = Task.detached(priority: .userInitiated) {
        var next = current
        let changedCover = next.load(fetch, targetRows: current.rows.count + 240)
        return (next, changedCover)
      }
      cancelMetadata = { work.cancel() }
      let result = await work.value
      guard expected == requestGeneration, authorized else { return }
      cancelMetadata = nil
      presentation = result.0
      loaded = presentation.rows.count
      loadingPage = false
      if result.1 { generation += 1 }
    }
  }
  func asset(at index: Int) -> PHAsset? {
    guard let fetch, presentation.rows.indices.contains(index) else { return nil }
    return fetch.object(at: presentation.rows[index].rawIndex)
  }
  func identifier(at index: Int) -> String? {
    guard presentation.rows.indices.contains(index) else { return nil }
    return presentation.rows[index].key
  }
  func index(of identifier: String?) -> Int? {
    guard let identifier else { return nil }
    return presentation.positions[identifier]
  }
  func selectedIndexPaths() -> Set<IndexPath> {
    Set(selection.compactMap { presentation.positions[$0] }.map { IndexPath(item: $0, section: 0) })
  }
  /// The export scheduler expands the accessible burst siblings. Only actual
  /// PhotoKit asset identifiers cross that boundary, never presentation tokens.
  func selectedAssetIdentifiers(_ tokens: Set<String>) async -> [String] {
    guard let fetch else { return [] }
    return await Task.detached(priority: .userInitiated) {
      Array(Self.resolveTokens(tokens, in: fetch).values)
    }.value
  }
  private nonisolated static func resolveTokens(_ tokens: Set<String>, in fetch: PHFetchResult<PHAsset>) -> [String: String] {
    var resolved: [String: String] = [:]
    let ordinary = tokens.filter { !$0.hasPrefix("burst:") }
    if !ordinary.isEmpty {
      let assets = PHAsset.fetchAssets(withLocalIdentifiers: Array(ordinary), options: photoLibraryFetchOptions())
      assets.enumerateObjects { asset, _, _ in
        if fetch.index(of: asset) != NSNotFound { resolved[asset.localIdentifier] = asset.localIdentifier }
      }
    }
    for token in tokens where token.hasPrefix("burst:") {
      if let index = rawIndex(of: token, in: fetch) { resolved[token] = fetch.object(at: index).localIdentifier }
    }
    return resolved
  }
  private nonisolated static func rawIndex(of token: String?, in fetch: PHFetchResult<PHAsset>) -> Int? {
    guard let token else { return nil }
    let assets: PHFetchResult<PHAsset>
    if token.hasPrefix("burst:") {
      assets = PHAsset.fetchAssets(withBurstIdentifier: String(token.dropFirst(6)), options: photoLibraryFetchOptions())
    } else {
      assets = PHAsset.fetchAssets(withLocalIdentifiers: [token], options: photoLibraryFetchOptions())
    }
    var position: Int?
    assets.enumerateObjects { asset, _, _ in
      let index = fetch.index(of: asset)
      if index != NSNotFound { position = min(position ?? index, index) }
    }
    return position
  }
  /// Resolve only visible groups. The bounded cache contains metadata, not images.
  func visibleGroups(_ assets: [PHAsset]) async -> [String: LibraryGroup] {
    let expected = requestGeneration
    let missing = assets.filter { groups[$0.localIdentifier] == nil }
    let resolved = await Task.detached(priority: .userInitiated) {
      var out: [String: LibraryGroup] = [:]
      for asset in missing {
        var members = [asset]
        if let burst = asset.burstIdentifier {
          let result = PHAsset.fetchAssets(withBurstIdentifier: burst, options: photoLibraryFetchOptions())
          members = result.objects(at: IndexSet(integersIn: 0..<result.count))
        }
        out[asset.localIdentifier] = LibraryGroup(sources: members.map {
          LibrarySource(id: $0.localIdentifier, revision: Self.revision($0))
        }, complete: !members.isEmpty)
      }
      return out
    }.value
    guard !Task.isCancelled, expected == requestGeneration, authorized else { return [:] }
    for (id, group) in resolved where group.complete {
      groups[id] = group
      groupOrder.removeAll { $0 == id }; groupOrder.append(id)
    }
    let visible = Set(assets.map(\.localIdentifier))
    let result = groups.merging(resolved, uniquingKeysWith: { _, new in new }).filter { visible.contains($0.key) }
    while groups.count > 128 || groups.values.reduce(0, { $0 + $1.sources.count }) > 4096 {
      guard !groupOrder.isEmpty else { break }
      groups.removeValue(forKey: groupOrder.removeFirst())
    }
    return result
  }
  nonisolated static func revision(_ asset: PHAsset) -> String {
    String(
      Int64(
        (asset.modificationDate ?? asset.creationDate ?? Date(timeIntervalSince1970: 0))
          .timeIntervalSince1970 * 1000))
  }
  nonisolated func photoLibraryDidChange(_ changeInstance: PHChange) {
    Task { @MainActor in
      let access = PHPhotoLibrary.authorizationStatus(for: .readWrite)
      guard self.applyAuthorization(access), let fetch = self.fetch,
        let details = changeInstance.changeDetails(for: fetch) else { return }
      let structural = !details.hasIncrementalChanges || details.hasMoves
        || !(details.insertedIndexes?.isEmpty ?? true) || !(details.removedIndexes?.isEmpty ?? true)
      if !structural && !self.loading {
        // Availability and metadata callbacks must not recycle the whole grid.
        self.fetch = details.fetchResultAfterChanges
        if !(details.changedIndexes?.isEmpty ?? true) {
          self.groups.removeAll(); self.groupOrder.removeAll()
          self.contentGeneration += 1
        }
        return
      }
      self.changesTask?.cancel()
      self.changesTask = Task {
        try? await Task.sleep(nanoseconds: 300_000_000)
        guard !Task.isCancelled else { return }
        await self.reload()
      }
    }
  }
  deinit { if observerRegistered { PHPhotoLibrary.shared().unregisterChangeObserver(self) } }
}

/// Local preheating plus at most three visible iCloud image requests. PhotoKit
/// controls the network fetch size; only requested display images enter our cache.
@MainActor final class ThumbnailPipeline {
  private let manager = PHCachingImageManager()
  private let cache = NSCache<NSString, LibraryImage>()
  private var preheated: [String: PHAsset] = [:]
  private var preheatSize = CGSize.zero
  private struct Pending {
    let asset: PHAsset
    let target: CGSize
    let key: NSString
    let completion: (LibraryImage?, Bool) -> Void
    var native: PHImageRequestID = PHInvalidImageRequestID
    var cloud = false
    var hasImage = false
  }
  private var pending: [PHImageRequestID: Pending] = [:]
  private var cloudQueue: [PHImageRequestID] = []
  private var activeCloud: Set<PHImageRequestID> = []
  private var sequence: PHImageRequestID = 0
  private(set) var requestsStarted = 0
  private(set) var requestsCancelled = 0
  init() {
    cache.totalCostLimit = 64 * 1024 * 1024
    cache.countLimit = 500
    manager.allowsCachingHighQualityImages = false
  }
  func size(_ requested: CGFloat) -> CGSize {
    let edge = min(640, max(128, (requested / 64).rounded(.up) * 64))
    return CGSize(width: edge, height: edge)
  }
  private func options(network: Bool = false) -> PHImageRequestOptions {
    let options = PHImageRequestOptions()
    options.isNetworkAccessAllowed = network
    options.deliveryMode = .opportunistic
    options.resizeMode = .fast
    return options
  }
  @discardableResult func request(
    _ asset: PHAsset, pixels: CGFloat, completion: @escaping (LibraryImage?, Bool) -> Void
  ) -> PHImageRequestID {
    let target = size(pixels)
    let key = "\(asset.localIdentifier)|\(PhotoLibraryModel.revision(asset))|\(Int(target.width))" as NSString
    if let image = cache.object(forKey: key) {
      completion(image, false)
      return PHInvalidImageRequestID
    }
    sequence = sequence == Int32.max ? 1 : sequence + 1
    let id = sequence
    requestsStarted += 1
    pending[id] = Pending(asset: asset, target: target, key: key, completion: completion)
    start(id, network: false)
    return id
  }
  private func start(_ id: PHImageRequestID, network: Bool) {
    guard let item = pending[id] else { return }
    pending[id]?.cloud = network
    let request = manager.requestImage(for: item.asset, targetSize: item.target,
      contentMode: .aspectFill, options: options(network: network)) { [weak self] image, info in
      let cancelled = info?[PHImageCancelledKey] as? Bool == true
      let degraded = info?[PHImageResultIsDegradedKey] as? Bool == true
      let cloud = info?[PHImageResultIsInCloudKey] as? Bool == true
      Task { @MainActor in
        guard let self, let item = self.pending[id], item.cloud == network, !cancelled else { return }
        if let image {
          self.pending[id]?.hasImage = true
          if !degraded { self.cache.setObject(image, forKey: item.key, cost: Int(item.target.width * item.target.height * 4)) }
          item.completion(image, false)
        }
        guard !degraded else { return }
        if image == nil && cloud && !network && !item.hasImage {
          item.completion(nil, true)
          self.cloudQueue.append(id)
          self.pumpCloud()
        } else {
          if image == nil && !item.hasImage { item.completion(nil, cloud) }
          self.pending.removeValue(forKey: id)
          self.activeCloud.remove(id)
          self.pumpCloud()
        }
      }
    }
    pending[id]?.native = request
  }
  private func pumpCloud() {
    while activeCloud.count < 3 && !cloudQueue.isEmpty {
      let id = cloudQueue.removeFirst()
      guard pending[id] != nil else { continue }
      activeCloud.insert(id)
      start(id, network: true)
    }
  }
  func cancel(_ request: PHImageRequestID) {
    guard let item = pending.removeValue(forKey: request) else { return }
    requestsCancelled += 1
    manager.cancelImageRequest(item.native)
    cloudQueue.removeAll { $0 == request }
    activeCloud.remove(request)
    pumpCloud()
  }
  func preheat(_ assets: [PHAsset], pixels: CGFloat) {
    let target = size(pixels)
    if preheatSize != target {
      manager.stopCachingImagesForAllAssets()
      preheated.removeAll()
      preheatSize = target
    }
    let next = Dictionary(assets.prefix(180).map { ($0.localIdentifier, $0) }, uniquingKeysWith: { a, _ in a })
    let removed = preheated.filter { next[$0.key] == nil }.map(\.value)
    let added = next.filter { preheated[$0.key] == nil }.map(\.value)
    manager.stopCachingImages(for: removed, targetSize: target, contentMode: .aspectFill, options: options())
    manager.startCachingImages(for: added, targetSize: target, contentMode: .aspectFill, options: options())
    preheated = next
  }
  func clear() {
    let requests = pending.values.map(\.native)
    pending.removeAll(); cloudQueue.removeAll(); activeCloud.removeAll()
    requests.forEach { manager.cancelImageRequest($0) }
    cache.removeAllObjects()
    manager.stopCachingImagesForAllAssets()
    preheated.removeAll()
  }
}

func taskSymbol(_ state: String?) -> String {
  switch state {
  case "received": "checkmark.circle.fill"
  case "received_previous": "checkmark.circle"
  case "partial": "circle.lefthalf.filled"
  case "scheduled", "running": "arrow.up.circle"
  case "waiting": "clock.arrow.circlepath"
  case "failed": "exclamationmark.circle"
  case "paused": "pause.circle"
  case "scanned": "photo.stack"
  case "preparing": "arrow.down.circle"
  case "queued": "clock"
  default: "circle.dashed"
  }
}
