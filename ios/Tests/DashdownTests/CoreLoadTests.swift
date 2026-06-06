import XCTest

@testable import Dashdown
import UniFFI

/// Binding-load smoke for Apple platforms: proves the generated Swift bindings
/// load the Rust core and calls cross the UniFFI boundary (sync + async).
///
/// NOTE: this links the iOS `.xcframework`, so it runs on a device or macOS —
/// not on a bare Linux host (there is no iOS Simulator on Linux).
final class CoreLoadTests: XCTestCase {
  func testSyncFfi() {
    XCTAssertEqual(ping(), "pong")
    XCTAssertFalse(version().isEmpty)
  }

  func testAsyncFfi() async throws {
    let pong = try await pingAsync()
    XCTAssertEqual(pong, "pong")
  }
}
