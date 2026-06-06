package org.sunnypilot.dashdown

import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import org.sunnypilot.dashdown.ui.theme.DashdownTheme
import uniffi.dashdown_core.ping
import uniffi.dashdown_core.pingAsync
import uniffi.dashdown_core.version

class MainActivity : ComponentActivity() {
  override fun onCreate(savedInstanceState: Bundle?) {
    super.onCreate(savedInstanceState)
    setContent {
      DashdownTheme {
        Surface(modifier = Modifier.fillMaxSize(), color = MaterialTheme.colorScheme.background) {
          CoreStatus()
        }
      }
    }
  }
}

@Composable
private fun CoreStatus() {
  // version() + ping() are sync FFI; pingAsync() exercises the suspend/JNA path.
  var async by remember { mutableStateOf("…") }
  LaunchedEffect(Unit) { async = pingAsync() }
  Text(
      text = "dashdown core ${version()}\nsync: ${ping()}\nasync: $async",
      modifier = Modifier.padding(24.dp),
  )
}
