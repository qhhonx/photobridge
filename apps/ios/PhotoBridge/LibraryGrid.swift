import Photos
import SwiftUI
import UIKit

struct IOSLibraryGrid: UIViewRepresentable {
  @ObservedObject var library: PhotoLibraryModel
  var preparationRevision: String = ""
  var queueRevision: Int64
  var receiverID: String?
  func makeCoordinator() -> Coordinator { Coordinator(self) }
  func makeUIView(context: Context) -> UICollectionView {
    let layout = UICollectionViewFlowLayout()
    layout.minimumLineSpacing = 3
    layout.minimumInteritemSpacing = 3
    layout.sectionInset = UIEdgeInsets(top: 4, left: 8, bottom: 100, right: 8)
    let view = LibraryCollectionView(frame: .zero, collectionViewLayout: layout)
    view.backgroundColor = .clear
    view.allowsMultipleSelection = true
    view.register(PhotoCell.self, forCellWithReuseIdentifier: "photo")
    view.dataSource = context.coordinator
    view.delegate = context.coordinator
    context.coordinator.collection = view
    return view
  }
  func updateUIView(_ view: UICollectionView, context: Context) {
    let c = context.coordinator
    c.parent = self
    if c.generation != library.generation {
      let sameFilter = (c.filter == nil || c.filter == library.filter)
        && (c.scrollResetGeneration == nil || c.scrollResetGeneration == library.scrollResetGeneration)
      if sameFilter, c.restored { c.rememberScroll() }
      let savedOffset = sameFilter ? library.scrollOffset : CGPoint.zero
      let savedAnchor = sameFilter ? library.scrollAnchorID : nil
      let savedInset = library.scrollAnchorInset
      c.filter = library.filter
      c.scrollResetGeneration = library.scrollResetGeneration
      c.generation = library.generation
      c.count = library.loaded
      c.restored = false
      c.applyingSelection = true
      c.appliedSelection = nil
      view.reloadData()
      // SwiftUI can create the collection with zero bounds. Restore only after
      // the native layout has a viewport, otherwise UIKit clamps to the top.
      (view as? LibraryCollectionView)?.restoreScroll = { [weak c] view in
        guard let c else { return }
        var offset = savedOffset
        if let index = library.index(of: savedAnchor), index < c.count,
          let frame = view.collectionViewLayout.layoutAttributesForItem(
            at: IndexPath(item: index, section: 0))?.frame {
          offset.y = frame.minY + savedInset
        }
        offset.y = min(max(-view.adjustedContentInset.top, offset.y),
          max(-view.adjustedContentInset.top,
            view.contentSize.height - view.bounds.height + view.adjustedContentInset.bottom))
        view.setContentOffset(offset, animated: false)
        c.restored = true
        c.applyingSelection = false
        c.scrolled()
      }
      view.setNeedsLayout()
      view.layoutIfNeeded()
    } else if c.count < library.loaded {
      let old = c.count
      c.count = library.loaded
      c.appliedSelection = nil
      view.insertItems(at: (old..<c.count).map { IndexPath(item: $0, section: 0) })
    }
    if c.contentGeneration != library.contentGeneration {
      c.contentGeneration = library.contentGeneration
      for path in view.indexPathsForVisibleItems {
        if let cell = view.cellForItem(at: path) as? PhotoCell { c.configure(cell, path: path) }
      }
      c.scrolled()
    }
    if c.preparationRevision != preparationRevision {
      c.preparationRevision = preparationRevision
      c.scrolled()
    }
    if c.queueRevision != queueRevision || c.receiverID != receiverID {
      c.queueRevision = queueRevision
      c.receiverID = receiverID
      c.scrolled()
    }
    if c.appliedSelection != library.selection {
      c.applyingSelection = true
      let desired = library.selectedIndexPaths()
      let current = Set(view.indexPathsForSelectedItems ?? [])
      current.subtracting(desired).forEach { view.deselectItem(at: $0, animated: false) }
      desired.subtracting(current).forEach {
        view.selectItem(at: $0, animated: false, scrollPosition: [])
      }
      c.appliedSelection = library.selection
      c.applyingSelection = false
    }
  }
  static func dismantleUIView(_ view: UICollectionView, coordinator: Coordinator) {
    coordinator.rememberScroll()
    coordinator.refresh.cancel()
    coordinator.parent.library.thumbnails.clear()
  }
  @MainActor
  final class Coordinator: NSObject, UICollectionViewDataSource, UICollectionViewDelegateFlowLayout
  {
    var parent: IOSLibraryGrid
    weak var collection: UICollectionView?
    let refresh = LibraryRefreshScheduler()
    var count = 0, generation = -1
    var preparationRevision = ""
    var contentGeneration = -1
    var queueRevision: Int64 = -2
    var receiverID: String?
    var restored = false
    var applyingSelection = false
    var appliedSelection: Set<String>?
    var filter: LibraryFilter?
    var scrollResetGeneration: Int?
    init(_ parent: IOSLibraryGrid) { self.parent = parent }
    func collectionView(_ collectionView: UICollectionView, numberOfItemsInSection section: Int)
      -> Int
    { count }
    func collectionView(
      _ collectionView: UICollectionView, layout collectionViewLayout: UICollectionViewLayout,
      sizeForItemAt indexPath: IndexPath
    ) -> CGSize {
      let columns = max(3, floor(collectionView.bounds.width / 135))
      let side = floor((collectionView.bounds.width - 16 - (columns - 1) * 3) / columns)
      return CGSize(width: side, height: side)
    }
    func collectionView(_ collectionView: UICollectionView, cellForItemAt indexPath: IndexPath)
      -> UICollectionViewCell
    {
      let cell =
        collectionView.dequeueReusableCell(withReuseIdentifier: "photo", for: indexPath)
        as! PhotoCell
      configure(cell, path: indexPath)
      return cell
    }
    func collectionView(
      _ collectionView: UICollectionView, willDisplay cell: UICollectionViewCell,
      forItemAt indexPath: IndexPath
    ) {
      if let cell = cell as? PhotoCell { configure(cell, path: indexPath) }
      scrolled()
    }
    func configure(_ cell: PhotoCell, path: IndexPath) {
      if let asset = parent.library.asset(at: path.item) {
        cell.configure(asset, pipeline: parent.library.thumbnails, pixels: 400,
          identifier: parent.library.identifier(at: path.item))
      }
    }
    func collectionView(
      _ collectionView: UICollectionView, didEndDisplaying cell: UICollectionViewCell,
      forItemAt indexPath: IndexPath
    ) { (cell as? PhotoCell)?.cancel() }
    func collectionView(_ collectionView: UICollectionView, didSelectItemAt indexPath: IndexPath) {
      guard !applyingSelection, let id = parent.library.identifier(at: indexPath.item) else { return }
      parent.library.selection.insert(id)
      appliedSelection = parent.library.selection
    }
    func collectionView(_ collectionView: UICollectionView, didDeselectItemAt indexPath: IndexPath) {
      guard !applyingSelection, let id = parent.library.identifier(at: indexPath.item) else { return }
      parent.library.selection.remove(id)
      appliedSelection = parent.library.selection
    }
    func scrollViewDidScroll(_ scrollView: UIScrollView) {
      if restored, generation == parent.library.generation { rememberScroll() }
      scrolled()
    }
    func rememberScroll() {
      guard restored, parent.library.authorized, filter == parent.library.filter,
        scrollResetGeneration == parent.library.scrollResetGeneration, let collection,
        let top = collection.indexPathsForVisibleItems.sorted().first,
        let id = (collection.cellForItem(at: top) as? PhotoCell)?.presentationID,
        let frame = collection.collectionViewLayout.layoutAttributesForItem(at: top)?.frame,
        frame.intersects(collection.bounds)
      else { return }
      parent.library.scrollOffset = collection.contentOffset
      parent.library.scrollAnchorID = id
      parent.library.scrollAnchorInset = collection.contentOffset.y - frame.minY
    }
    func scrolled() {
      guard restored else { return }
      refresh.schedule { [weak self] in
        guard let self, !Task.isCancelled, let collection = self.collection else { return }
        let parent = self.parent
        self.rememberScroll()
        let generation = parent.library.generation
        let receiverID = parent.receiverID
        let revision = parent.queueRevision
        let preparation = parent.preparationRevision
        let paths = collection.indexPathsForVisibleItems.sorted()
        guard let first = paths.first?.item, let last = paths.last?.item else { return }
        let identities = Dictionary(uniqueKeysWithValues: paths.compactMap { path in
          parent.library.asset(at: path.item).map { (path, $0.localIdentifier) }
        })
        if last >= parent.library.loaded - 36 { parent.library.loadMore() }
        let range = max(0, first - 18)..<min(parent.library.loaded, last + 37)
        parent.library.thumbnails.preheat(
          range.compactMap { parent.library.asset(at: $0) }, pixels: 400)
        let visible = paths.compactMap { parent.library.asset(at: $0.item) }
        let groups = await parent.library.visibleGroups(visible)
        guard !Task.isCancelled, generation == self.parent.library.generation else { return }
        guard let states = await librarySourceStates(groups, receiver: receiverID) else { return }
        guard !Task.isCancelled, generation == self.parent.library.generation,
          receiverID == self.parent.receiverID, revision == self.parent.queueRevision,
          preparation == self.parent.preparationRevision else { return }
        for path in paths {
          if let cell = collection.cellForItem(at: path) as? PhotoCell {
            guard cell.assetID == identities[path] else { continue }
            let group = groups[cell.assetID ?? ""]
            let burst = parent.library.asset(at: path.item)?.burstIdentifier != nil
            cell.setGroup(count: burst ? group?.count : nil, state: group?.state(states))
          }
        }
      }
    }
  }
}

