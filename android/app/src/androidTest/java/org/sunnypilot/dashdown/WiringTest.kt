package org.sunnypilot.dashdown

import androidx.test.core.app.ApplicationProvider
import androidx.test.ext.junit.runners.AndroidJUnit4
import kotlinx.coroutines.runBlocking
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertSame
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith

/**
 * Step-1 wiring smoke. Proves the DI graph builds the real cross-compiled `AppCore` through the
 * repository on-device, that the locator singletons are stable, and that the progress bus starts
 * empty — elevating the B0 binding-load smoke into the :app module.
 */
@RunWith(AndroidJUnit4::class)
class WiringTest {
  private val app: DashdownApp
    get() = ApplicationProvider.getApplicationContext()

  @Test
  fun locatorSingletonsAreStable() {
    val locator = app.locator
    assertSame(locator, app.locator)
    assertSame(locator.repository, app.locator.repository)
    assertSame(locator.progressBus, app.locator.progressBus)
  }

  @Test
  fun coreReachableThroughRepository() = runBlocking {
    // Builds the real AppCore on first call and exercises a suspend FFI round-trip.
    val devices = app.locator.repository.listDevices()
    assertNotNull(devices)
    assertTrue(app.locator.progressBus.states.value.isEmpty())
  }
}
