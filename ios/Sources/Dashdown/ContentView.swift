import SwiftUI
import UniFFI

/// B0 binding-load smoke UI: `version()` + `ping()` exercise the sync FFI path;
/// `pingAsync()` exercises the async (`async throws`) path over the Rust core.
struct ContentView: View {
  @State private var async = "…"

  var body: some View {
    VStack(alignment: .leading, spacing: 8) {
      Text("dashdown core \(version())")
      Text("sync: \(ping())")
      Text("async: \(async)")
    }
    .padding(24)
    .task {
      async = await pingAsync()
    }
  }
}
