import XCTest

final class SettingsTests: XCTestCase {
  @MainActor func testCompactStatusIndicatorsOpenDetails() {
    let app = XCUIApplication()
    app.launchArguments = ["-AppleLanguages", "(en)", "-AppleLocale", "en_US"]
    app.launch()
    XCTAssertTrue(app.tabBars.buttons.element(boundBy: 1).waitForExistence(timeout: 20))
    app.tabBars.buttons.element(boundBy: 1).tap()
    let backup = app.buttons["backup.status_indicator"]
    XCTAssertTrue(backup.waitForExistence(timeout: 10))
    XCTAssertLessThanOrEqual(backup.frame.height, 48)
    backup.tap()
    XCTAssertTrue(app.buttons["Done"].waitForExistence(timeout: 10))
    app.buttons["Done"].tap()
    app.tabBars.buttons.element(boundBy: 2).tap()
    let receiver = app.buttons["receiver.status_indicator"]
    XCTAssertTrue(receiver.waitForExistence(timeout: 10))
    XCTAssertLessThanOrEqual(receiver.frame.height, 48)
    receiver.tap()
    XCTAssertTrue(app.buttons["Done"].waitForExistence(timeout: 10))
    app.buttons["Done"].tap()
  }

  @MainActor func testTaskExplanationsAndTrailingFilter() {
    continueAfterFailure = false
    let app = XCUIApplication()
    app.launchArguments = ["-AppleLanguages", "(en)", "-AppleLocale", "en_US"]
    app.launch()
    let backup = app.tabBars.buttons.element(boundBy: 1)
    XCTAssertTrue(backup.waitForExistence(timeout: 20))
    backup.tap()
    XCTAssertFalse(app.staticTexts["backup.receipt_explanation"].exists)
    for state in ["queued", "received"] {
      let entry = app.buttons["backup.filter.\(state)"]
      for _ in 0..<8 {
        if entry.exists && entry.isHittable { break }
        app.swipeUp()
      }
      XCTAssertTrue(entry.isHittable)
      entry.tap()
      let explanation = app.staticTexts["transfers.explanation"]
      XCTAssertTrue(explanation.waitForExistence(timeout: 10))
      XCTAssertEqual(app.staticTexts["transfers.title"].label, state == "queued" ? "Queued" : "Received")
      XCTAssertEqual(app.staticTexts["transfers.count"].label, "0 items")
      XCTAssertTrue(explanation.label.contains(state == "queued" ? "Files are ready" : "received and verified"))
      let filter = app.descendants(matching: .any)["transfers.filter"].firstMatch
      XCTAssertTrue(filter.exists)
      XCTAssertGreaterThan(filter.frame.maxX, app.frame.maxX - 40)
      XCTAssertLessThan(filter.frame.maxY, explanation.frame.minY)
      XCTAssertFalse(app.staticTexts["backup.receipt_explanation"].exists)
      let screen = XCTAttachment(screenshot: app.screenshot())
      screen.name = "Task explanation — \(state)"; screen.lifetime = .keepAlways; add(screen)
      app.navigationBars.buttons.element(boundBy: 0).tap()
    }
  }

  @MainActor func testTransferSortPersistsAcrossListsAndRelaunch() {
    continueAfterFailure = false
    let app = XCUIApplication()
    app.launchArguments = ["-AppleLanguages", "(en)", "-AppleLocale", "en_US"]
    func openList(_ state: String) {
      let backup = app.tabBars.buttons.element(boundBy: 1)
      XCTAssertTrue(backup.waitForExistence(timeout: 20))
      backup.tap()
      let entry = app.buttons["backup.filter.\(state)"]
      for _ in 0..<8 {
        if entry.exists && entry.isHittable { break }
        app.swipeUp()
      }
      XCTAssertTrue(entry.isHittable)
      entry.tap()
      XCTAssertTrue(app.staticTexts["transfers.sort_description"].waitForExistence(timeout: 10))
    }
    app.launch()
    openList("received")
    app.buttons["transfers.sort"].tap()
    app.buttons["transfers.sort_recommended"].tap()
    app.buttons["transfers.sort"].tap()
    app.buttons["transfers.sort_descending"].tap()
    XCTAssertEqual(app.staticTexts["transfers.sort_description"].label, "Capture time · newest first")
    app.buttons["transfers.sort"].tap()
    app.buttons["transfers.sort_ascending"].tap()
    XCTAssertEqual(app.staticTexts["transfers.sort_description"].label, "Capture time · oldest first")
    app.navigationBars.buttons.element(boundBy: 0).tap()
    openList("preparing")
    XCTAssertEqual(app.staticTexts["transfers.sort_description"].label, "Capture time · newest first")
    app.terminate()
    app.launch()
    openList("received")
    XCTAssertEqual(app.staticTexts["transfers.sort_description"].label, "Capture time · oldest first")
    app.buttons["transfers.sort"].tap()
    app.buttons["transfers.sort_by_activity"].tap()
    XCTAssertEqual(app.staticTexts["transfers.sort_description"].label, "Receipt time · newest first")
    app.navigationBars.buttons.element(boundBy: 0).tap()
    openList("scanned")
    XCTAssertEqual(app.staticTexts["transfers.sort_description"].label, "Capture time · newest first")
    app.buttons["transfers.sort"].tap()
    app.buttons["transfers.sort_descending"].tap()
    XCTAssertEqual(app.staticTexts["transfers.sort_description"].label, "Capture time · newest first")
    let screen = XCTAttachment(screenshot: app.screenshot())
    screen.name = "Transfer sorting"; screen.lifetime = .keepAlways; add(screen)
  }

