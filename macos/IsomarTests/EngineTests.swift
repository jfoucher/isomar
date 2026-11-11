import XCTest

/// A smoke test of the engine through its C interface: it starts, draws pixels, answers JSON and
/// takes settings. It runs in its own copy of the engine, with a throwaway data folder.
final class EngineTests: XCTestCase {
    static var started = false

    override class func setUp() {
        let dir = NSTemporaryDirectory() + "isomar-tests-\(getpid())"
        try? FileManager.default.createDirectory(atPath: dir, withIntermediateDirectories: true)
        setenv("ISOMAR_STATE_DIR", dir, 1)
        setenv("ISOMAR_CONFIG_DIR", dir, 1)
        setenv("ISOMAR_OFFLINE", "1", 1)
        setenv("ISOMAR_NO_SAVE", "1", 1)
        setenv("ISOMAR_DEBUG_VIEW", "48.38,-4.49,10", 1)
        started = isomar_start(nil, { _, _ in }, { _ in }, { _, _ in }, { _, _, _ in })
    }

    private func call(_ name: String, _ json: String = "{}") -> [String: Any]? {
        guard let p = isomar_call(name, json) else { return nil }
        return (try? JSONSerialization.jsonObject(with: Data(String(cString: p).utf8))) as? [String: Any]
    }

    func testTheEngineStartsOnce() {
        XCTAssertTrue(Self.started)
        XCTAssertFalse(isomar_start(nil, { _, _ in }, { _ in }, { _, _ in }, { _, _, _ in }), "a second start is refused")
    }

    func testItDrawsPixels() {
        isomar_resize(400, 300)
        let w = 800, h = 600, stride = w * 4
        var buffer = [UInt8](repeating: 0, count: stride * h)
        buffer.withUnsafeMutableBufferPointer { isomar_render($0.baseAddress, Int32(w), Int32(h), Int32(stride), 2.0, false) }
        let painted = buffer.enumerated().filter { $0.offset % 4 == 3 && $0.element > 0 }.count
        XCTAssertGreaterThan(painted, w * h / 2, "the sea and the grid fill the chart")
    }

    func testPagesAnswerJSON() {
        XCTAssertEqual((call("passage")?["routes"] as? [Any])?.count, 0)
        XCTAssertEqual(call("footer")?["planning_only"] as? Bool, true)
        XCTAssertEqual(call("window")?["link"] as? String, "none")
        XCTAssertNotNil(call("nonsense")?["error"])
    }

    func testSettingsRoundTrip() {
        XCTAssertNil(call("settings_patch", "{\"chart_style\":\"ecdis\"}")?["error"])
        XCTAssertEqual(call("settings_get")?["chart_style"] as? String, "ecdis")
        XCTAssertNotNil(call("settings_patch", "{\"safety_depth\":\"deep\"}")?["error"], "a bad value is refused")
    }

    func testTokensFollowTheTheme() {
        IsomarTheme.current = .day
        let day = NSColor.isomar(.windowBg).usingColorSpace(.sRGB)!
        IsomarTheme.current = .night
        let night = NSColor.isomar(.windowBg).usingColorSpace(.sRGB)!
        XCTAssertGreaterThan(day.brightnessComponent, night.brightnessComponent)
        XCTAssertEqual(Tok.byName("alt-3"), .alt3)
    }
}
