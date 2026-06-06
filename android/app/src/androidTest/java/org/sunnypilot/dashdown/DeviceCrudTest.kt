package org.sunnypilot.dashdown

import androidx.test.core.app.ApplicationProvider
import androidx.test.ext.junit.runners.AndroidJUnit4
import kotlinx.coroutines.runBlocking
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotEquals
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith
import uniffi.dashdown_core.ConnMode
import uniffi.dashdown_core.Device
import uniffi.dashdown_core.DeviceSettings
import uniffi.dashdown_core.FileSelection

/**
 * Step-3 integration test: add → list → update → settings round-trip → remove through the real
 * repository + core + SQLite on-device. These are pure DB operations (no network), so no mock
 * server is needed. The test cleans up after itself so the device's index returns to its prior
 * state.
 */
@RunWith(AndroidJUnit4::class)
class DeviceCrudTest {
  private val repo
    get() = ApplicationProvider.getApplicationContext<DashdownApp>().locator.repository

  private fun newDevice(name: String) =
      Device(
          id = 0,
          name = name,
          dongleLabel = "dongleabc",
          hotspotIp = "192.168.43.1",
          wifiIp = null,
          port = 3923.toUShort(),
          activeMode = ConnMode.HOTSPOT,
          password = null,
          autoSync = false,
          fileSelection = FileSelection(false, false, false, true, false, false, false, false),
          retentionMaxMinutes = null,
          autoDeleteFromComma = false,
          autoDeleteMinAgeMin = 60,
      )

  @Test
  fun addUpdateSettingsRemoveRoundTrip() = runBlocking {
    val name = "CrudTest-${System.nanoTime()}"
    val added = repo.addDevice(newDevice(name))
    try {
      assertNotEquals("addDevice should assign a non-zero id", 0L, added.id)
      assertTrue(repo.listDevices().any { it.id == added.id && it.name == name })

      // Update identity/connection fields.
      repo.updateDevice(added.copy(name = "$name-renamed", hotspotIp = "10.0.0.5"))
      val reloaded = repo.listDevices().first { it.id == added.id }
      assertEquals("$name-renamed", reloaded.name)
      assertEquals("10.0.0.5", reloaded.hotspotIp)

      // Settings round-trip via get/setSettings.
      val newSettings =
          DeviceSettings(
              autoSync = true,
              fileSelection = FileSelection(true, false, false, true, false, false, false, false),
              retentionMaxMinutes = 120L,
              autoDeleteFromComma = true,
              autoDeleteMinAgeMin = 30L,
          )
      repo.setSettings(added.id, newSettings)
      val got = repo.getSettings(added.id)
      assertEquals(true, got.autoSync)
      assertEquals(120L, got.retentionMaxMinutes)
      assertEquals(true, got.autoDeleteFromComma)
      assertEquals(30L, got.autoDeleteMinAgeMin)
      assertEquals(true, got.fileSelection.fcamera)
      assertEquals(true, got.fileSelection.qcamera)
      assertEquals(false, got.fileSelection.rlog)
    } finally {
      repo.removeDevice(added.id)
      assertTrue(repo.listDevices().none { it.id == added.id })
    }
  }
}
