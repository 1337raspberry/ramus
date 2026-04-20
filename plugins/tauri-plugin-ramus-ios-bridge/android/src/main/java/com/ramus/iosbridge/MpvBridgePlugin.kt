package com.ramus.iosbridge

import android.app.Activity
import android.net.Uri
import android.os.Handler
import android.os.Looper
import android.util.Log
import androidx.media3.common.AudioAttributes
import androidx.media3.common.C
import androidx.media3.common.MediaItem
import androidx.media3.common.PlaybackException
import androidx.media3.common.Player
import androidx.media3.exoplayer.ExoPlayer
import app.tauri.annotation.Command
import app.tauri.annotation.InvokeArg
import app.tauri.annotation.TauriPlugin
import app.tauri.plugin.Invoke
import app.tauri.plugin.JSObject
import app.tauri.plugin.Plugin

private const val TAG = "MpvBridge"
private const val POSITION_POLL_MS = 500L

@InvokeArg
internal class LoadFileArgs {
    lateinit var url: String
    lateinit var mode: String
    var options: String? = null
}

@InvokeArg
internal class LoadFileAtArgs {
    lateinit var url: String
    var index: Long = 0
    var options: String? = null
}

@InvokeArg
internal class PlaylistIndexArgs {
    var index: Long = 0
}

@InvokeArg
internal class PlaylistMoveArgs {
    var from: Long = 0
    var to: Long = 0
}

@InvokeArg
internal class SeekArgs {
    var position: Double = 0.0
}

@InvokeArg
internal class PauseArgs {
    var paused: Boolean = false
}

@InvokeArg
internal class VolumeArgs {
    var volume: Double = 100.0
}

@InvokeArg
internal class AudioFiltersArgs {
    lateinit var value: String
}

/**
 * Tauri plugin that owns the ExoPlayer instance on Android.
 *
 * The Rust IPC surface (`mpv_load_file`, `mpv_pause_change`, …) is named
 * after libmpv because desktop and iOS use mpv directly; Android wraps
 * ExoPlayer behind the same names so the Rust `MpvPlayer` trait + the
 * shared `mpv_mobile.rs` listener wiring work without per-platform forks.
 *
 * All ExoPlayer access is marshalled to the main thread — ExoPlayer is
 * not thread-safe.
 */
@TauriPlugin
class MpvBridgePlugin(private val activity: Activity) : Plugin(activity) {
    private val mainHandler = Handler(Looper.getMainLooper())
    private var player: ExoPlayer? = null
    private val playerListener = PlayerListener()
    private var lastReportedDuration: Long = C.TIME_UNSET
    private var lastReportedIndex: Int = -1
    // mpv `file-loaded` is one-shot per loadfile; ExoPlayer re-enters
    // STATE_READY after every seek + buffer recovery. Gate the bridged
    // event so the Rust callback only fires on genuine track loads.
    private var fileLoadedEmitted = false

    @Command
    fun mpvInit(invoke: Invoke) {
        // Always post (never inline-execute via `runOnMain`) — Tauri can
        // dispatch this command on a background IPC thread, and an inline
        // path here followed by a queued path for the next command would
        // let `mpvLoadFile` race ahead of `mpvInit`. Posting unconditionally
        // serialises init + first load on the main looper FIFO.
        mainHandler.post {
            if (player == null) {
                val audioAttrs = AudioAttributes.Builder()
                    .setUsage(C.USAGE_MEDIA)
                    .setContentType(C.AUDIO_CONTENT_TYPE_MUSIC)
                    .build()
                player = ExoPlayer.Builder(activity.applicationContext)
                    // `true` here lets ExoPlayer manage audio focus + ducking
                    // automatically (request on play, release on pause/stop).
                    .setAudioAttributes(audioAttrs, /* handleAudioFocus = */ true)
                    .setHandleAudioBecomingNoisy(true)
                    .build()
                    .apply {
                        addListener(playerListener)
                    }
                mainHandler.post(positionPoller)
                Log.i(TAG, "ExoPlayer initialised")
            }
            invoke.resolve()
        }
    }

