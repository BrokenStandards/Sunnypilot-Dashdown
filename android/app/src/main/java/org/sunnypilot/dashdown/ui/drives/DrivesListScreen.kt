@file:OptIn(ExperimentalMaterial3Api::class)

package org.sunnypilot.dashdown.ui.drives

import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material.icons.filled.CheckCircle
import androidx.compose.material.icons.filled.Star
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.LinearProgressIndicator
import androidx.compose.material3.ListItem
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.TopAppBar
import androidx.compose.material3.pulltorefresh.PullToRefreshBox
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.LifecycleResumeEffect
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import androidx.lifecycle.viewmodel.compose.viewModel
import androidx.lifecycle.viewmodel.initializer
import androidx.lifecycle.viewmodel.viewModelFactory
import coil3.compose.AsyncImage
import coil3.request.ImageRequest
import coil3.request.crossfade
import coil3.video.videoFrameMillis
import java.io.File
import java.time.Instant
import java.time.ZoneId
import java.time.format.DateTimeFormatter
import org.sunnypilot.dashdown.data.DriveProgress
import org.sunnypilot.dashdown.service.DownloadService
import org.sunnypilot.dashdown.ui.rememberRepository
import uniffi.dashdown_core.Drive
import uniffi.dashdown_core.SyncStatus

@Composable
fun DrivesListRoute(deviceId: Long, onDriveClick: (String) -> Unit, onBack: () -> Unit) {
  val repo = rememberRepository()
  val vm: DrivesListViewModel =
      viewModel(factory = viewModelFactory { initializer { DrivesListViewModel(repo, deviceId) } })
  val state by vm.state.collectAsStateWithLifecycle()
  val progress by vm.progress.collectAsStateWithLifecycle()
  val thumbnails by vm.thumbnails.collectAsStateWithLifecycle()
  val context = LocalContext.current
  // Re-run the cheap offline reclassify on resume (e.g. returning from a download).
  LifecycleResumeEffect(Unit) {
    vm.loadOffline()
    onPauseOrDispose {}
  }
  DrivesListScreen(
      state = state,
      progress = progress,
      thumbnails = thumbnails,
      onRequestThumbnail = vm::requestThumbnail,
      onRefresh = vm::refreshOnline,
      onPreserve = vm::togglePreserve,
      onDownload = { d -> DownloadService.start(context, deviceId, d.driveKey) },
      onCancel = { d -> DownloadService.cancel(context, d.driveKey) },
      onDriveClick = onDriveClick,
      onBack = onBack,
  )
}

@Composable
fun DrivesListScreen(
    state: DrivesUiState,
    progress: Map<String, DriveProgress>,
    thumbnails: Map<String, String?>,
    onRequestThumbnail: (Drive) -> Unit,
    onRefresh: () -> Unit,
    onPreserve: (Drive) -> Unit,
    onDownload: (Drive) -> Unit,
    onCancel: (Drive) -> Unit,
    onDriveClick: (String) -> Unit,
    onBack: () -> Unit,
) {
  Scaffold(
      topBar = {
        TopAppBar(
            title = { Text("Drives") },
            navigationIcon = {
              IconButton(onClick = onBack) {
                Icon(Icons.AutoMirrored.Filled.ArrowBack, contentDescription = "Back")
              }
            },
        )
      }) { padding ->
        PullToRefreshBox(
            isRefreshing = state.refreshing,
            onRefresh = onRefresh,
            modifier = Modifier.fillMaxSize().padding(padding).testTag("drives_pull_refresh"),
        ) {
          if (state.loading && state.drives.isEmpty()) {
            Box(Modifier.fillMaxSize()) {
              CircularProgressIndicator(Modifier.align(Alignment.Center))
            }
          } else {
            // Always a LazyColumn (even when empty) so the pull-to-refresh nested-scroll gesture
            // registers; the empty message is a full-viewport item.
            LazyColumn(Modifier.fillMaxSize()) {
              if (state.drives.isEmpty()) {
                item {
                  Box(Modifier.fillParentMaxSize(), contentAlignment = Alignment.Center) {
                    Text(
                        "No drives yet. Pull down to sync from the device.",
                        modifier = Modifier.padding(24.dp),
                        style = MaterialTheme.typography.bodyLarge,
                    )
                  }
                }
              } else {
                items(state.drives, key = { it.driveKey }) { drive ->
                  DriveRow(
                      drive = drive,
                      live = progress[drive.driveKey],
                      thumbPath = thumbnails[drive.driveKey],
                      onRequestThumbnail = { onRequestThumbnail(drive) },
                      onClick = { onDriveClick(drive.driveKey) },
                      onPreserve = { onPreserve(drive) },
                      onDownload = { onDownload(drive) },
                      onCancel = { onCancel(drive) },
                  )
                  HorizontalDivider()
                }
              }
            }
          }
          state.error?.let { err ->
            Text(
                "Error: $err",
                color = MaterialTheme.colorScheme.error,
                modifier = Modifier.align(Alignment.BottomCenter).padding(16.dp),
            )
          }
        }
      }
}

