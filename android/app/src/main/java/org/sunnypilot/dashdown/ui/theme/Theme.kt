package org.sunnypilot.dashdown.ui.theme

import androidx.compose.foundation.isSystemInDarkTheme
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.darkColorScheme
import androidx.compose.material3.lightColorScheme
import androidx.compose.runtime.Composable

private val DarkColors =
    darkColorScheme(primary = Purple80, secondary = PurpleGrey80, tertiary = Pink80)
private val LightColors =
    lightColorScheme(primary = Purple40, secondary = PurpleGrey40, tertiary = Pink40)

@Composable
fun DashdownTheme(
    darkTheme: Boolean = isSystemInDarkTheme(),
    content: @Composable () -> Unit,
) {
  MaterialTheme(
      colorScheme = if (darkTheme) DarkColors else LightColors,
      typography = Typography,
      content = content,
  )
}