    @Command
    fun initAudio(invoke: Invoke) {
        // Audio focus is handled by ExoPlayer (see setAudioAttributes above).
        // Foreground service + MediaSession come in a follow-up phase.
        invoke.resolve()
    }

    @Command
    fun mpvLoadFile(invoke: Invoke) {
        val args = invoke.parseArgs(LoadFileArgs::class.java)
        runOnMain {
            val p = player ?: return@runOnMain invoke.reject("player not initialised")
            val item = MediaItem.fromUri(Uri.parse(args.url))
            when (args.mode) {
                "replace" -> {
                    p.setMediaItem(item)
                    p.prepare()
                    p.play()
                }
                "append" -> {
                    p.addMediaItem(item)
                    if (p.playbackState == Player.STATE_IDLE) p.prepare()
                }
                "append-play" -> {
                    val wasEmpty = p.mediaItemCount == 0
                    p.addMediaItem(item)
                    if (wasEmpty || p.playbackState == Player.STATE_IDLE) {
                        p.prepare()
                        p.play()
                    }
                }
                else -> Log.w(TAG, "unknown loadfile mode: ${args.mode}")
            }
            invoke.resolve()
        }
    }

    @Command
    fun mpvLoadFileAt(invoke: Invoke) {
        val args = invoke.parseArgs(LoadFileAtArgs::class.java)
        runOnMain {
            val p = player ?: return@runOnMain invoke.reject("player not initialised")
            p.addMediaItem(args.index.toInt(), MediaItem.fromUri(Uri.parse(args.url)))
            if (p.playbackState == Player.STATE_IDLE) p.prepare()
            invoke.resolve()
        }
    }

    @Command
    fun mpvPlaylistPlayIndex(invoke: Invoke) {
        val args = invoke.parseArgs(PlaylistIndexArgs::class.java)
        runOnMain {
            val p = player ?: return@runOnMain invoke.reject("player not initialised")
            p.seekTo(args.index.toInt(), 0L)
            if (p.playbackState == Player.STATE_IDLE) p.prepare()
            p.play()
            invoke.resolve()
        }
    }

    @Command
    fun mpvPlaylistRemove(invoke: Invoke) {
        val args = invoke.parseArgs(PlaylistIndexArgs::class.java)
        runOnMain {
            val p = player ?: return@runOnMain invoke.reject("player not initialised")
            p.removeMediaItem(args.index.toInt())
            invoke.resolve()
        }
    }

    @Command
    fun mpvPlaylistMove(invoke: Invoke) {
        val args = invoke.parseArgs(PlaylistMoveArgs::class.java)
        runOnMain {
            val p = player ?: return@runOnMain invoke.reject("player not initialised")
            p.moveMediaItem(args.from.toInt(), args.to.toInt())
            invoke.resolve()
        }
    }

    @Command
    fun mpvSeek(invoke: Invoke) {
        val args = invoke.parseArgs(SeekArgs::class.java)
        runOnMain {
            val p = player ?: return@runOnMain invoke.reject("player not initialised")
            p.seekTo((args.position * 1000.0).toLong())
            invoke.resolve()
        }
    }

    @Command
    fun mpvSetPause(invoke: Invoke) {
        val args = invoke.parseArgs(PauseArgs::class.java)
        runOnMain {
            val p = player ?: return@runOnMain invoke.reject("player not initialised")
            if (args.paused) p.pause() else p.play()
            invoke.resolve()
        }
    }

    @Command
    fun mpvSetVolume(invoke: Invoke) {
        val args = invoke.parseArgs(VolumeArgs::class.java)
        runOnMain {
            val p = player ?: return@runOnMain invoke.reject("player not initialised")
            // mpv: 0-100; ExoPlayer: 0.0-1.0
            p.volume = (args.volume / 100.0).toFloat().coerceIn(0f, 1f)
            invoke.resolve()
        }
    }