@Composable
private fun DriveRow(
    drive: Drive,
    live: DriveProgress?,
    thumbPath: String?,
    onRequestThumbnail: () -> Unit,
    onClick: () -> Unit,
    onPreserve: () -> Unit,
    onDownload: () -> Unit,
    onCancel: () -> Unit,
) {
  // Resolve this drive's thumbnail once it scrolls into view (only visible rows hit the core).
  LaunchedEffect(drive.driveKey) { onRequestThumbnail() }
  val downloading = live != null && live.terminal == null
  val status = if (downloading) SyncStatus.DOWNLOADING else drive.syncState
  ListItem(
      leadingContent = { DriveThumbnail(thumbPath) },
      headlineContent = { Text(driveTitle(drive)) },
      supportingContent = {
        Column {
          Row(verticalAlignment = Alignment.CenterVertically) {
            Text(driveSubtitle(drive))
            Spacer(Modifier.width(8.dp))
            SyncBadge(status)
          }
          if (downloading && live != null && live.bytesTotal > 0) {
            LinearProgressIndicator(
                progress = { (live.bytesDone.toFloat() / live.bytesTotal).coerceIn(0f, 1f) },
                modifier = Modifier.fillMaxWidth().testTag("drive_progress"),
            )
          }
        }
      },
      trailingContent = {
        Row(verticalAlignment = Alignment.CenterVertically) {
          IconButton(
              onClick = onPreserve,
              modifier = Modifier.testTag("drive_preserve_${drive.driveKey}"),
          ) {
            Icon(
                Icons.Filled.Star,
                contentDescription = if (drive.preserved) "preserve_on" else "preserve_off",
                tint =
                    if (drive.preserved) MaterialTheme.colorScheme.primary
                    else MaterialTheme.colorScheme.onSurfaceVariant.copy(alpha = 0.35f),
            )
          }
          DriveAction(drive.driveKey, status, onDownload, onCancel)
        }
      },
      modifier = Modifier.clickable(onClick = onClick).testTag("drive_row_${drive.driveKey}"),
  )
}

/**
 * A 16:9-ish drive thumbnail: a frame decoded straight from the first mirrored `qcamera.ts` via
 * Coil's [coil3.video.VideoFrameDecoder] when [path] is non-null, else a neutral placeholder block.
 */
@Composable
private fun DriveThumbnail(path: String?) {
  Box(
      Modifier.size(width = 72.dp, height = 40.dp)
          .clip(RoundedCornerShape(6.dp))
          .background(MaterialTheme.colorScheme.surfaceVariant)
          .testTag("drive_thumb"),
      contentAlignment = Alignment.Center,
  ) {
    if (path != null) {
      AsyncImage(
          model =
              ImageRequest.Builder(LocalContext.current)
                  .data(File(path))
                  .videoFrameMillis(0L)
                  .crossfade(true)
                  .build(),
          contentDescription = "drive thumbnail",
          contentScale = ContentScale.Crop,
          modifier = Modifier.fillMaxSize(),
      )
    }
  }
}

/** Per-row action: Download/Resume to start, Cancel while downloading, a check when complete. */
@Composable
private fun DriveAction(
    driveKey: String,
    status: SyncStatus,
    onDownload: () -> Unit,
    onCancel: () -> Unit,
) {
  when (status) {
    SyncStatus.DOWNLOADING ->
        TextButton(onClick = onCancel, modifier = Modifier.testTag("drive_cancel_$driveKey")) {
          Text("Cancel")
        }
    SyncStatus.COMPLETE ->
        Icon(
            Icons.Filled.CheckCircle,
            contentDescription = "complete",
            tint = Color(0xFF2E7D32),
        )
    SyncStatus.PARTIAL,
    SyncStatus.FAILED ->
        TextButton(onClick = onDownload, modifier = Modifier.testTag("drive_download_$driveKey")) {
          Text("Resume")
        }
    SyncStatus.NOT_DOWNLOADED ->
        TextButton(onClick = onDownload, modifier = Modifier.testTag("drive_download_$driveKey")) {
          Text("Download")
        }
  }
}

@Composable
private fun SyncBadge(status: SyncStatus) {
  val (label, color) =
      when (status) {
        SyncStatus.COMPLETE -> "Complete" to Color(0xFF2E7D32)
        SyncStatus.PARTIAL -> "Partial" to Color(0xFFF9A825)
        SyncStatus.DOWNLOADING -> "Downloading" to Color(0xFF1565C0)
        SyncStatus.FAILED -> "Failed" to Color(0xFFC62828)
        SyncStatus.NOT_DOWNLOADED -> "Not downloaded" to Color(0xFF9E9E9E)
      }
  Surface(
      color = color.copy(alpha = 0.15f),
      contentColor = color,
      shape = RoundedCornerShape(8.dp),
      modifier =
          Modifier.testTag("drive_sync_badge").semantics { contentDescription = badgeDesc(status) },
  ) {
    Text(
        label,
        modifier = Modifier.padding(horizontal = 8.dp, vertical = 4.dp),
        style = MaterialTheme.typography.labelMedium,
    )
  }
}

private fun badgeDesc(status: SyncStatus): String =
    "sync_" +
        when (status) {
          SyncStatus.COMPLETE -> "complete"
          SyncStatus.PARTIAL -> "partial"
          SyncStatus.DOWNLOADING -> "downloading"
          SyncStatus.FAILED -> "failed"
          SyncStatus.NOT_DOWNLOADED -> "not_downloaded"
        }

private val TITLE_FORMAT = DateTimeFormatter.ofPattern("MMM d, h:mm a")

private fun driveTitle(d: Drive): String {
  val start = d.startMs ?: return d.routeId
  return Instant.ofEpochMilli(start).atZone(ZoneId.systemDefault()).format(TITLE_FORMAT)
}

private fun driveSubtitle(d: Drive): String {
  val segs = "${d.segmentCount} seg"
  val s = d.startMs
  val e = d.endMs
  if (s == null || e == null || e <= s) return segs
  val totalSec = (e - s) / 1000
  val mins = totalSec / 60
  val secs = totalSec % 60
  val dur = if (mins > 0) "${mins}m ${secs}s" else "${secs}s"
  val recording = if (d.recording) " · recording" else ""
  return "$dur · $segs$recording"
}