  @MainActor func testBackupPreparationAndScanDrilldowns() {
    continueAfterFailure = false
    let app = XCUIApplication()
    app.launchArguments = ["-AppleLanguages", "(en)", "-AppleLocale", "en_US"]
    app.launch()
    let backup = app.tabBars.buttons.element(boundBy: 1)
    XCTAssertTrue(backup.waitForExistence(timeout: 20))
    backup.tap()
    for state in ["preparing", "scanned"] {
      let entry = app.buttons["backup.filter.\(state)"]
      for _ in 0..<8 {
        if entry.exists && entry.isHittable { break }
        app.swipeUp()
      }
      XCTAssertTrue(entry.isHittable)
      entry.tap()
      XCTAssertTrue(app.staticTexts["0 photos · 0 loaded"].waitForExistence(timeout: 10))
      let screen = XCTAttachment(screenshot: app.screenshot())
      screen.name = "Backup source drilldown — \(state)"
      screen.lifetime = .keepAlways
      add(screen)
      app.navigationBars.buttons.element(boundBy: 0).tap()
    }
  }

  @MainActor func testAccessibleTextNavigationAndRotation() {
    continueAfterFailure = false
    let app = XCUIApplication()
    app.launchArguments = ["-AppleLanguages", "(en)", "-AppleLocale", "en_US",
      "-UIPreferredContentSizeCategoryName", "UICTContentSizeCategoryAccessibilityXXXL"]
    app.launch()
    defer { XCUIDevice.shared.orientation = .portrait }
    for orientation in [UIDeviceOrientation.portrait, .landscapeLeft] {
      XCUIDevice.shared.orientation = orientation
      let settings = app.buttons["Settings"].firstMatch
      XCTAssertTrue(settings.waitForExistence(timeout: 20))
      for title in ["Library", "Backup", "Receiver", "Settings"] {
        let tab = app.buttons[title].firstMatch
        XCTAssertTrue(tab.isHittable, "Navigation must remain reachable at accessibility sizes")
        tab.tap()
        let screenshot = XCTAttachment(screenshot: XCUIScreen.main.screenshot())
        screenshot.name = "Accessible text — \(title) — \(orientation.rawValue)"
        screenshot.lifetime = .keepAlways
        add(screenshot)
      }
      let help = app.buttons["settings.help"]
      for _ in 0..<12 {
        if help.exists && help.isHittable { break }
        app.swipeUp()
      }
      XCTAssertTrue(help.isHittable)
      help.tap()
      let body = app.staticTexts.matching(NSPredicate(
        format: "label BEGINSWITH %@", "Open Settings → Diagnostics → Activity log")).firstMatch
      for _ in 0..<25 {
        if body.exists && body.isHittable { break }
        app.swipeUp()
      }
      XCTAssertTrue(body.isHittable)
      let screenshot = XCTAttachment(screenshot: XCUIScreen.main.screenshot())
      screenshot.name = "Accessible help end — \(orientation.rawValue)"
      screenshot.lifetime = .keepAlways
      add(screenshot)
      app.navigationBars.buttons.element(boundBy: 0).tap()
    }
  }

