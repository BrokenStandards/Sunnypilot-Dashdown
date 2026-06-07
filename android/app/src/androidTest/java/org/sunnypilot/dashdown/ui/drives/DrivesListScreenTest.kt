package org.sunnypilot.dashdown.ui.drives

import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onAllNodesWithTag
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.onNodeWithText
import org.junit.Rule
import org.junit.Test
import org.sunnypilot.dashdown.ui.theme.DashdownTheme
import uniffi.dashdown_core.Drive
import uniffi.dashdown_core.SyncStatus

/**
 * Step-4 Compose test for the stateless drives list: rows render with the right sync badges and the
 * empty state shows. The real network sync path is covered by [DrivesSyncLiveTest] (skip-gated) and
 * the Rust core's own integration tests.
 */
class DrivesListScreenTest {
  @get:Rule val rule = createComposeRule()

  private fun drive(key: String, status: SyncStatus, preserved: Boolean = false, segs: UInt = 3u) =
      Drive(
          driveKey = key,
          routeId = key.substringBeforeLast("--"),
          firstSegmentNum = 0u,
          lastSegmentNum = segs - 1u,
          startMs = 1_700_000_000_000L,
          endMs = 1_700_000_180_000L,
          segmentCount = segs,
          recording = false,
          syncState = status,
          preserved = preserved,
          segments = emptyList(),
      )

  @Test
  fun showsDrivesWithBadges() {
    val drives =
        listOf(
            drive("000001a3--c20ba54385--0", SyncStatus.COMPLETE, preserved = true),
            drive("000001a4--aabbccddee--0", SyncStatus.PARTIAL),
            drive("000001a5--1122334455--0", SyncStatus.NOT_DOWNLOADED),
        )
    rule.setContent {
      DashdownTheme {
        DrivesListScreen(
            DrivesUiState(drives = drives, loading = false),
            emptyMap(),
            emptyMap(),
            {},
            {},
            {},
            {},
            {},
            {},
            {},
        )
      }
    }
    rule.onNodeWithTag("drive_row_000001a3--c20ba54385--0").assertExists()
    rule.onNodeWithText("Complete").assertIsDisplayed()
    rule.onNodeWithText("Partial").assertIsDisplayed()
    rule.onNodeWithText("Not downloaded").assertIsDisplayed()
    // Every row carries a thumbnail slot (placeholder until a qcamera frame loads).
    assert(
        rule.onAllNodesWithTag("drive_thumb", useUnmergedTree = true).fetchSemanticsNodes().size ==
            3) {
          "expected one thumbnail slot per drive row"
        }
  }

  @Test
  fun showsEmptyState() {
    rule.setContent {
      DashdownTheme {
        DrivesListScreen(
            DrivesUiState(loading = false), emptyMap(), emptyMap(), {}, {}, {}, {}, {}, {}, {})
      }
    }
    rule.onNodeWithText("No drives yet", substring = true).assertExists()
  }
}
