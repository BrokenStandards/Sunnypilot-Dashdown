package org.sunnypilot.dashdown.ui.components

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.unit.dp
import uniffi.dashdown_core.ConnDot

fun connDotColor(dot: ConnDot?): Color =
    when (dot) {
      ConnDot.GREEN -> Color(0xFF2E7D32)
      ConnDot.BLUE -> Color(0xFF1565C0)
      ConnDot.RED -> Color(0xFFC62828)
      null -> Color(0xFF9E9E9E)
    }

/** Stable accessibility label encoding the dot state (`conn_dot_green|blue|red|unknown`). */
fun connDotLabel(dot: ConnDot?): String =
    "conn_dot_" +
        when (dot) {
          ConnDot.GREEN -> "green"
          ConnDot.BLUE -> "blue"
          ConnDot.RED -> "red"
          null -> "unknown"
        }

/**
 * Colored connectivity dot. Its `contentDescription` encodes the state so Maestro / Compose tests
 * can assert green/blue/red transitions.
 */
@Composable
fun ConnDotIndicator(dot: ConnDot?, modifier: Modifier = Modifier) {
  val label = connDotLabel(dot)
  Box(
      modifier.size(12.dp).clip(CircleShape).background(connDotColor(dot)).semantics {
        contentDescription = label
      })
}
