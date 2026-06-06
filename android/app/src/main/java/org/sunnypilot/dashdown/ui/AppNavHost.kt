@file:OptIn(ExperimentalComposeUiApi::class)

package org.sunnypilot.dashdown.ui

import android.net.Uri
import androidx.compose.runtime.Composable
import androidx.compose.ui.ExperimentalComposeUiApi
import androidx.compose.ui.Modifier
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.semantics.testTagsAsResourceId
import androidx.navigation.NavHostController
import androidx.navigation.NavType
import androidx.navigation.compose.NavHost
import androidx.navigation.compose.composable
import androidx.navigation.compose.rememberNavController
import androidx.navigation.navArgument
import org.sunnypilot.dashdown.ui.detail.DriveDetailRoute
import org.sunnypilot.dashdown.ui.devices.DeviceListRoute
import org.sunnypilot.dashdown.ui.drives.DrivesListRoute
import org.sunnypilot.dashdown.ui.edit.DeviceEditRoute
import org.sunnypilot.dashdown.ui.settings.DeviceSettingsRoute

/**
 * Single-activity navigation graph. `testTagsAsResourceId` on the root exposes Compose test tags as
 * Android resource-ids so Maestro / mobile-mcp / uiautomator can match them.
 */
@Composable
fun AppNavHost(navController: NavHostController = rememberNavController()) {
  NavHost(
      navController = navController,
      startDestination = "devices",
      modifier = Modifier.semantics { testTagsAsResourceId = true },
  ) {
    composable("devices") {
      DeviceListRoute(
          onAddDevice = { navController.navigate("device/edit") },
          onDeviceClick = { id -> navController.navigate("device/$id/drives") },
          onDeviceEdit = { id -> navController.navigate("device/edit?deviceId=$id") },
          onDeviceSettings = { id -> navController.navigate("device/$id/settings") },
      )
    }
    composable(
        route = "device/edit?deviceId={deviceId}",
        arguments =
            listOf(
                navArgument("deviceId") {
                  type = NavType.StringType
                  nullable = true
                  defaultValue = null
                }),
    ) { entry ->
      val id = entry.arguments?.getString("deviceId")?.toLongOrNull()
      DeviceEditRoute(deviceId = id, onDone = { navController.popBackStack() })
    }
    composable(
        route = "device/{id}/settings",
        arguments = listOf(navArgument("id") { type = NavType.LongType }),
    ) { entry ->
      val id = entry.arguments?.getLong("id") ?: return@composable
      DeviceSettingsRoute(deviceId = id, onDone = { navController.popBackStack() })
    }
    composable(
        route = "device/{id}/drives",
        arguments = listOf(navArgument("id") { type = NavType.LongType }),
    ) { entry ->
      val id = entry.arguments?.getLong("id") ?: return@composable
      DrivesListRoute(
          deviceId = id,
          onDriveClick = { key -> navController.navigate("device/$id/drive/${Uri.encode(key)}") },
          onBack = { navController.popBackStack() },
      )
    }
    composable(
        route = "device/{id}/drive/{driveKey}",
        arguments =
            listOf(
                navArgument("id") { type = NavType.LongType },
                navArgument("driveKey") { type = NavType.StringType },
            ),
    ) { entry ->
      val id = entry.arguments?.getLong("id") ?: return@composable
      val driveKey = entry.arguments?.getString("driveKey")?.let(Uri::decode) ?: return@composable
      DriveDetailRoute(
          deviceId = id, driveKey = driveKey, onBack = { navController.popBackStack() })
    }
  }
}
