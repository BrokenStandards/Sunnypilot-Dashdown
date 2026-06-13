@file:OptIn(ExperimentalMaterial3Api::class)

package org.sunnypilot.dashdown.ui.detail

import android.app.Activity
import android.net.Uri
import android.widget.Toast
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.animation.AnimatedVisibility
import androidx.compose.animation.fadeIn
import androidx.compose.animation.fadeOut
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.statusBarsPadding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material.icons.filled.Star
import androidx.compose.material3.Button
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.LinearProgressIndicator
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.TopAppBar
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalView
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.unit.dp
import androidx.core.view.WindowCompat
import androidx.core.view.WindowInsetsCompat
import androidx.core.view.WindowInsetsControllerCompat
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
  val hasVideo = state.qcamera.isNotEmpty() || state.hdCameras.isNotEmpty()
  when {
    state.loading && drive == null ->
        Box(Modifier.fillMaxSize(), contentAlignment = Alignment.Center) {
          CircularProgressIndicator()
        }
    drive == null ->
        Box(Modifier.fillMaxSize(), contentAlignment = Alignment.Center) {
          Text("Error: ${state.error ?: "not found"}", Modifier.padding(24.dp))
        }
    // Something is playable → go full-bleed immersive (the player fills the screen, chrome
    // overlays).
    hasVideo ->
        ImmersivePlayer(
            state = state,
            status = status,
            downloading = downloading,
            live = live,
            resolveHd = resolveHd,
            onBack = onBack,
            onPreserve = onPreserve,
            onDownload = onDownload,
            onCancel = onCancel,
            onExport = onExport,
        )
    // Nothing downloaded yet → the normal info screen with Download/Export (no segment list).
    else ->
        DriveInfoScreen(
            state = state,
            status = status,
            downloading = downloading,
            live = live,
            onBack = onBack,
            onPreserve = onPreserve,
            onDownload = onDownload,
            onCancel = onCancel,
            onExport = onExport,
        )
  }
}

/**
 * Full-bleed, immersive playback: the multi-cam player fills the screen; chrome is a tap-reveal
 * scrim.
 */
@Composable
private fun ImmersivePlayer(
    state: DriveDetailUiState,
    status: SyncStatus,
    downloading: Boolean,
    live: DriveProgress?,
    resolveHd: suspend (FileKind, UInt) -> String?,
    onBack: () -> Unit,
    onPreserve: () -> Unit,
    onDownload: () -> Unit,
    onCancel: () -> Unit,
    onExport: () -> Unit,
) {
  val drive = state.drive ?: return
  // Hide the system bars while watching; restore them on leave.
  val view = LocalView.current
  DisposableEffect(Unit) {
    val window = (view.context as? Activity)?.window
    val controller = window?.let { WindowCompat.getInsetsController(it, view) }
    controller?.apply {
      systemBarsBehavior = WindowInsetsControllerCompat.BEHAVIOR_SHOW_TRANSIENT_BARS_BY_SWIPE
      hide(WindowInsetsCompat.Type.systemBars())
    }
    onDispose { controller?.show(WindowInsetsCompat.Type.systemBars()) }
  }
  // Top chrome shows/hides in lock-step with the player's tap-to-reveal controls.
  var chromeVisible by remember { mutableStateOf(true) }
  Box(Modifier.fillMaxSize().background(Color.Black)) {
    MultiCamPlayer(
        qcamera = state.qcamera,
        hdCameras = state.hdCameras,
        resolveHd = resolveHd,
        modifier = Modifier.fillMaxSize(),
        onControlsVisibleChange = { chromeVisible = it },
    )
    AnimatedVisibility(
        visible = chromeVisible,
        enter = fadeIn(),
        exit = fadeOut(),
        modifier = Modifier.align(Alignment.TopCenter),
    ) {
      Column(Modifier.fillMaxWidth().background(Color.Black.copy(alpha = 0.45f))) {
        Row(
            verticalAlignment = Alignment.CenterVertically,
            modifier = Modifier.fillMaxWidth().statusBarsPadding().padding(horizontal = 4.dp),
        ) {
          IconButton(onClick = onBack) {
            Icon(
                Icons.AutoMirrored.Filled.ArrowBack,
                contentDescription = "Back",
                tint = Color.White)
          }
          Text(
              drive.routeId,
              color = Color.White,
              style = MaterialTheme.typography.titleMedium,
              modifier = Modifier.weight(1f),
          )
          if (downloading) {
            TextButton(onClick = onCancel, modifier = Modifier.testTag("drive_detail_cancel_btn")) {
              Text("Cancel", color = Color.White)
            }
          } else if (status != SyncStatus.COMPLETE) {
            TextButton(
                onClick = onDownload, modifier = Modifier.testTag("drive_detail_download_btn")) {
                  Text("Download", color = Color.White)
                }
          }
          TextButton(onClick = onExport, modifier = Modifier.testTag("drive_detail_export_btn")) {
            Text("Export", color = Color.White)
          }
          IconButton(onClick = onPreserve, modifier = Modifier.testTag("drive_detail_preserve")) {
            Icon(
                Icons.Filled.Star,
                contentDescription = if (drive.preserved) "preserve_on" else "preserve_off",
                tint = if (drive.preserved) MaterialTheme.colorScheme.primary else Color.White,
            )
          }
        }
        if (downloading && live != null && live.bytesTotal > 0) {
          LinearProgressIndicator(
              progress = { (live.bytesDone.toFloat() / live.bytesTotal).coerceIn(0f, 1f) },
              modifier = Modifier.fillMaxWidth().testTag("drive_progress"),
          )
        }
      }
    }
  }
}

/** Pre-playback info: title, sync status, progress, and the Download/Export actions. */
@Composable
private fun DriveInfoScreen(
    state: DriveDetailUiState,
    status: SyncStatus,
    downloading: Boolean,
    live: DriveProgress?,
    onBack: () -> Unit,
    onPreserve: () -> Unit,
    onDownload: () -> Unit,
    onCancel: () -> Unit,
    onExport: () -> Unit,
) {
  val drive = state.drive ?: return
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
              IconButton(
                  onClick = onPreserve, modifier = Modifier.testTag("drive_detail_preserve")) {
                    Icon(
                        Icons.Filled.Star,
                        contentDescription = if (drive.preserved) "preserve_on" else "preserve_off",
                        tint =
                            if (drive.preserved) MaterialTheme.colorScheme.primary
                            else MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                  }
            },
        )
      }) { padding ->
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
          Row(horizontalArrangement = Arrangement.spacedBy(12.dp)) {
            when (status) {
              SyncStatus.DOWNLOADING ->
                  Button(
                      onClick = onCancel, modifier = Modifier.testTag("drive_detail_cancel_btn")) {
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