    @Command
    fun mpvGetVolume(invoke: Invoke) {
        runOnMain {
            val p = player
            val volume = if (p != null) (p.volume * 100.0) else 100.0
            invoke.resolve(JSObject().put("volume", volume))
        }
    }

    @Command
    fun mpvSetAudioFilters(invoke: Invoke) {
        // mpv `af` filter chain (lavfi biquad EQ etc.) has no direct ExoPlayer
        // equivalent. Wiring Android's system Equalizer FX or a custom
        // AudioProcessor is the next step; for now this no-ops so the EQ
        // toggle in the UI doesn't error.
        val args = invoke.parseArgs(AudioFiltersArgs::class.java)
        if (args.value.isNotEmpty()) {
            Log.w(TAG, "EQ filter not yet implemented on Android: ${args.value}")
        }
        invoke.resolve()
    }

    @Command
    fun mpvStop(invoke: Invoke) {
        runOnMain {
            val p = player ?: return@runOnMain invoke.resolve()
            p.stop()
            p.clearMediaItems()
            invoke.resolve()
        }
    }

    private val positionPoller = object : Runnable {
        override fun run() {
            val p = player
            if (p != null && p.isPlaying) {
                val seconds = p.currentPosition / 1000.0
                trigger("mpvPositionChange", JSObject().put("position", seconds))
            }
            mainHandler.postDelayed(this, POSITION_POLL_MS)
        }
    }

    private inner class PlayerListener : Player.Listener {
        override fun onIsPlayingChanged(isPlaying: Boolean) {
            trigger("mpvPauseChange", JSObject().put("paused", !isPlaying))
        }

        override fun onPlaybackStateChanged(state: Int) {
            when (state) {
                Player.STATE_READY -> {
                    val p = player ?: return
                    val dur = p.duration
                    if (dur != C.TIME_UNSET && dur != lastReportedDuration) {
                        lastReportedDuration = dur
                        trigger("mpvDurationChange", JSObject().put("duration", dur / 1000.0))
                    }
                    if (!fileLoadedEmitted) {
                        fileLoadedEmitted = true
                        trigger("mpvFileLoaded", JSObject())
                    }
                }
                Player.STATE_ENDED -> {
                    trigger("mpvFileEnded", JSObject().put("reason", "eof"))
                }
                Player.STATE_IDLE -> {
                    trigger("mpvIdleActive", JSObject())
                }
                Player.STATE_BUFFERING -> { /* no-op */ }
            }
        }

        override fun onMediaItemTransition(mediaItem: MediaItem?, reason: Int) {
            val p = player ?: return
            val idx = p.currentMediaItemIndex
            if (idx != lastReportedIndex) {
                lastReportedIndex = idx
                trigger("mpvPlaylistPosChange", JSObject().put("index", idx.toLong()))
            }
            // Fresh item → reset duration + file-loaded latch so the next
            // STATE_READY re-emits both for the new track.
            lastReportedDuration = C.TIME_UNSET
            fileLoadedEmitted = false
        }

        override fun onPlayerError(error: PlaybackException) {
            Log.e(TAG, "ExoPlayer error: ${error.errorCodeName}", error)
            trigger("mpvFileEnded", JSObject().put("reason", "error"))
        }
    }

    override fun onDestroy() {
        // Tauri delivers `onDestroy` on the main thread (TauriActivity.onDestroy
        // runs synchronously), so no `runOnMain` wrapper. Order matters: cancel
        // the poller, null the player BEFORE release() so any in-flight poller
        // body or listener callback that wins the race sees `player == null`
        // and bails instead of touching a released ExoPlayer. Removing the
        // listener also drops the `inner class` reference back to this plugin
        // (and the Activity it holds), which would otherwise leak until GC.
        mainHandler.removeCallbacks(positionPoller)
        val p = player
        player = null
        p?.removeListener(playerListener)
        p?.release()
    }

    private fun runOnMain(block: () -> Unit) {
        if (Looper.myLooper() == Looper.getMainLooper()) block()
        else mainHandler.post(block)
    }
}
