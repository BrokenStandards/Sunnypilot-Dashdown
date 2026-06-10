package org.sunnypilot.dashdown

import androidx.test.core.app.ApplicationProvider
import androidx.test.platform.app.InstrumentationRegistry
import kotlinx.coroutines.delay
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.withTimeout
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Assume.assumeTrue
import org.junit.Test
import org.sunnypilot.dashdown.ui.drives.DrivesListViewModel

/**
 * Phase C correctness guard: a silent poll tick refreshes the drive list **without** flipping
 * either spinner flag — so a periodic refresh never flashes the pull-to-refresh or initial-load
 * spinner. VM-level (no Compose) for a deterministic assertion. Skipped unless `mockPort` is
 * supplied.
 */
class DrivesSilentRefreshTest {
  private val app
    get() = ApplicationProvider.getApplicationContext<DashdownApp>()

  private val repo
    get() = app.locator.repository

  @Test
  fun silentRefreshDoesNotFlipSpinner() = runBlocking {
    val port = InstrumentationRegistry.getArguments().getString("mockPort")
    assumeTrue("requires mockPort + fixture + adb reverse", port != null)
    val device = repo.addDevice(probeDevice("SilentVM-${System.nanoTime()}", port!!.toUShort()))
    try {
      val vm = DrivesListViewModel(repo, device.id)
      // Let init settle: offline (empty) → auto online sync populates the fixture drive.
      withTimeout(20_000) { while (vm.state.value.drives.isEmpty()) delay(100) }

      vm.silentRefresh()
      delay(2_000) // allow the silent tick's network sync to complete

      val s = vm.state.value
      assertFalse("silent refresh must not show the pull-to-refresh spinner", s.refreshing)
      assertFalse("silent refresh must not show the initial-load spinner", s.loading)
      assertTrue("silent refresh should keep the drive list populated", s.drives.isNotEmpty())
    } finally {
      repo.removeDevice(device.id)
    }
  }
}
