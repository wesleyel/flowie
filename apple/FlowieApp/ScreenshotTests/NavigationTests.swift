import XCTest

final class NavigationTests: XCTestCase {
    @MainActor
    func testSearchKeyboardAndHistory() throws {
        #if !targetEnvironment(simulator)
        throw XCTSkip("Uses a Simulator-only journal fixture")
        #endif
        continueAfterFailure = false
        let app = XCUIApplication()
        let isPad = UIDevice.current.userInterfaceIdiom == .pad
        XCUIDevice.shared.orientation = isPad ? .landscapeLeft : .portrait
        let prefix = "Fieldnotes"
        let suffix = String(UUID().uuidString.prefix(6))
        let marker = prefix + suffix
        app.launchEnvironment["FLOWIE_UI_TEST_ENTRY"] = marker
        app.launchArguments = ["-AppleLanguages", "(en)", "-AppleLocale", "en_US"]
        app.launch()
        if isPad {
            app.buttons["Show Sidebar"].tap()
            app.buttons["Search"].tap()
        } else {
            app.tabBars.buttons["Search"].tap()
        }
        let field = app.searchFields.firstMatch
        XCTAssertTrue(field.waitForExistence(timeout: 8))
        field.tap()
        field.typeText(marker)
        let keyboard = app.keyboards.firstMatch
        let hide = app.buttons["Hide Keyboard"]
        XCTAssertTrue(keyboard.waitForExistence(timeout: 4))
        XCTAssertTrue(hide.waitForExistence(timeout: 4))
        hide.tap()
        XCTAssertTrue(keyboard.waitForNonExistence(timeout: 4))
        XCTAssertEqual(field.value as? String, marker)

        field.tap()
        XCTAssertTrue(keyboard.waitForExistence(timeout: 4))
        let hit = app.buttons.matching(NSPredicate(format: "label CONTAINS %@", marker)).firstMatch
        XCTAssertTrue(hit.waitForExistence(timeout: 4))
        hit.tap()
        XCTAssertTrue(keyboard.waitForNonExistence(timeout: 4))
        let row = app.cells.matching(NSPredicate(format: "label == %@", marker)).firstMatch
        XCTAssertTrue(row.waitForExistence(timeout: 5))

        if isPad {
            XCTAssertLessThan(field.frame.midX, app.frame.midX)
            XCTAssertGreaterThan(row.frame.midX, app.frame.midX)
        }
        app.buttons["Back"].firstMatch.tap()
        XCTAssertTrue(field.waitForExistence(timeout: 4))
        XCTAssertEqual(field.value as? String, marker)
        XCTAssertTrue(hit.isHittable)
        if isPad { XCTAssertGreaterThan(field.frame.midX, app.frame.midX) }
        app.buttons["Forward"].firstMatch.tap()
        XCTAssertTrue(row.waitForExistence(timeout: 5))
        XCTAssertFalse(keyboard.exists)

        // Drag the pages back and forward through the same history as the toolbar buttons.
        let left = app.coordinate(withNormalizedOffset: CGVector(dx: 0.19, dy: 0.55))
        let right = app.coordinate(withNormalizedOffset: CGVector(dx: 0.81, dy: 0.55))
        left.press(forDuration: 0.05, thenDragTo: right, withVelocity: .slow, thenHoldForDuration: 0.1)
        XCTAssertTrue(app.buttons["Forward"].firstMatch.waitForExistence(timeout: 4))
        XCTAssertEqual(field.value as? String, marker)
        right.press(forDuration: 0.05, thenDragTo: left, withVelocity: .slow, thenHoldForDuration: 0.1)
        XCTAssertTrue(app.buttons["Forward"].firstMatch.waitForNonExistence(timeout: 4))
        XCTAssertTrue(row.isHittable)
    }
}
