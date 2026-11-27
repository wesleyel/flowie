// Not a test: scripts/ios-screenshots.py runs it to turn the iPad Simulator to landscape before
// the iPad screenshots. simctl can't turn a Simulator, and an iPad app can't choose its own
// orientation; XCTest can turn the device.
import XCTest

final class Landscape: XCTestCase {
    func testTurnToLandscape() {
        XCUIDevice.shared.orientation = .landscapeLeft
    }
}
