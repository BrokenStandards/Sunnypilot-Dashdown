package org.sunnypilot.dashdown.ui.components

import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.compose.LocalLifecycleOwner
import androidx.lifecycle.repeatOnLifecycle
import kotlinx.coroutines.delay

/**
 * Run [tick] every [intervalMs] **only while the hosting screen is RESUMED**. The loop is bound to
 * the lifecycle via [repeatOnLifecycle], so it starts when the screen becomes visible/foreground
 * and is cancelled the moment it pauses — foreground status polling that never leaks into the
 * background (unlike a bare `viewModelScope.launch`).
 *
 * Delay-first: callers already do an immediate (loud) load on resume via `LifecycleResumeEffect`,
 * so the first silent [tick] fires one [intervalMs] later. Pass a [key] that changes when the work
 * should restart (e.g. a device id).
 */
@Composable
fun PollWhileResumed(intervalMs: Long, key: Any? = Unit, tick: suspend () -> Unit) {
  val owner = LocalLifecycleOwner.current
  LaunchedEffect(owner, key, intervalMs) {
    owner.lifecycle.repeatOnLifecycle(Lifecycle.State.RESUMED) {
      while (true) {
        delay(intervalMs)
        tick()
      }
    }
  }
}
