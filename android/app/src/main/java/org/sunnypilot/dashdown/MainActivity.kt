package org.sunnypilot.dashdown

import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import org.sunnypilot.dashdown.ui.AppNavHost
import org.sunnypilot.dashdown.ui.theme.DashdownTheme

class MainActivity : ComponentActivity() {
  override fun onCreate(savedInstanceState: Bundle?) {
    super.onCreate(savedInstanceState)
    setContent { DashdownTheme { AppNavHost() } }
  }
}
