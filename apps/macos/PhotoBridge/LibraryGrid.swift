import AppKit
import Photos
import SwiftUI

struct MacLibraryGrid: NSViewRepresentable {
  @ObservedObject var library: PhotoLibraryModel
  var preparationRevision: String = ""
  var queueRevision: Int64
  var receiverID: String?
  var itemSize: CGFloat
  func makeCoordinator() -> Coordinator { Coordinator(self) }
  func makeNSView(context: Context) -> NSScrollView {
    let layout = NSCollectionViewFlowLayout()
    layout.minimumInteritemSpacing = 4
    layout.minimumLineSpacing = 4
    layout.sectionInset = NSEdgeInsets(top: 12, left: 16, bottom: 20, right: 16)
    let collection = NSCollectionView()
    collection.collectionViewLayout = layout
    collection.autoresizingMask = [.width]
    collection.backgroundColors = [.clear]
    collection.isSelectable = true
    collection.allowsMultipleSelection = true
    collection.register(PhotoCell.self, forItemWithIdentifier: .init("photo"))
    collection.dataSource = context.coordinator
    collection.delegate = context.coordinator
    let scroll = LibraryScrollView()
    scroll.hasVerticalScroller = true
    scroll.drawsBackground = false
    scroll.documentView = collection
    scroll.contentView.postsBoundsChangedNotifications = true
    context.coordinator.collection = collection
    context.coordinator.observer = NotificationCenter.default.addObserver(
      forName: NSView.boundsDidChangeNotification, object: scroll.contentView, queue: .main
    ) { [weak coordinator = context.coordinator] _ in
      Task { @MainActor in coordinator?.scrolled() }
    }
    return scroll
  }
  func updateNSView(_ scroll: NSScrollView, context: Context) {
    let c = context.coordinator
    c.parent = self
    guard let collection = c.collection,
      let layout = collection.collectionViewLayout as? NSCollectionViewFlowLayout
    else { return }
    if layout.itemSize.width != itemSize {
      layout.itemSize = NSSize(width: itemSize, height: itemSize)
      layout.invalidateLayout()
      c.refreshVisibleImages()
    }
    if c.generation != library.generation {
      let sameFilter = (c.filter == nil || c.filter == library.filter)
        && (c.scrollResetGeneration == nil || c.scrollResetGeneration == library.scrollResetGeneration)
      if sameFilter, c.generation >= 0 { c.rememberScroll() }
      let offset = sameFilter ? library.scrollOffset : .zero
      let anchor = sameFilter ? library.scrollAnchorID : nil
      let inset = library.scrollAnchorInset
      c.restoring = true
      c.filter = library.filter
      c.scrollResetGeneration = library.scrollResetGeneration
      c.generation = library.generation
      c.count = library.loaded
      c.applyingSelection = true
      c.appliedSelection = nil
      collection.reloadData()
      (scroll as? LibraryScrollView)?.restoreScroll = { [weak c] scroll in
        guard let c, let collection = c.collection,
          let layout = collection.collectionViewLayout else { return }
        collection.layoutSubtreeIfNeeded()
        var target = offset
        if let index = library.index(of: anchor), index < library.loaded {
          if let frame = layout.layoutAttributesForItem(at: IndexPath(item: index, section: 0))?.frame {
            target.y = frame.minY + inset
          }
        }
        let maximum = max(0, collection.frame.height - scroll.contentView.bounds.height)
        target.y = min(maximum, max(0, target.y))
        scroll.contentView.scroll(to: target)
        scroll.reflectScrolledClipView(scroll.contentView)
        c.restoring = false
        c.applyingSelection = false
        c.scrolled()
      }
      scroll.needsLayout = true
      scroll.layoutSubtreeIfNeeded()
    } else if c.count < library.loaded {
      let old = c.count
      c.count = library.loaded
      c.appliedSelection = nil
      collection.insertItems(at: Set((old..<c.count).map { IndexPath(item: $0, section: 0) }))
    }
    if c.contentGeneration != library.contentGeneration {
      c.contentGeneration = library.contentGeneration
      c.refreshVisibleImages()
      c.scrolled()
    }
    if c.preparationRevision != preparationRevision {
      c.preparationRevision = preparationRevision
      c.scrolled()
    }
    if c.statusRevision != queueRevision || c.receiverID != receiverID {
      c.statusRevision = queueRevision
      c.receiverID = receiverID
      c.scrolled()
    }
    if c.appliedSelection != library.selection {
      c.applyingSelection = true
      collection.selectionIndexPaths = library.selectedIndexPaths()
      c.appliedSelection = library.selection
      c.applyingSelection = false
    }
  }
  static func dismantleNSView(_ nsView: NSScrollView, coordinator: Coordinator) {
    coordinator.rememberScroll()
    coordinator.refresh.cancel()
    if let observer = coordinator.observer { NotificationCenter.default.removeObserver(observer) }
    coordinator.parent.library.thumbnails.clear()
  }
  @MainActor final class Coordinator: NSObject, NSCollectionViewDataSource, NSCollectionViewDelegate
  {
    var parent: MacLibraryGrid
    weak var collection: NSCollectionView?
    var observer: NSObjectProtocol?
    let refresh = LibraryRefreshScheduler()
    var restoring = false
    var applyingSelection = false
    var appliedSelection: Set<String>?
    var count = 0
    var generation = -1
    var filter: LibraryFilter?
    var scrollResetGeneration: Int?
    var preparationRevision = ""
    var contentGeneration = -1
    var statusRevision: Int64 = -2
    var receiverID: String?
    var visibleIdentifiers: [IndexPath: String] = [:]
    init(_ parent: MacLibraryGrid) { self.parent = parent }
    func collectionView(_ collectionView: NSCollectionView, numberOfItemsInSection section: Int)
      -> Int
    { count }
    func collectionView(
      _ collectionView: NSCollectionView, itemForRepresentedObjectAt indexPath: IndexPath
    ) -> NSCollectionViewItem {
      let item =
        collectionView.makeItem(withIdentifier: .init("photo"), for: indexPath) as! PhotoCell
      if let asset = parent.library.asset(at: indexPath.item) {
        item.configure(
          asset, pipeline: parent.library.thumbnails,
          pixels: parent.itemSize * (collectionView.window?.backingScaleFactor ?? 2),
          identifier: parent.library.identifier(at: indexPath.item))
        visibleIdentifiers[indexPath] = asset.localIdentifier
      }
      return item
    }
    func collectionView(
      _ collectionView: NSCollectionView, willDisplay item: NSCollectionViewItem,
      forRepresentedObjectAt indexPath: IndexPath
    ) {
      if let cell = item as? PhotoCell, let asset = parent.library.asset(at: indexPath.item) {
        cell.configure(
          asset, pipeline: parent.library.thumbnails,
          pixels: parent.itemSize * (collectionView.window?.backingScaleFactor ?? 2),
          identifier: parent.library.identifier(at: indexPath.item))
        visibleIdentifiers[indexPath] = asset.localIdentifier
      }
      scrolled()
    }
    func collectionView(
      _ collectionView: NSCollectionView, didEndDisplaying item: NSCollectionViewItem,
      forRepresentedObjectAt indexPath: IndexPath
    ) {
      (item as? PhotoCell)?.cancel()
      visibleIdentifiers.removeValue(forKey: indexPath)
    }
    func collectionView(
      _ collectionView: NSCollectionView, didSelectItemsAt indexPaths: Set<IndexPath>
    ) { changeSelection(indexPaths, selected: true) }
    func collectionView(
      _ collectionView: NSCollectionView, didDeselectItemsAt indexPaths: Set<IndexPath>
    ) { changeSelection(indexPaths, selected: false) }
    private func changeSelection(_ paths: Set<IndexPath>, selected: Bool) {
      guard !applyingSelection else { return }
      let ids = paths.compactMap { parent.library.identifier(at: $0.item) }
      if selected { parent.library.selection.formUnion(ids) }
      else { parent.library.selection.subtract(ids) }
      appliedSelection = parent.library.selection
    }
    func refreshVisibleImages() {
      guard let collection else { return }
      for path in collection.indexPathsForVisibleItems() {
        if let item = collection.item(at: path) as? PhotoCell,
          let asset = parent.library.asset(at: path.item)
        {
          item.configure(
            asset, pipeline: parent.library.thumbnails,
            pixels: parent.itemSize * (collection.window?.backingScaleFactor ?? 2),
            identifier: parent.library.identifier(at: path.item))
        }
      }
    }
    func scrolled() {
      guard !restoring else { return }
      if generation == parent.library.generation { rememberScroll() }
      refresh.schedule { [weak self] in
        guard let self, !Task.isCancelled, let collection = self.collection else { return }
        let parent = self.parent
        self.rememberScroll()
        let generation = parent.library.generation
        let receiverID = parent.receiverID
        let revision = parent.queueRevision
        let preparation = parent.preparationRevision
        let paths = collection.indexPathsForVisibleItems().sorted()
        guard let first = paths.first?.item, let last = paths.last?.item else { return }
        let identities = Dictionary(uniqueKeysWithValues: paths.compactMap { path in
          parent.library.asset(at: path.item).map { (path, $0.localIdentifier) }
        })
        if last >= parent.library.loaded - 48 { parent.library.loadMore() }
        let range = max(0, first - 24)..<min(parent.library.loaded, last + 49)
        let assets = range.compactMap { parent.library.asset(at: $0) }
        parent.library.thumbnails.preheat(
          assets, pixels: parent.itemSize * (collection.window?.backingScaleFactor ?? 2))
        let visible = paths.compactMap { parent.library.asset(at: $0.item) }
        let groups = await parent.library.visibleGroups(visible)
        guard !Task.isCancelled, generation == self.parent.library.generation else { return }
        guard let states = await librarySourceStates(groups, receiver: receiverID) else { return }
        guard !Task.isCancelled, generation == self.parent.library.generation,
          receiverID == self.parent.receiverID, revision == self.parent.queueRevision,
          preparation == self.parent.preparationRevision else { return }
        for path in paths {
          if let item = collection.item(at: path) as? PhotoCell {
            guard item.assetID == identities[path] else { continue }
            let group = groups[item.assetID ?? ""]
            let burst = parent.library.asset(at: path.item)?.burstIdentifier != nil
            item.setGroup(count: burst ? group?.count : nil, state: group?.state(states))
          }
        }
      }
    }
    func rememberScroll() {
      guard !restoring, parent.library.authorized, filter == parent.library.filter,
        scrollResetGeneration == parent.library.scrollResetGeneration, let collection,
        let scroll = collection.enclosingScrollView,
        let top = collection.indexPathsForVisibleItems().sorted().first,
        let id = (collection.item(at: top) as? PhotoCell)?.presentationID,
        let frame = collection.collectionViewLayout?.layoutAttributesForItem(at: top)?.frame,
        frame.intersects(collection.visibleRect)
      else { return }
      parent.library.scrollOffset = scroll.contentView.bounds.origin
      parent.library.scrollAnchorID = id
      parent.library.scrollAnchorInset = scroll.contentView.bounds.origin.y - frame.minY
    }
  }
}

