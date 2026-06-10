@file:OptIn(ExperimentalMaterial3Api::class)

package org.sunnypilot.dashdown.ui.detail

import android.net.Uri
import android.widget.Toast
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material.icons.filled.Star
import androidx.compose.material3.Button
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.LinearProgressIndicator
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Text
import androidx.compose.material3.TopAppBar
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.LifecycleResumeEffect
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import androidx.lifecycle.viewmodel.compose.viewModel
import androidx.lifecycle.viewmodel.initializer
import androidx.lifecycle.viewmodel.viewModelFactory
import java.io.File
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import org.sunnypilot.dashdown.data.DashdownRepository
import org.sunnypilot.dashdown.data.DriveProgress
import org.sunnypilot.dashdown.service.DownloadService
import org.sunnypilot.dashdown.ui.rememberRepository
import uniffi.dashdown_core.FileKind
import uniffi.dashdown_core.Segment
import uniffi.dashdown_core.SyncStatus

@Composable
fun DriveDetailRoute(deviceId: Long, driveKey: String, onBack: () -> Unit) {
  val repo = rememberRepository()
  val vm: DriveDetailViewModel =
      viewModel(
          factory =
              viewModelFactory { initializer { DriveDetailViewModel(repo, deviceId, driveKey) } })
  val state by vm.state.collectAsStateWithLifecycle()
  val progress by vm.progress.collectAsStateWithLifecycle()
  val context = LocalContext.current
  val scope = rememberCoroutineScope()
  LifecycleResumeEffect(Unit) {
    vm.load()
    onPauseOrDispose {}
  }
  val exportLauncher =
      rememberLauncherForActivityResult(
          ActivityResultContracts.CreateDocument("application/zip")) { uri ->
            if (uri != null) {
              scope.launch {
                val ok = exportZip(context, repo, deviceId, driveKey, uri)
                Toast.makeText(
                        context,
                        if (ok) "Exported drive" else "Export failed",
                        Toast.LENGTH_SHORT,
                    )
                    .show()
              }
            }
          }
  DriveDetailScreen(
      state = state,
      live = progress[driveKey],
      onBack = onBack,
      onPreserve = vm::togglePreserve,
      onDownload = { DownloadService.start(context, deviceId, driveKey) },
      onCancel = { DownloadService.cancel(context, driveKey) },
      onExport = { exportLauncher.launch("${state.drive?.routeId ?: "drive"}.zip") },
      resolveHd = vm::ensurePlayable,
  )
}

