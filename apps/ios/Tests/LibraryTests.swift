import Photos
import SwiftUI
import UIKit
import XCTest

@testable import PhotoBridge

/// Run only on a disposable simulator; fixtures remain available for UI checks.
@MainActor final class LibraryTests: XCTestCase {
  func testBackupCaptureOrderIgnoresIdentifierAndSelectionOrder() async throws {
    #if !targetEnvironment(simulator)
      throw XCTSkip("Synthetic library acceptance is simulator-only")
    #endif
    let access = await PHPhotoLibrary.requestAuthorization(for: .readWrite)
    guard access == .authorized else { throw XCTSkip("Photos authorization required") }
    let ids = try await createPhotos(count: 8, date: Date())
    let shuffled = [ids[3], ids[0], ids[6], ids[2], ids[7], ids[1], ids[5], ids[4]]
    let dates = await PhotoBackupOrder.captureDates(shuffled)
    XCTAssertEqual(dates.count, ids.count)
    let ordered = shuffled.sorted {
      PhotoBackupOrder.precedes($0, dates[$0] ?? Int64.min, $1, dates[$1] ?? Int64.min)
    }
    XCTAssertEqual(ordered, Array(ids.reversed()))
    XCTAssertTrue(PhotoBackupOrder.precedes("a", 1, "b", 1))
    XCTAssertTrue(PhotoBackupOrder.precedes("new", 1, "missing", Int64.min))
  }