@MainActor final class PhotoCell: NSCollectionViewItem {
  private var pipeline: ThumbnailPipeline?
  private var request: PHImageRequestID = PHInvalidImageRequestID
  private var token = UUID()
  private var imageKey = ""
  private(set) var assetID: String?
  private(set) var presentationID: String?
  private let photoLayer = CALayer()
  private let media = NSImageView()
  private let status = NSImageView()
  private let cloud = NSImageView()
  private let burstCount = NSTextField(labelWithString: "")
  private var groupDescription = ""
  override func loadView() {
    view = NSView()
    view.wantsLayer = true
    view.layer?.backgroundColor = NSColor.quaternaryLabelColor.cgColor
    view.layer?.masksToBounds = true
    view.layer?.cornerRadius = 3
    photoLayer.contentsGravity = .resizeAspectFill
    photoLayer.masksToBounds = true
    view.layer?.addSublayer(photoLayer)
    for badge in [media, status, cloud] {
      badge.wantsLayer = true
      badge.contentTintColor = .white
      badge.layer?.backgroundColor = NSColor.black.withAlphaComponent(0.38).cgColor
      badge.layer?.cornerRadius = 12
      badge.imageScaling = .scaleProportionallyUpOrDown
      view.addSubview(badge)
    }
    burstCount.textColor = .white
    burstCount.backgroundColor = .black.withAlphaComponent(0.45)
    burstCount.drawsBackground = true
    burstCount.font = .monospacedDigitSystemFont(ofSize: 12, weight: .medium)
    burstCount.alignment = .center
    burstCount.wantsLayer = true
    burstCount.layer?.cornerRadius = 4
    burstCount.isHidden = true
    view.addSubview(burstCount)
    cloud.isHidden = true
    media.isHidden = true
    view.setAccessibilityElement(true)
    view.setAccessibilityRole(.image)
  }
  override func viewDidLayout() {
    super.viewDidLayout()
    CATransaction.begin()
    CATransaction.setDisableActions(true)
    photoLayer.frame = view.bounds
    CATransaction.commit()
    media.frame = NSRect(x: 8, y: view.bounds.height - 32, width: 24, height: 24)
    burstCount.frame = NSRect(x: 36, y: view.bounds.height - 30, width: 42, height: 20)
    cloud.frame = NSRect(x: 8, y: 8, width: 24, height: 24)
    status.frame = NSRect(x: view.bounds.width - 32, y: 8, width: 24, height: 24)
  }
  override var isSelected: Bool {
    didSet {
      view.layer?.borderWidth = isSelected ? 3 : 0
      view.layer?.borderColor = NSColor.controlAccentColor.cgColor
    }
  }
  func configure(_ asset: PHAsset, pipeline: ThumbnailPipeline, pixels: CGFloat, identifier: String? = nil) {
    presentationID = identifier ?? asset.localIdentifier
    let key =
      "\(asset.localIdentifier)|\(PhotoLibraryModel.revision(asset))|\(pipeline.size(pixels).width)"
    guard imageKey != key else { return }
    cancel()
    self.pipeline = pipeline
    assetID = asset.localIdentifier
    imageKey = key
    let current = token
    photoLayer.contents = nil
    cloud.isHidden = true
    let symbol =
      asset.burstIdentifier != nil ? "square.stack" : asset.mediaSubtypes.contains(.photoLive)
      ? "livephoto" : asset.mediaType == .video ? "video.fill" : nil
    media.image = symbol.flatMap { NSImage(systemSymbolName: $0, accessibilityDescription: nil) }
    media.isHidden = symbol == nil
    view.setAccessibilityLabel(
      (asset.creationDate?.formatted(date: .abbreviated, time: .shortened) ?? "") + " "
        + NSLocalizedString(
          symbol == "square.stack" ? "library_filter_burst" : symbol == "livephoto"
            ? "library_filter_motion"
            : symbol == nil ? "library_filter_photo" : "library_filter_video", comment: ""))
    setGroup(count: nil, state: nil)
    request = pipeline.request(asset, pixels: pixels) { [weak self] image, inCloud in
      guard let self, self.token == current else { return }
      if let image {
        self.photoLayer.contents = image.cgImage(forProposedRect: nil, context: nil, hints: nil)
      }
      self.cloud.image = NSImage(systemSymbolName: "icloud", accessibilityDescription: nil)
      self.cloud.isHidden = image != nil || !inCloud
    }
  }
  func setGroup(count: Int?, state: String?) {
    if count != nil {
      media.image = NSImage(systemSymbolName: "square.stack", accessibilityDescription: nil)
      media.isHidden = false
    }
    burstCount.stringValue = count.map(String.init) ?? ""
    burstCount.isHidden = count == nil
    groupDescription = count.map { String(format: NSLocalizedString("library_burst_count", comment: ""), $0) + ". " } ?? ""
    burstCount.toolTip = NSLocalizedString("library_burst_selection", comment: "")
    setState(state)
  }
  func setState(_ state: String?) {
    status.image = NSImage(systemSymbolName: taskSymbol(state), accessibilityDescription: nil)
    status.contentTintColor =
      state == "received" ? .systemGreen : state == "failed" ? .systemOrange : .white
    status.toolTip = groupDescription + NSLocalizedString(
      state.map { "state_" + $0 } ?? "state_not_queued", comment: "")
    view.setAccessibilityValue(status.toolTip)
  }
  func cancel() {
    pipeline?.cancel(request)
    request = PHInvalidImageRequestID
    token = UUID()
    imageKey = ""
  }
  override func prepareForReuse() {
    super.prepareForReuse()
    cancel()
    photoLayer.contents = nil
    assetID = nil
    presentationID = nil
  }
}

@MainActor private final class LibraryScrollView: NSScrollView {
  var restoreScroll: ((LibraryScrollView) -> Void)?
  override func layout() {
    super.layout()
    if let collection = documentView as? NSCollectionView,
      collection.frame.width != contentSize.width
    {
      collection.setFrameSize(NSSize(width: contentSize.width, height: collection.frame.height))
      collection.collectionViewLayout?.invalidateLayout()
    }
    if contentSize.width > 0, contentSize.height > 0, let restore = restoreScroll {
      restoreScroll = nil
      restore(self)
    }
  }
}
