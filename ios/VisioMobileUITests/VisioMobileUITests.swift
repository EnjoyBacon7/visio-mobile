import XCTest

final class VisioMobileUITests: XCTestCase {

    private var app: XCUIApplication!

    override func setUp() {
        super.setUp()
        continueAfterFailure = false
        app = XCUIApplication()
    }

    /// Test: launch via visio-test:// deep link, verify call view appears.
    func testDeepLinkAutoConnect() throws {
        app.launch()

        // Give the app a moment to load
        sleep(2)

        // Open the test deep link (URL set by environment variable)
        let livekitUrl = ProcessInfo.processInfo.environment["LIVEKIT_URL"] ?? "ws://localhost:7880"
        let token = ProcessInfo.processInfo.environment["LIVEKIT_TOKEN"] ?? ""

        guard !token.isEmpty else {
            XCTFail("LIVEKIT_TOKEN environment variable not set")
            return
        }

        let encodedUrl = livekitUrl.addingPercentEncoding(withAllowedCharacters: .urlQueryAllowed)!
        let encodedToken = token.addingPercentEncoding(withAllowedCharacters: .urlQueryAllowed)!
        let deepLink = URL(string: "visio-test://connect?livekit_url=\(encodedUrl)&token=\(encodedToken)")!

        // Open deep link
        XCUIDevice.shared.system.open(deepLink)
        sleep(5)

        // Verify we're in a call (hangup button should appear)
        let hangupButton = app.buttons["hangup"]
        XCTAssertTrue(hangupButton.waitForExistence(timeout: 10), "Hangup button should appear after auto-connect")

        // Wait for test duration
        let duration = Int(ProcessInfo.processInfo.environment["TEST_DURATION"] ?? "60") ?? 60
        sleep(UInt32(duration))

        // Verify still connected (hangup button still exists)
        XCTAssertTrue(hangupButton.exists, "Should still be in call after test duration")
    }
}
