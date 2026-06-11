package org.sunnypilot.dashdown.ui.detail;

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
 * Builds {@code videoRendererCount} independent {@link MediaCodecVideoRenderer}s inside ONE
 * ExoPlayer so N camera tiles share one clock — the structural fix for multi-camera sync (no
 * master/follower, no chase-seeks, no decoder flushes). Each renderer keeps an index-stable slot
 * (see {@code CameraId.rendererIndex}); a per-renderer {@link VideoRendererEventListener} exposes
 * "first frame rendered" so each tile can clear its "Preparing HD…" spinner independently.
 *
 * <p>Written in Java (not Kotlin) to override the media3 extension points without Kotlin
 * platform-type friction; pairs with {@link TileMultiCamSelector}, which routes one merged video
 * group to each renderer (media3's {@code MappingTrackSelector} can't spread video groups).
 */
@OptIn(markerClass = UnstableApi.class)
public class MultiRenderersFactory extends DefaultRenderersFactory {

  /** Live readiness for one video renderer (written on the playback thread, read on the UI thread). */
  public static final class TileStats {
    /** True once this renderer has rendered a frame; cleared when it is disabled (track deselected). */
    public volatile boolean firstFrameRendered;
    /** This renderer's decoded video size + pixel aspect, for letterboxing the tile (0 until known). */
    public volatile int videoWidth;
    public volatile int videoHeight;
    public volatile float pixelWidthHeightRatio = 1f;

    /** Display aspect (w/h) honoring non-square pixels, or 0 if not yet reported. */
    public float displayAspect() {
      if (videoWidth <= 0 || videoHeight <= 0) {
        return 0f;
      }
      return (videoWidth * pixelWidthHeightRatio) / videoHeight;
    }
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

  /** The N video renderers in renderer-index order; each is the target for MSG_SET_VIDEO_OUTPUT. */
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
            public void onRenderedFirstFrame(Object output, long renderTimeMs) {
              s.firstFrameRendered = true;
            }

            @Override
            public void onVideoSizeChanged(androidx.media3.common.VideoSize videoSize) {
              s.videoWidth = videoSize.width;
              s.videoHeight = videoSize.height;
              s.pixelWidthHeightRatio = videoSize.pixelWidthHeightRatio;
            }

            @Override
            public void onVideoDisabled(DecoderCounters counters) {
              s.firstFrameRendered = false;
            }
          };
      MediaCodecVideoRenderer renderer =
          new MediaCodecVideoRenderer.Builder(context)
              .setMediaCodecSelector(mediaCodecSelector)
              .setAllowedJoiningTimeMs(allowedVideoJoiningTimeMs)
              .setEnableDecoderFallback(enableDecoderFallback)
              .setEventHandler(eventHandler)
              .setEventListener(listener)
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
