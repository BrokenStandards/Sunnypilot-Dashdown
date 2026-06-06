package org.sunnypilot.dashdown.ui

import androidx.compose.runtime.Composable
import androidx.compose.ui.platform.LocalContext
import org.sunnypilot.dashdown.DashdownApp
import org.sunnypilot.dashdown.data.DashdownRepository

/** Resolve the app-wide [DashdownRepository] from the Compose tree. */
@Composable
fun rememberRepository(): DashdownRepository {
  val context = LocalContext.current
  return (context.applicationContext as DashdownApp).locator.repository
}