@MainActor private final class LibraryCollectionView: UICollectionView {
  var restoreScroll: ((LibraryCollectionView) -> Void)?
  override func layoutSubviews() {
    super.layoutSubviews()
    guard bounds.width > 0, bounds.height > 0, let restore = restoreScroll else { return }
    restoreScroll = nil
    restore(self)
  }
}

@MainActor final class PhotoCell: UICollectionViewCell {
  private let photo = UIImageView()
  private let media = UIImageView()
  private let status = UIImageView()
  private let cloud = UIImageView()
  private let burstCount = UILabel()
  private var groupDescription = ""
  private var request: PHImageRequestID = PHInvalidImageRequestID
  private var pipeline: ThumbnailPipeline?
  private var token = UUID(), key = ""
  private(set) var assetID: String?
  private(set) var presentationID: String?
  override init(frame: CGRect) {
    super.init(frame: frame)
    contentView.backgroundColor = .tertiarySystemFill
    photo.contentMode = .scaleAspectFill
    photo.clipsToBounds = true
    contentView.addSubview(photo)
    for badge in [media, status, cloud] {
      badge.tintColor = .white
      badge.backgroundColor = .black.withAlphaComponent(0.35)
      badge.layer.cornerRadius = 11
      badge.clipsToBounds = true
      badge.contentMode = .scaleAspectFit
      contentView.addSubview(badge)
    }
    burstCount.textColor = .white
    burstCount.backgroundColor = .black.withAlphaComponent(0.45)
    burstCount.font = .monospacedDigitSystemFont(ofSize: 12, weight: .medium)
    burstCount.textAlignment = .center
    burstCount.layer.cornerRadius = 9
    burstCount.clipsToBounds = true
    burstCount.adjustsFontSizeToFitWidth = true
    burstCount.isHidden = true
    contentView.addSubview(burstCount)
    cloud.isHidden = true
    media.isHidden = true
    isAccessibilityElement = true
  }
  required init?(coder: NSCoder) { fatalError("init(coder:) is not supported") }
  override func layoutSubviews() {
    super.layoutSubviews()
    photo.frame = contentView.bounds
    media.frame = CGRect(x: 6, y: 6, width: 22, height: 22)
    burstCount.frame = CGRect(x: 31, y: 6, width: 42, height: 22)
    cloud.frame = CGRect(x: 6, y: bounds.height - 28, width: 22, height: 22)
    status.frame = CGRect(x: bounds.width - 28, y: bounds.height - 28, width: 22, height: 22)
  }
  override var isSelected: Bool {
    didSet {
      contentView.layer.borderWidth = isSelected ? 3 : 0
      contentView.layer.borderColor = UIColor.systemBlue.cgColor
    }
  }
  func configure(_ asset: PHAsset, pipeline: ThumbnailPipeline, pixels: CGFloat, identifier: String? = nil) {
    presentationID = identifier ?? asset.localIdentifier
    let next = asset.localIdentifier + PhotoLibraryModel.revision(asset)
    guard key != next else { return }
    let sameAsset = assetID == asset.localIdentifier
    cancel()
    self.pipeline = pipeline
    key = next
    assetID = asset.localIdentifier
    let expected = token
    photo.image = nil
    cloud.isHidden = true
    let symbol =
      asset.burstIdentifier != nil ? "square.stack" : asset.mediaSubtypes.contains(.photoLive)
      ? "livephoto" : asset.mediaType == .video ? "video.fill" : nil
    media.image = symbol.flatMap { UIImage(systemName: $0) }
    media.isHidden = symbol == nil
    accessibilityLabel = (asset.creationDate?.formatted(date: .abbreviated, time: .shortened) ?? "") + (asset.burstIdentifier != nil ? " " + NSLocalizedString("library_filter_burst", comment: "") : "")
    if !sameAsset { setGroup(count: nil, state: nil); photo.image = nil }
    request = pipeline.request(asset, pixels: pixels) { [weak self] image, inCloud in
      guard let self, expected == self.token else { return }
      if let image { self.photo.image = image }
      self.cloud.image = UIImage(systemName: "icloud")
      self.cloud.isHidden = self.photo.image != nil || !inCloud
    }
  }
  func setGroup(count: Int?, state: String?) {
    if count != nil {
      media.image = UIImage(systemName: "square.stack")
      media.isHidden = false
    }
    accessibilityHint = count == nil ? nil : NSLocalizedString("library_burst_selection", comment: "")
    burstCount.text = count.map(String.init)
    burstCount.isHidden = count == nil
    groupDescription = count.map { String(format: NSLocalizedString("library_burst_count", comment: ""), $0) + ". " } ?? ""
    setState(state)
  }
  func setState(_ state: String?) {
    status.image = UIImage(systemName: taskSymbol(state))
    status.tintColor =
      state == "received" ? .systemGreen : state == "failed" ? .systemOrange : .white
    accessibilityValue = groupDescription + NSLocalizedString(
      state.map { "state_" + $0 } ?? "state_not_queued", comment: "")
  }
  func cancel() {
    pipeline?.cancel(request)
    request = PHInvalidImageRequestID
    token = UUID()
    key = ""
  }
  override func prepareForReuse() {
    super.prepareForReuse()
    cancel()
    photo.image = nil
    assetID = nil
    presentationID = nil
  }
}