@Composable
fun DriveDetailScreen(
    state: DriveDetailUiState,
    live: DriveProgress?,
    onBack: () -> Unit,
    onPreserve: () -> Unit,
    onDownload: () -> Unit,
    onCancel: () -> Unit,
    onExport: () -> Unit,
    resolveHd: suspend (FileKind, UInt) -> String? = { _, _ -> null },
) {
  val drive = state.drive
  val downloading = live != null && live.terminal == null
  val status =
      if (downloading) SyncStatus.DOWNLOADING else drive?.syncState ?: SyncStatus.NOT_DOWNLOADED
  Scaffold(
      topBar = {
        TopAppBar(
            title = { Text("Drive") },
            navigationIcon = {
              IconButton(onClick = onBack) {
                Icon(Icons.AutoMirrored.Filled.ArrowBack, contentDescription = "Back")
              }
            },
            actions = {
              if (drive != null) {
                IconButton(
                    onClick = onPreserve, modifier = Modifier.testTag("drive_detail_preserve")) {
                      Icon(
                          Icons.Filled.Star,
                          contentDescription =
                              if (drive.preserved) "preserve_on" else "preserve_off",
                          tint =
                              if (drive.preserved) MaterialTheme.colorScheme.primary
                              else MaterialTheme.colorScheme.onSurfaceVariant,
                      )
                    }
              }
            },
        )
      }) { padding ->
        when {
          state.loading && drive == null ->
              CircularProgressIndicator(Modifier.padding(padding).padding(24.dp))
          drive == null ->
              Text("Error: ${state.error ?: "not found"}", Modifier.padding(padding).padding(24.dp))
          else ->
              Column(
                  Modifier.fillMaxSize()
                      .padding(padding)
                      .padding(16.dp)
                      .verticalScroll(rememberScrollState()),
                  verticalArrangement = Arrangement.spacedBy(12.dp),
              ) {
                Text(drive.routeId, style = MaterialTheme.typography.titleMedium)
                val st = state.status
                if (st != null) {
                  Text("${st.filesDone}/${st.filesTotal} files · ${status.name.lowercase()}")
                  st.error?.let { Text("Error: $it", color = MaterialTheme.colorScheme.error) }
                }
                if (downloading && live != null && live.bytesTotal > 0) {
                  LinearProgressIndicator(
                      progress = { (live.bytesDone.toFloat() / live.bytesTotal).coerceIn(0f, 1f) },
                      modifier = Modifier.fillMaxWidth().testTag("drive_progress"),
                  )
                }

                if (state.playablePaths.isNotEmpty() || state.hdCameras.isNotEmpty()) {
                  MultiCamPlayer(
                      qcameraPaths = state.playablePaths,
                      hdCameras = state.hdCameras,
                      resolveHd = resolveHd,
                      modifier = Modifier.fillMaxWidth(),
                  )
                }

                Row(horizontalArrangement = Arrangement.spacedBy(12.dp)) {
                  when (status) {
                    SyncStatus.DOWNLOADING ->
                        Button(
                            onClick = onCancel,
                            modifier = Modifier.testTag("drive_detail_cancel_btn")) {
                              Text("Cancel")
                            }
                    SyncStatus.COMPLETE -> {}
                    else ->
                        Button(
                            onClick = onDownload,
                            modifier = Modifier.testTag("drive_detail_download_btn"),
                        ) {
                          Text(if (status == SyncStatus.NOT_DOWNLOADED) "Download" else "Resume")
                        }
                  }
                  OutlinedButton(
                      onClick = onExport, modifier = Modifier.testTag("drive_detail_export_btn")) {
                        Text("Export as zip")
                      }
                }

                HorizontalDivider()
                drive.segments.forEach { seg -> SegmentBlock(seg) }
              }
        }
      }
}

@Composable
private fun SegmentBlock(seg: Segment) {
  Column(Modifier.fillMaxWidth().padding(vertical = 4.dp)) {
    Text(
        "Segment ${seg.name.segmentNum}${if (seg.recording) " · recording" else ""}",
        style = MaterialTheme.typography.labelLarge,
    )
    seg.files.forEach { f ->
      Text(
          "  ${f.name} · ${formatBytes(f.remoteSize.toLong())}",
          style = MaterialTheme.typography.bodySmall,
      )
    }
  }
}

/** Core writes the zip to a temp file; we stream it to the user-chosen SAF document. */
private suspend fun exportZip(
    context: android.content.Context,
    repo: DashdownRepository,
    deviceId: Long,
    driveKey: String,
    uri: Uri,
): Boolean =
    withContext(Dispatchers.IO) {
      val temp = File.createTempFile("export", ".zip", context.cacheDir)
      try {
        repo.exportDriveZip(deviceId, driveKey, temp.absolutePath)
        context.contentResolver.openOutputStream(uri)?.use { out ->
          temp.inputStream().use { it.copyTo(out) }
        } ?: return@withContext false
        true
      } catch (t: Throwable) {
        false
      } finally {
        temp.delete()
      }
    }

private fun formatBytes(bytes: Long): String {
  if (bytes < 1024) return "$bytes B"
  val kb = bytes / 1024.0
  if (kb < 1024) return "%.1f KB".format(kb)
  val mb = kb / 1024.0
  if (mb < 1024) return "%.1f MB".format(mb)
  return "%.1f GB".format(mb / 1024.0)
}