  func testSelectionAcrossPaginationInsertionAndViewRecreation() async throws {
    #if !targetEnvironment(simulator)
      throw XCTSkip("Synthetic library acceptance is simulator-only")
    #endif
    // Resolve the simulator's pre-granted authorization through PhotoKit before
    // querying metadata; its initial cached status may still be notDetermined.
    let access = await PHPhotoLibrary.requestAuthorization(for: .readWrite)
    guard access == .authorized else {
      XCTFail("Grant Photos to the test host on the disposable simulator (status \(access.rawValue))")
      return
    }
    // Prior runs intentionally leave synthetic fixtures available for UI review.
    // Put this run after their inserted dates so its pagination is deterministic.
    let latestOptions = photoLibraryFetchOptions()
    latestOptions.sortDescriptors = [NSSortDescriptor(key: "creationDate", ascending: false)]
    latestOptions.fetchLimit = 1
    let latest = PHAsset.fetchAssets(with: latestOptions).firstObject?.creationDate ?? Date()
    let base = max(Date(), latest).addingTimeInterval(1_000)
    let ids = try await createPhotos(count: 260, date: base)
    let model = PhotoLibraryModel()
    await model.open()
    XCTAssertEqual(model.loaded, 240)
    let chosen = Set([ids[0], ids[238], ids[259]])
    model.selection = chosen
    XCTAssertEqual(model.selectedIndexPaths().count, 2)

    let scene = try XCTUnwrap(UIApplication.shared.connectedScenes.compactMap { $0 as? UIWindowScene }.first)
    let window = UIWindow(windowScene: scene)
    window.frame = scene.coordinateSpace.bounds
    func host() {
      window.rootViewController = UIHostingController(rootView:
        IOSLibraryGrid(library: model, queueRevision: 0, receiverID: nil))
      window.makeKeyAndVisible()
    }
    defer { window.isHidden = true; window.rootViewController = nil }
    host()
    try await waitUntil("initial host: " + String(describing: collection(in: window)?.bounds)) {
      self.collection(in: window)?.indexPathsForSelectedItems?.count == 2
    }
    let grid = try XCTUnwrap(collection(in: window))
    XCTAssertEqual(selectedIDs(grid, model), chosen.subtracting([ids[0]]))
    let stableGeneration = model.generation
    let oldContentGeneration = model.contentGeneration
    let firstPath = try XCTUnwrap(grid.indexPathsForVisibleItems.sorted().first)
    let stableCell = try XCTUnwrap(grid.cellForItem(at: firstPath))
    let changedAsset = try XCTUnwrap(model.asset(at: firstPath.item))
    try await PHPhotoLibrary.shared().performChanges {
      PHAssetChangeRequest(for: changedAsset).isFavorite = !changedAsset.isFavorite
    }
    try await waitUntil("metadata update without grid rebuild") { model.contentGeneration > oldContentGeneration }
    XCTAssertEqual(model.generation, stableGeneration)
    XCTAssertTrue(grid.cellForItem(at: firstPath) === stableCell)

    model.loadMore()
    try await waitUntil("loaded selection") { self.selectedIDs(grid, model) == chosen }

    let originalIndex = try XCTUnwrap(model.selectedIndexPaths().map(\.item).min())
    _ = try await createPhotos(count: 1, date: base.addingTimeInterval(10))
    await model.reload()
    XCTAssertEqual(model.selection, chosen)
    XCTAssertEqual(model.selectedIndexPaths().map(\.item).min(), originalIndex + 1)
    model.loadMore()
    try await waitUntil("loaded selection") { self.selectedIDs(grid, model) == chosen }

    // Recreate the native collection with the same durable presentation model.
    host()
    try await waitUntil("recreated selection") {
      guard let recreated = self.collection(in: window) else { return false }
      return recreated !== grid && self.selectedIDs(recreated, model) == chosen
    }
    let recreated = try XCTUnwrap(collection(in: window))
    let extra = IndexPath(item: 5, section: 0)
    let extraID = try XCTUnwrap(model.asset(at: extra.item)?.localIdentifier)
    recreated.selectItem(at: extra, animated: false, scrollPosition: [])
    recreated.delegate?.collectionView?(recreated, didSelectItemAt: extra)
    XCTAssertEqual(model.selection, chosen.union([extraID]))
    recreated.deselectItem(at: extra, animated: false)
    recreated.delegate?.collectionView?(recreated, didDeselectItemAt: extra)
    XCTAssertEqual(model.selection, chosen)
    model.selection.removeAll()
    try await waitUntil("cleared selection") { recreated.indexPathsForSelectedItems?.isEmpty != false }

    // Preserve the same visible photo and partial-row inset, rather than only
    // the old pixel distance, across navigation and concurrent insertions.
    let scrollPath = IndexPath(item: 160, section: 0)
    let scrollFrame = try XCTUnwrap(recreated.collectionViewLayout.layoutAttributesForItem(at: scrollPath)?.frame)
    recreated.setContentOffset(CGPoint(x: 0, y: scrollFrame.minY + 17), animated: false)
    try await waitUntil("settled scroll bookmark") {
      guard let id = model.scrollAnchorID else { return false }
      return self.anchorIsVisible(id, inset: model.scrollAnchorInset, grid: recreated, model: model)
        && abs(model.scrollAnchorInset) < scrollFrame.height
    }
    let anchor = try XCTUnwrap(model.scrollAnchorID)
    let inset = model.scrollAnchorInset
    let position = try XCTUnwrap(model.index(of: anchor))
    host()
    try await waitUntil("navigation scroll anchor") {
      guard let grid = self.collection(in: window), grid !== recreated else { return false }
      return self.anchorIsVisible(anchor, inset: inset, grid: grid, model: model)
    }
    let afterNavigation = try XCTUnwrap(collection(in: window))
    // More than one page of insertions exercises expansion of the metadata
    // window when the visible photo moves beyond its previous boundary.
    _ = try await createPhotos(count: 250, date: base.addingTimeInterval(1_000))
    await model.reload()
    XCTAssertEqual(model.index(of: anchor), position + 250)
    XCTAssertGreaterThan(model.loaded, position + 250)
    try await waitUntil("insertion scroll anchor") {
      self.anchorIsVisible(anchor, inset: inset, grid: afterNavigation, model: model)
    }

    await model.changeFilter(.video)
    XCTAssertEqual(model.scrollOffset, .zero)
    XCTAssertNil(model.scrollAnchorID)
    await model.changeFilter(.all)
    try await waitUntil("filter resets scroll", details: {
      "offset=\(afterNavigation.contentOffset), inset=\(afterNavigation.adjustedContentInset), anchor=\(String(describing: model.index(of: model.scrollAnchorID)))"
    }) {
      abs(afterNavigation.contentOffset.y) < 1
        && (model.index(of: model.scrollAnchorID) ?? 0) < 3
    }

    model.selection = chosen
    model.scrollOffset = CGPoint(x: 0, y: 500)
    let oldGeneration = model.generation
    XCTAssertFalse(model.applyAuthorization(.denied))
    XCTAssertEqual(model.authorizationStatus, .denied)
    guard case .permission(.denied)? = model.placeholder else {
      return XCTFail("Denied access must show the recovery action instead of an empty library")
    }
    XCTAssertNil(model.fetch)
    XCTAssertEqual(model.loaded, 0)
    XCTAssertEqual(model.total, 0)
    XCTAssertFalse(model.loading)
    XCTAssertTrue(model.selection.isEmpty)
    XCTAssertEqual(model.scrollOffset, .zero)
    XCTAssertNil(model.scrollAnchorID)
    XCTAssertGreaterThan(model.generation, oldGeneration)
    await model.reload()
    XCTAssertNil(model.fetch, "Denied models must not reload metadata")
    XCTAssertTrue(model.applyAuthorization(.authorized))
    XCTAssertEqual(model.authorizationStatus, .authorized)
    guard case .loading? = model.placeholder else {
      return XCTFail("The initial fetch must show loading instead of an empty library")
    }
    await model.reload()
    XCTAssertGreaterThanOrEqual(model.total, 261)
    XCTAssertTrue(model.selection.isEmpty, "Restoring permission must not restore a stale selection")
  }