struct IOSLibraryPage: View {
  @ObservedObject var library: PhotoLibraryModel
  @ObservedObject var model: BackupModel
  var body: some View {
    NavigationStack {
      VStack(spacing: 10) {
        HStack {
          if library.authorized, library.fetch != nil {
            Text(String(format: NSLocalizedString("library_count", comment: ""), library.total)).font(
              .subheadline
            ).foregroundStyle(.secondary)
          }
          Spacer()
          Picker(
            "media_type",
            selection: Binding(
              get: { library.filter }, set: { value in Task { await library.changeFilter(value) } })
          ) {
            ForEach(LibraryFilter.allCases) { Text(LocalizedStringKey($0.title)).tag($0) }
          }.labelsHidden().disabled(!library.authorized)
        }.padding(.horizontal, 16)
        if let placeholder = library.placeholder {
          LibraryPlaceholder(kind: placeholder,
            requestingAccess: library.requestingPermission,
            requestAccess: { Task { await library.open(requestPermission: true) } },
            clearFilter: { Task { await library.changeFilter(.all) } })
        } else {
          IOSLibraryGrid(
            library: library, preparationRevision: "\(model.pendingImports)|\(model.importingSourceID ?? "")", queueRevision: model.queueRevision,
            receiverID: model.pairing?.receiverID)
        }
      }
      .ignoresSafeArea(.container, edges: .bottom)
      .navigationTitle("nav_library").navigationBarTitleDisplayMode(.inline)
      .toolbar {
        ToolbarItem(placement: .topBarTrailing) {
          Button {
            let tokens = library.selection
            Task {
              let ids = await library.selectedAssetIdentifiers(tokens)
              await model.setPaused(false)
              await model.importAssets(ids)
              library.selection.subtract(tokens)
            }
          } label: {
            Label(
              String(
                format: NSLocalizedString("backup_selected", comment: ""), library.selection.count),
              systemImage: "arrow.up.circle")
          }
          .disabled(library.selection.isEmpty || model.pairing == nil || model.importing)
        }
      }
      .task { await library.open() }
    }
  }
}
