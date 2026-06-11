package org.sunnypilot.dashdown.work

import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test
import uniffi.dashdown_core.RetentionStatus

/**
 * Pure decision logic for the low-headroom storage warning: threshold, toggle, and re-notify gate.
 */
class MaintenanceTest {
  private fun status(local: Long, preserved: Long, budget: Long?) =
      RetentionStatus(localMinutes = local, preservedMinutes = preserved, budgetMinutes = budget)

  private val DAY = Maintenance.MIN_NOTIFY_INTERVAL_MS

  @Test
  fun noBudgetNeverWarns() {
    assertFalse(Maintenance.shouldWarn(status(10_000, 0, null), enabled = true, threshold = 10))
  }

  @Test
  fun warnsWhenNonPreservedFootageNearsBudget() {
    // budget 100, 95 non-preserved local → headroom 5 < 10 → warn.
    assertTrue(Maintenance.shouldWarn(status(95, 0, 100), enabled = true, threshold = 10))
  }

  @Test
  fun warnsAtSteadyStateCap() {
    // non-preserved local == budget → headroom 0 < 10 → warn.
    assertTrue(Maintenance.shouldWarn(status(100, 0, 100), enabled = true, threshold = 10))
  }

  @Test
  fun noWarnWithPlentyHeadroom() {
    // budget 100, 50 local → headroom 50 ≥ 10 → no warn.
    assertFalse(Maintenance.shouldWarn(status(50, 0, 100), enabled = true, threshold = 10))
  }

  @Test
  fun preservedFootageDoesNotCountTowardBudget() {
    // 100 local but 60 preserved → non-preserved 40, budget 100 → headroom 60 → no warn.
    assertFalse(Maintenance.shouldWarn(status(100, 60, 100), enabled = true, threshold = 10))
  }

  @Test
  fun disabledNeverWarns() {
    // Even at the cap, a disabled toggle suppresses the warning entirely.
    assertFalse(Maintenance.shouldWarn(status(100, 0, 100), enabled = false, threshold = 10))
  }

  @Test
  fun customThresholdChangesBoundary() {
    // budget 100, 80 local → headroom 20. Threshold 10 → no warn; threshold 30 → warn.
    assertFalse(Maintenance.shouldWarn(status(80, 0, 100), enabled = true, threshold = 10))
    assertTrue(Maintenance.shouldWarn(status(80, 0, 100), enabled = true, threshold = 30))
  }

  @Test
  fun notDueWithinInterval() {
    val now = 10 * DAY
    assertFalse(Maintenance.dueForNotification(lastMs = now - (DAY - 1), nowMs = now))
  }

  @Test
  fun dueAtOrAfterInterval() {
    val now = 10 * DAY
    assertTrue(Maintenance.dueForNotification(lastMs = now - DAY, nowMs = now))
    assertTrue(Maintenance.dueForNotification(lastMs = now - 2 * DAY, nowMs = now))
  }

  @Test
  fun dueWhenNeverNotified() {
    // last == 0 (no prior alert) is always due.
    assertTrue(Maintenance.dueForNotification(lastMs = 0L, nowMs = DAY))
  }
}