  private func selectedIDs(_ grid: UICollectionView, _ model: PhotoLibraryModel) -> Set<String> {
    Set((grid.indexPathsForSelectedItems ?? []).compactMap { model.asset(at: $0.item)?.localIdentifier })
  }
  private func anchorIsVisible(_ id: String, inset: CGFloat, grid: UICollectionView,
    model: PhotoLibraryModel) -> Bool {
    guard let index = model.index(of: id),
      let frame = grid.collectionViewLayout.layoutAttributesForItem(at: IndexPath(item: index, section: 0))?.frame,
      let cell = grid.cellForItem(at: IndexPath(item: index, section: 0)) as? PhotoCell
    else { return false }
    return cell.assetID == id && abs(grid.contentOffset.y - frame.minY - inset) < 1
  }
  private func collection(in view: UIView) -> UICollectionView? {
    if let collection = view as? UICollectionView { return collection }
    return view.subviews.lazy.compactMap { self.collection(in: $0) }.first
  }
  private func waitUntil(_ stage: String, details: () -> String = { "" },
    _ condition: () -> Bool) async throws {
    let deadline = Date().addingTimeInterval(10)
    while !condition() {
      guard Date() < deadline else {
        XCTFail("Native collection did not converge: \(stage) \(details())")
        throw NSError(domain: "LibraryTests", code: 1)
      }
      try await Task.sleep(nanoseconds: 50_000_000)
    }
  }
  private func createPhotos(count: Int, date: Date) async throws -> [String] {
    let image = UIGraphicsImageRenderer(size: CGSize(width: 32, height: 32)).image { context in
      UIColor.systemTeal.setFill()
      context.fill(CGRect(x: 0, y: 0, width: 32, height: 32))
    }
    let data = try XCTUnwrap(image.jpegData(compressionQuality: 0.8))
    var ids: [String] = []
    try await PHPhotoLibrary.shared().performChanges {
      for offset in 0..<count {
        let request = PHAssetCreationRequest.forAsset()
        request.creationDate = date.addingTimeInterval(Double(offset - count))
        request.addResource(with: .photo, data: data, options: nil)
        if let id = request.placeholderForCreatedAsset?.localIdentifier { ids.append(id) }
      }
    }
    XCTAssertEqual(ids.count, count)
    return ids
  }
}
