package org.sunnypilot.dashdown.core

import kotlinx.coroutines.runBlocking
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test
import uniffi.dashdown_core.ping
import uniffi.dashdown_core.pingAsync
import uniffi.dashdown_core.version

/**
 * On-device binding-load smoke (B0): proves the real cross-compiled `.so` loads via JNA and that we
 * can call across the UniFFI boundary — sync and async.
 *
 * Runs under `connectedDebugAndroidTest` on the emulator/device, so it exercises the full Gradle →
 * cargo-ndk → bindgen → JNA pipeline, not just generated source.
 */
class CoreLoadTest {
  @Test
  fun syncFfiWorks() {
    assertEquals("pong", ping())
    assertTrue("version() should be non-empty", version().isNotEmpty())
  }

  @Test fun asyncFfiWorks() = runBlocking { assertEquals("pong", pingAsync()) }
}
