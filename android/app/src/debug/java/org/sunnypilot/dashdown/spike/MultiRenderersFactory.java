package org.sunnypilot.dashdown.spike;

import android.content.Context;
import android.os.Handler;
import androidx.annotation.Nullable;
import androidx.annotation.OptIn;
import androidx.media3.common.util.UnstableApi;
import androidx.media3.exoplayer.DecoderCounters;
import androidx.media3.exoplayer.DefaultRenderersFactory;
import androidx.media3.exoplayer.Renderer;
import androidx.media3.exoplayer.mediacodec.MediaCodecSelector;
import androidx.media3.exoplayer.video.MediaCodecVideoRenderer;
import androidx.media3.exoplayer.video.VideoRendererEventListener;
import java.util.ArrayList;
import java.util.List;

/**
 * SPIKE A0 (throwaway, debug-only). Builds {@code videoRendererCount} independent
 * {@link MediaCodecVideoRenderer}s inside ONE ExoPlayer so N tiles share one clock — the candidate
 * fix for the chase-by-seek choppiness. Each renderer gets its own {@link VideoRendererEventListener}
 * so we can read per-tile decoder counters (dropped/rendered frames) to prove smoothness.
 *
 * <p>Not Kotlin: the media3 extension points are Java and we avoid Kotlin platform-type friction on
 * the {@code MappingTrackSelector.selectTracks} override (see {@link TileTrackSelector}).
 */
@OptIn(markerClass = UnstableApi.class)
public class MultiRenderersFactory extends DefaultRenderersFactory {

  /** Live, thread-safe per-renderer stats (updated on the playback thread, read on the UI thread). */
  public static final class TileStats {
    @Nullable public volatile DecoderCounters counters;
    public volatile boolean firstFrameRendered;
    public volatile int lastDroppedBatch;
  }

  private final int videoRendererCount;
  public final List<TileStats> stats = new ArrayList<>();
  private final List<Renderer> videoRenderers = new ArrayList<>();

  public MultiRenderersFactory(Context context, int videoRendererCount) {
    super(context);
    this.videoRendererCount = videoRendererCount;
    for (int i = 0; i < videoRendererCount; i++) {
      stats.add(new TileStats());
    }
  }

  /** The N video renderers in player renderer-index order; each is the target for MSG_SET_VIDEO_OUTPUT. */
  public List<Renderer> getVideoRenderers() {
    return videoRenderers;
  }

  @Override
  protected void buildVideoRenderers(
      Context context,
      int extensionRendererMode,
      MediaCodecSelector mediaCodecSelector,
      boolean enableDecoderFallback,
      Handler eventHandler,
      VideoRendererEventListener eventListener,
      long allowedVideoJoiningTimeMs,
      ArrayList<Renderer> out) {
    videoRenderers.clear();
    for (int i = 0; i < videoRendererCount; i++) {
      final TileStats s = stats.get(i);
      VideoRendererEventListener listener =
          new VideoRendererEventListener() {
            @Override
            public void onVideoEnabled(DecoderCounters counters) {
              s.counters = counters;
            }

            @Override
            public void onVideoDisabled(DecoderCounters counters) {
              s.firstFrameRendered = false;
              s.counters = null;
            }

            @Override
            public void onRenderedFirstFrame(Object output, long renderTimeMs) {
              s.firstFrameRendered = true;
            }

            @Override
            public void onDroppedFrames(int count, long elapsedMs) {
              s.lastDroppedBatch = count;
            }
          };
      MediaCodecVideoRenderer renderer =
          new MediaCodecVideoRenderer.Builder(context)
              .setMediaCodecSelector(mediaCodecSelector)
              .setAllowedJoiningTimeMs(allowedVideoJoiningTimeMs)
              .setEnableDecoderFallback(enableDecoderFallback)
              .setEventHandler(eventHandler)
              .setEventListener(listener)
              .setMaxDroppedFramesToNotify(10)
              .build();
      videoRenderers.add(renderer);
      out.add(renderer);
    }
  }

  // Disable pre-warming (a single secondary renderer), which assumes one primary video renderer.
  @Nullable
  @Override
  protected Renderer buildSecondaryVideoRenderer(
      Renderer renderer,
      Context context,
      int extensionRendererMode,
      MediaCodecSelector mediaCodecSelector,
      boolean enableDecoderFallback,
      Handler eventHandler,
      VideoRendererEventListener eventListener,
      long allowedVideoJoiningTimeMs) {
    return null;
  }
}
