package org.sunnypilot.dashdown.spike;

import android.util.Log;
import androidx.annotation.Nullable;
import androidx.annotation.OptIn;
import androidx.media3.common.C;
import androidx.media3.common.Format;
import androidx.media3.common.Timeline;
import androidx.media3.common.TrackGroup;
import androidx.media3.common.Tracks;
import androidx.media3.common.util.UnstableApi;
import androidx.media3.exoplayer.ExoPlaybackException;
import androidx.media3.exoplayer.RendererCapabilities;
import androidx.media3.exoplayer.RendererConfiguration;
import androidx.media3.exoplayer.source.MediaSource;
import androidx.media3.exoplayer.source.TrackGroupArray;
import androidx.media3.exoplayer.trackselection.ExoTrackSelection;
import androidx.media3.exoplayer.trackselection.FixedTrackSelection;
import androidx.media3.exoplayer.trackselection.TrackSelector;
import androidx.media3.exoplayer.trackselection.TrackSelectorResult;

/**
 * SPIKE A0 (throwaway, debug-only). One merged video group → one video renderer.
 *
 * <p>Extends {@link TrackSelector} <em>directly</em> rather than {@code MappingTrackSelector},
 * because in media3 1.10.1 {@code MappingTrackSelector.findRenderer} only prefers an unassociated
 * renderer for {@code TRACK_TYPE_METADATA} groups (hardcoded, no setter) — so all video groups map
 * to renderer 0 and every other video renderer stays decoder-less. We assign positionally instead:
 * the k-th VIDEO track group goes to the k-th VIDEO renderer, so N tiles each get their own decoder
 * under one clock. Audio (qcamera) goes to the audio renderer; everything else is disabled.
 *
 * <p>{@code videoTileEnabled} is indexed by video-renderer ordinal (= merged-source order). Toggling
 * an entry then calling {@link #reselect()} re-runs selection: a now-disabled renderer releases its
 * decoder (frees a HW codec) and a now-enabled one creates its decoder against its already-attached
 * surface — all without a seek (the same-frame camera toggle).
 */
@OptIn(markerClass = UnstableApi.class)
public class TileTrackSelector extends TrackSelector {

  private static final String TAG = "SpikeTrackSel";

  public volatile boolean[] videoTileEnabled;
  public volatile boolean audioEnabled;

  public TileTrackSelector(boolean[] videoTileEnabled, boolean audioEnabled) {
    this.videoTileEnabled = videoTileEnabled;
    this.audioEnabled = audioEnabled;
  }

  /** Public passthrough to the protected {@code invalidate()} so the harness can force reselection. */
  public void reselect() {
    invalidate();
  }

  @Override
  public TrackSelectorResult selectTracks(
      RendererCapabilities[] rendererCapabilities,
      TrackGroupArray trackGroups,
      MediaSource.MediaPeriodId periodId,
      Timeline timeline)
      throws ExoPlaybackException {
    int n = rendererCapabilities.length;
    RendererConfiguration[] configs = new RendererConfiguration[n];
    ExoTrackSelection[] selections = new ExoTrackSelection[n];

    StringBuilder dbg = new StringBuilder("groups=").append(trackGroups.length).append(" [");
    for (int i = 0; i < trackGroups.length; i++) {
      Format f = trackGroups.get(i).getFormat(0);
      dbg.append(i).append(":t").append(trackGroups.get(i).type).append("/").append(f.sampleMimeType);
      if (f.width > 0) dbg.append(" ").append(f.width).append("x").append(f.height);
      dbg.append(i + 1 < trackGroups.length ? ", " : "");
    }
    dbg.append("] -> ");

    int videoOrdinal = 0;
    for (int r = 0; r < n; r++) {
      int type = rendererCapabilities[r].getTrackType();
      if (type == C.TRACK_TYPE_VIDEO) {
        TrackGroup g = findNthGroupOfType(trackGroups, C.TRACK_TYPE_VIDEO, videoOrdinal);
        boolean on = videoOrdinal < videoTileEnabled.length && videoTileEnabled[videoOrdinal] && g != null;
        if (on) {
          configs[r] = RendererConfiguration.DEFAULT;
          selections[r] = new FixedTrackSelection(g, /* track= */ 0);
        }
        dbg.append("r").append(r).append("=V").append(videoOrdinal).append(on ? "(on) " : "(off) ");
        videoOrdinal++;
      } else if (type == C.TRACK_TYPE_AUDIO) {
        TrackGroup g = findNthGroupOfType(trackGroups, C.TRACK_TYPE_AUDIO, 0);
        boolean on = audioEnabled && g != null;
        if (on) {
          configs[r] = RendererConfiguration.DEFAULT;
          selections[r] = new FixedTrackSelection(g, /* track= */ 0);
        }
        dbg.append("r").append(r).append("=A").append(on ? "(on) " : "(off) ");
      }
      // Other renderer types: leave disabled (null config + null selection).
    }
    Log.i(TAG, dbg.toString());

    return new TrackSelectorResult(configs, selections, Tracks.EMPTY, /* info= */ null);
  }

  @Override
  public void onSelectionActivated(@Nullable Object info) {
    // No-op: `info` is the 4th TrackSelectorResult ctor arg (null here).
  }

  /** The n-th track group of {@code type} in source order, or null if there are fewer. */
  @Nullable
  private static TrackGroup findNthGroupOfType(TrackGroupArray groups, int type, int n) {
    int seen = 0;
    for (int i = 0; i < groups.length; i++) {
      TrackGroup g = groups.get(i);
      if (g.type == type) {
        if (seen == n) return g;
        seen++;
      }
    }
    return null;
  }
}
