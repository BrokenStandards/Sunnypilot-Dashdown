package org.sunnypilot.dashdown.ui.detail

import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.onNodeWithText
import org.junit.Rule
import org.junit.Test
import org.sunnypilot.dashdown.ui.theme.DashdownTheme
import uniffi.dashdown_core.Drive
import uniffi.dashdown_core.DriveSyncStatus
import uniffi.dashdown_core.FileKind
import uniffi.dashdown_core.Segment
import uniffi.dashdown_core.SegmentFile
import uniffi.dashdown_core.SegmentName
import uniffi.dashdown_core.SyncStatus

/**
 * Step-6 Compose test for the stateless drive-detail screen. Renders with `playablePath = null` so
 * the ExoPlayer isn't instantiated; verifies the header, the download + export actions, and the
 * segment/file listing.
 */
class DriveDetailScreenTest {
  @get:Rule val rule = createComposeRule()

  private fun sampleDrive(): Drive {
    val seg =
        Segment(
            name = SegmentName(routeId = "000001a3--c20ba54385", segmentNum = 0u),
            files =
                listOf(
                    SegmentFile(
                        kind = FileKind.Q_CAMERA,
                        name = "qcamera.ts",
                        remoteSize = 1200u,
                        mtimeS = 0L)),
            recording = false,
        )
    return Drive(
        driveKey = "000001a3--c20ba54385--0",
        routeId = "000001a3--c20ba54385",
        firstSegmentNum = 0u,
        lastSegmentNum = 0u,
        startMs = 1_700_000_000_000L,
        endMs = 1_700_000_060_000L,
        segmentCount = 1u,
        recording = false,
        syncState = SyncStatus.NOT_DOWNLOADED,
        preserved = false,
        segments = listOf(seg),
    )
  }

  @Test
  fun rendersHeaderActionsAndFiles() {
    val drive = sampleDrive()
    val status = DriveSyncStatus(drive.driveKey, SyncStatus.NOT_DOWNLOADED, 0u, 1u, 0u, 1200u, null)
    rule.setContent {
      DashdownTheme {
        DriveDetailScreen(
            state =
                DriveDetailUiState(
                    drive = drive, status = status, loading = false, playablePath = null),
            live = null,
            onBack = {},
            onPreserve = {},
            onDownload = {},
            onCancel = {},
            onExport = {},
        )
      }
    }
    rule.onNodeWithText("000001a3--c20ba54385").assertExists()
    rule.onNodeWithTag("drive_detail_download_btn").assertExists()
    rule.onNodeWithTag("drive_detail_export_btn").assertExists()
    rule.onNodeWithText("qcamera.ts", substring = true).assertExists()
  }
}