  @MainActor func testHelpNavigationAndScrollingInBothLanguages() {
    continueAfterFailure = false
    for (language, title, finalSection, finalBody) in [
      ("en", "Using PhotoBridge", "Collect useful diagnostics", "Open Settings → Diagnostics → Activity log"),
      ("zh-Hans", "使用与恢复", "提供排查信息", "在“设置 → 诊断 → 运行记录”中"),
    ] {
      let app = XCUIApplication()
      app.launchArguments = ["-AppleLanguages", "(\(language))"]
      app.launch()
      XCTAssertTrue(app.tabBars.buttons.element(boundBy: 3).waitForExistence(timeout: 20))
      app.tabBars.buttons.element(boundBy: 3).tap()
      let help = app.buttons["settings.help"]
      for _ in 0..<8 {
        if help.exists && help.isHittable { break }
        app.swipeUp()
      }
      XCTAssertTrue(help.isHittable)
      help.tap()
      XCTAssertTrue(app.navigationBars[title].waitForExistence(timeout: 10))
      let end = app.staticTexts[finalSection]
      let body = app.staticTexts.matching(NSPredicate(format: "label BEGINSWITH %@", finalBody)).firstMatch
      for _ in 0..<12 {
        if body.exists && body.isHittable && body.frame.maxY < app.tabBars.firstMatch.frame.minY { break }
        app.swipeUp()
      }
      XCTAssertTrue(end.isHittable, "The final help section must remain reachable")
      XCTAssertTrue(body.isHittable)
      XCTAssertLessThan(body.frame.maxY, app.tabBars.firstMatch.frame.minY,
        "The last paragraph must scroll clear of the tab bar")
      let image = XCTAttachment(screenshot: app.screenshot())
      image.name = "Help end — \(language)"
      image.lifetime = .keepAlways
      add(image)
      app.navigationBars.buttons.element(boundBy: 0).tap()
      XCTAssertTrue(app.buttons["settings.help"].waitForExistence(timeout: 5))
      app.terminate()
    }
  }

  @MainActor func testActivityLogOpensIndependentlyAndExports() {
    continueAfterFailure = false
    let app = XCUIApplication()
    app.launchArguments = ["-AppleLanguages", "(en)", "-AppleLocale", "en_US"]
    app.launch()
    let settings = app.tabBars.buttons.element(boundBy: 3)
    XCTAssertTrue(settings.waitForExistence(timeout: 20))
    settings.tap()
    let log = app.buttons["settings.activity_log"]
    for _ in 0..<8 {
      if log.exists && log.isHittable { break }
      app.swipeUp()
    }
    XCTAssertTrue(log.isHittable)
    log.tap()
    let export = app.buttons["activity.export"]
    XCTAssertTrue(
      export.waitForExistence(timeout: 10),
      "Log entry must open its own sheet, not a storage picker")
    let enabled = NSPredicate(format: "enabled == true")
    expectation(for: enabled, evaluatedWith: export)
    waitForExpectations(timeout: 10)
    let details = app.buttons["activity.details"].firstMatch
    for _ in 0..<12 {
      if details.exists && details.isHittable { break }
      app.swipeUp()
    }
    XCTAssertTrue(details.exists && details.isHittable)
    details.tap()
    let expandedFrame = details.frame
    Thread.sleep(forTimeInterval: 3)
    XCTAssertEqual(details.frame.minY, expandedFrame.minY, accuracy: 1)
    XCTAssertEqual(details.value as? String, "Expanded")
    let screen = XCTAttachment(screenshot: app.screenshot())
    screen.name = "Activity log with export action"
    screen.lifetime = .keepAlways
    add(screen)
    export.tap()
    XCTAssertTrue(
      app.otherElements["ActivityListView"].waitForExistence(timeout: 10),
      "Export must present the native sharing sheet")
    XCTAssertTrue(
      app.descendants(matching: .any).matching(
        NSPredicate(format: "label CONTAINS %@", "PhotoBridge-diagnostics")
      ).firstMatch.exists, "The shared item must be the diagnostic file")
    let shared = XCTAttachment(screenshot: app.screenshot())
    shared.name = "Diagnostic file sharing sheet"
    shared.lifetime = .keepAlways
    add(shared)
  }
  @MainActor func testMainNavigationAndCacheIsolation() {
    continueAfterFailure = false
    let app = XCUIApplication()
    app.launchArguments = ["-AppleLanguages", "(en)", "-AppleLocale", "en_US"]
    app.launch()
    XCTAssertTrue(app.tabBars.buttons.element(boundBy: 3).waitForExistence(timeout: 20))
    XCTAssertEqual(app.tabBars.buttons.count, 4)
    for (index, title) in ["Library", "Backup", "Receiver", "Settings"].enumerated() {
      app.tabBars.buttons.element(boundBy: index).tap()
      let screen = XCTAttachment(screenshot: app.screenshot())
      screen.name = title
      screen.lifetime = .keepAlways
      add(screen)
    }
    XCTAssertFalse(app.buttons["settings.cache_budget"].exists)
    app.buttons["settings.cache"].tap()
    XCTAssertTrue(app.buttons["settings.cache_budget"].waitForExistence(timeout: 10))
    XCTAssertFalse(
      app.buttons["settings.activity_log"].exists,
      "Cache controls must not contain the unrelated log action")
    app.tabBars.buttons.element(boundBy: 2).tap()
    XCTAssertTrue(app.buttons["receiver.scan"].waitForExistence(timeout: 10))
    app.tabBars.buttons.element(boundBy: 1).tap()
    app.buttons["backup.all_tasks"].tap()
    XCTAssertTrue(
      app.otherElements["transfers.filter"].exists || app.buttons["transfers.filter"].exists)
  }

}
