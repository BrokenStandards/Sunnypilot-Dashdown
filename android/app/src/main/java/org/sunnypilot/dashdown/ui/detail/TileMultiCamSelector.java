package org.sunnypilot.dashdown.ui.detail;

import android.util.Pair;
import androidx.annotation.Nullable;
import androidx.annotation.OptIn;
import androidx.media3.common.C;
import androidx.media3.common.Timeline;
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
import java.util.List;

/**
 * Routes one merged video track group to each video renderer, per window (segment), so N camera
 * tiles render simultaneously from one player. Extends {@link TrackSelector} directly because
 * media3's {@code MappingTrackSelector} only spreads {@code TRACK_TYPE_METADATA} groups across
 * same-type renderers (hardcoded, no setter) — so it would pile every video group onto renderer 0.
 *
 * <p>For each window, {@link #windowLayouts} gives the ordered {@link VideoSlot}s whose video groups
 * appear in that window's merge (HD cameras in canonical order, then qcamera's video). The k-th
 * video group is therefore slot {@code layout.get(k)}, which carries a fixed
 * {@code rendererIndex} — so a camera always decodes on the same renderer and draws to the same
 * tile, regardless of which cameras are merged that window. A slot's track is selected only if its
 * renderer is currently {@link #visibleRenderers visible}; deselecting releases that HW decoder.
 * qcamera's audio group goes to the audio renderer when {@link #audioEnabled}.
 */
@OptIn(markerClass = UnstableApi.class)
public class TileMultiCamSelector extends TrackSelector {

  /** Per window index: the ordered video slots in that window's merge (set with the playlist). */
  public volatile List<List<VideoSlot>> windowLayouts;
  /** Per renderer index: whether that renderer's tile is currently shown (selected). */
  public volatile boolean[] visibleRenderers;
  public volatile boolean audioEnabled;

  public TileMultiCamSelector(
      List<List<VideoSlot>> windowLayouts, boolean[] visibleRenderers, boolean audioEnabled) {
    this.windowLayouts = windowLayouts;
    this.visibleRenderers = visibleRenderers;
    this.audioEnabled = audioEnabled;
  }

  /** Public passthrough to the protected {@code invalidate()} so the UI can force reselection. */
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

    // Which window (segment) this period belongs to → its video-slot layout.
    List<List<VideoSlot>> layouts = windowLayouts;
    int periodIndex = timeline.getIndexOfPeriod(periodId.periodUid);
    List<VideoSlot> layout =
        (layouts != null && periodIndex >= 0 && periodIndex < layouts.size())
            ? layouts.get(periodIndex)
            : null;

    boolean[] visible = visibleRenderers;
    int audioRenderer = firstRendererOfType(rendererCapabilities, C.TRACK_TYPE_AUDIO);

    int videoGroup = 0;
    for (int g = 0; g < trackGroups.length; g++) {
      int type = trackGroups.get(g).type;
      if (type == C.TRACK_TYPE_VIDEO) {
        if (layout != null && videoGroup < layout.size()) {
          int r = layout.get(videoGroup).getRendererIndex();
          if (r >= 0 && r < n && visible != null && r < visible.length && visible[r]) {
            configs[r] = RendererConfiguration.DEFAULT;
            selections[r] = new FixedTrackSelection(trackGroups.get(g), /* track= */ 0);
          }
        }
        videoGroup++;
      } else if (type == C.TRACK_TYPE_AUDIO) {
        if (audioEnabled && audioRenderer >= 0) {
          configs[audioRenderer] = RendererConfiguration.DEFAULT;
          selections[audioRenderer] = new FixedTrackSelection(trackGroups.get(g), /* track= */ 0);
        }
      }
      // Other group types: leave their renderer disabled (null config + null selection).
    }

    return new TrackSelectorResult(configs, selections, androidx.media3.common.Tracks.EMPTY, null);
  }

  @Override
  public void onSelectionActivated(@Nullable Object info) {
    // No-op.
  }

  private static int firstRendererOfType(RendererCapabilities[] caps, int trackType) {
    for (int r = 0; r < caps.length; r++) {
      if (caps[r].getTrackType() == trackType) {
        return r;
      }
    }
    return -1;
  }
}
