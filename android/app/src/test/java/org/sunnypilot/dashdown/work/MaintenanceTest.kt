package org.sunnypilot.dashdown.work

import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test
import uniffi.dashdown_core.RetentionStatus

/** Pure decision logic for the low-headroom storage warning. */
class MaintenanceTest {
  private fun status(local: Long, preserved: Long, budget: Long?) =
      RetentionStatus(localMinutes = local, preservedMinutes = preserved, budgetMinutes = budget)

  @Test
  fun noBudgetNeverWarns() {
    assertFalse(Maintenance.shouldWarn(status(10_000, 0, null), 10))
  }

  @Test
  fun warnsWhenNonPreservedFootageNearsBudget() {
    // budget 100, 95 non-preserved local → headroom 5 < 10 → warn.
    assertTrue(Maintenance.shouldWarn(status(95, 0, 100), 10))
  }

  @Test
  fun warnsAtSteadyStateCap() {
    // non-preserved local == budget → headroom 0 < 10 → warn.
    assertTrue(Maintenance.shouldWarn(status(100, 0, 100), 10))
  }

  @Test
  fun noWarnWithPlentyHeadroom() {
    // budget 100, 50 local → headroom 50 ≥ 10 → no warn.
    assertFalse(Maintenance.shouldWarn(status(50, 0, 100), 10))
  }

  @Test
  fun preservedFootageDoesNotCountTowardBudget() {
    // 100 local but 60 preserved → non-preserved 40, budget 100 → headroom 60 → no warn.
    assertFalse(Maintenance.shouldWarn(status(100, 60, 100), 10))
  }
}
