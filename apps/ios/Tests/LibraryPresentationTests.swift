import XCTest
import UIKit
@testable import PhotoBridge

final class LibraryPresentationTests: XCTestCase {
  func testBurstFramesAcrossPagesStayOneRowAndPreserveIdentity() {
    var records = (0..<1000).map { LibraryPresentationIndex.Metadata(id: "photo-\($0)", burst: nil, representative: false) }
    records[0] = .init(id: "burst-a-first", burst: "a", representative: false)
    records[1] = .init(id: "burst-a-second", burst: "a", representative: false)
    records[500] = .init(id: "burst-a-picked", burst: "a", representative: true)
    var index = LibraryPresentationIndex()
    var reads = 0
    index.load(sourceCount: records.count, targetRows: 240) { reads += 1; return records[$0] }
    XCTAssertEqual(index.rows.count, 240)
    XCTAssertEqual(index.cursor, 241)
    XCTAssertEqual(reads, 241, "Initial presentation must not enumerate the entire library")
    XCTAssertEqual(index.rows[0].key, "burst:a")
    XCTAssertEqual(index.rows[0].rawIndex, 0)
    let changed = index.load(sourceCount: records.count, targetRows: 720) { records[$0] }
    XCTAssertTrue(changed)
    XCTAssertEqual(index.rows[0].rawIndex, 500)
    XCTAssertEqual(index.positions["burst:a"], 0)
    XCTAssertEqual(index.rows.filter { $0.key == "burst:a" }.count, 1)
  }

  func testCancelledMetadataWorkStopsReadingAnOversizedBurst() {
    var index = LibraryPresentationIndex()
    var reads = 0
    index.load(sourceCount: 1_000_000, targetRows: 240, shouldContinue: { reads < 100 }) { i in
      reads += 1
      return .init(id: "frame-\(i)", burst: "a", representative: false)
    }
    XCTAssertEqual(reads, 100)
    XCTAssertEqual(index.cursor, 100)
    XCTAssertEqual(index.rows.count, 1)
  }

  func testLargeBurstDoesNotCreateEmptyPaginationAndAnchorCanExpandWindow() {
    let records = (0..<1200).map { LibraryPresentationIndex.Metadata(id: "frame-\($0)", burst: $0 < 1100 ? "a" : nil, representative: $0 == 900) }
    var index = LibraryPresentationIndex()
    index.load(sourceCount: records.count, targetRows: 20) { records[$0] }
    XCTAssertEqual(index.rows.count, 20)
    XCTAssertEqual(index.cursor, 1119)
    XCTAssertEqual(index.rows[0].rawIndex, 900)
    index.load(sourceCount: records.count, targetRows: 20, minimumCursor: 1200) { records[$0] }
    XCTAssertEqual(index.rows.count, 101)
    XCTAssertEqual(index.cursor, 1200)
  }

  @MainActor func testNativeBurstBadgesAndAccessibilityReset() async throws {
    let scene = try XCTUnwrap(UIApplication.shared.connectedScenes.compactMap { $0 as? UIWindowScene }.first)
    let window = UIWindow(windowScene: scene)
    window.frame = scene.coordinateSpace.bounds
    let controller = UIViewController()
    window.rootViewController = controller
    window.makeKeyAndVisible()
    defer { window.isHidden = true; window.rootViewController = nil }
    for style in [UIUserInterfaceStyle.light, .dark] {
      let panel = UIView(frame: CGRect(x: 0, y: 0, width: 390, height: 160))
      window.overrideUserInterfaceStyle = style
      panel.overrideUserInterfaceStyle = style
      panel.backgroundColor = .systemBackground
      controller.view.addSubview(panel)
      var cells: [PhotoCell] = []
      for (index, state) in ["received", "partial", "failed"].enumerated() {
        let cell = PhotoCell(frame: CGRect(x: 8 + index * 128, y: 8, width: 120, height: 140))
        cell.contentView.backgroundColor = [.systemTeal, .systemIndigo, .systemBrown][index]
        panel.addSubview(cell)
        cell.setGroup(count: [8, 32, 1200][index], state: state)
        cell.isSelected = index == 1
        cell.layoutIfNeeded()
        XCTAssertTrue(cell.accessibilityValue?.contains(String(format: NSLocalizedString("library_burst_count", comment: ""), [8, 32, 1200][index])) == true)
        XCTAssertEqual(cell.accessibilityHint, NSLocalizedString("library_burst_selection", comment: ""))
        cells.append(cell)
      }
      panel.layoutIfNeeded()
      try await Task.sleep(nanoseconds: 100_000_000)
      let image = UIGraphicsImageRenderer(bounds: panel.bounds).image { _ in
        XCTAssertTrue(panel.drawHierarchy(in: panel.bounds, afterScreenUpdates: true))
      }
      let attachment = XCTAttachment(image: image)
      attachment.name = style == .dark ? "burst-cells-dark" : "burst-cells-light"
      attachment.lifetime = .keepAlways
      add(attachment)
      cells[0].setGroup(count: nil, state: nil)
      XCTAssertNil(cells[0].accessibilityHint)
      XCTAssertEqual(cells[0].accessibilityValue, NSLocalizedString("state_not_queued", comment: ""))
      panel.removeFromSuperview()
    }
  }

  func testWholeBurstStatusCannotBeInferredFromCoverReceipt() {
    let group = LibraryGroup(sources: (0..<3).map { LibrarySource(id: "\($0)", revision: "1") }, complete: true)
    XCTAssertNil(group.state([:]))
    XCTAssertEqual(group.state(["0": "received"]), "partial")
    XCTAssertEqual(group.state(["0": "received", "1": "running"]), "running")
    XCTAssertEqual(group.state(["0": "received", "1": "failed", "2": "running"]), "failed")
    XCTAssertEqual(group.state(["0": "received", "1": "received", "2": "received"]), "received")
    XCTAssertEqual(group.state(["0": "received", "1": "received", "2": "waiting"]), "waiting")
    XCTAssertNil(LibraryGroup(sources: group.sources, complete: false).state(["0": "received", "1": "received", "2": "received"]))
    XCTAssertNil(LibraryGroup(sources: [], complete: true).state([:]))
  }
}
