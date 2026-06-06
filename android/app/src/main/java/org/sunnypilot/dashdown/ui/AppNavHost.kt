@file:OptIn(ExperimentalComposeUiApi::class, ExperimentalMaterial3Api::class)

package org.sunnypilot.dashdown.ui

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.padding
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Text
import androidx.compose.material3.TopAppBar
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.ExperimentalComposeUiApi
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.semantics.testTagsAsResourceId
import androidx.navigation.NavHostController
import androidx.navigation.NavType
import androidx.navigation.compose.NavHost
import androidx.navigation.compose.composable
import androidx.navigation.compose.rememberNavController
import androidx.navigation.navArgument
import org.sunnypilot.dashdown.ui.devices.DeviceListRoute

/**
 * Single-activity navigation graph. `testTagsAsResourceId` on the root exposes Compose test tags as
 * Android resource-ids so Maestro / mobile-mcp / uiautomator can match them.
 *
 * The edit and drives destinations are placeholders here; they're built in later steps so
 * navigation from the device list works end to end as the shell grows.
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
    ) {
      PlaceholderScreen("Add / edit device") { navController.popBackStack() }
    }
    composable(
        route = "device/{id}/drives",
        arguments = listOf(navArgument("id") { type = NavType.LongType }),
    ) {
      PlaceholderScreen("Drives") { navController.popBackStack() }
    }
  }
}

@Composable
private fun PlaceholderScreen(title: String, onBack: () -> Unit) {
  Scaffold(
      topBar = {
        TopAppBar(
            title = { Text(title) },
            navigationIcon = {
              IconButton(onClick = onBack) {
                Icon(Icons.AutoMirrored.Filled.ArrowBack, contentDescription = "Back")
              }
            },
        )
      }) { padding ->
        Box(Modifier.fillMaxSize().padding(padding), contentAlignment = Alignment.Center) {
          Text("$title — coming in a later step", Modifier.testTag("placeholder"))
        }
      }
}
