package com.ramus.iosbridge

import android.Manifest
import android.app.Activity
import android.app.AlertDialog
import android.content.Intent
import android.content.pm.PackageManager
import android.net.Uri
import android.os.Build
import android.os.Handler
import android.os.Looper
import android.util.Log
import androidx.core.app.ActivityCompat
import androidx.core.content.ContextCompat
import androidx.media3.common.AudioAttributes
import androidx.media3.common.C
import androidx.media3.common.MediaItem
import androidx.media3.common.MediaMetadata
import androidx.media3.common.PlaybackException
import androidx.media3.common.Player
import androidx.media3.common.util.UnstableApi
import androidx.media3.exoplayer.ExoPlayer
import androidx.media3.session.MediaSession
import app.tauri.annotation.Command
import app.tauri.annotation.InvokeArg
import app.tauri.annotation.TauriPlugin
import app.tauri.plugin.Invoke
import app.tauri.plugin.JSObject
import app.tauri.plugin.Plugin
import java.io.File

private const val TAG = "MpvBridge"
private const val POSITION_POLL_MS = 500L
private const val POST_NOTIFICATIONS_REQUEST_CODE = 1001
// Debug toggle: when false, the MediaSession + foreground service are skipped
// entirely. Lets us A/B whether MediaSession is the cause of a playback issue
// without rebuilding. Flip via `adb shell setprop debug.ramus.media_session 0`
// before launching, then back to anything else (or unset) to re-enable.
private const val MEDIA_SESSION_PROP = "debug.ramus.media_session"

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

@InvokeArg
internal class NowPlayingArgs {
    lateinit var title: String
    lateinit var artist: String
    lateinit var album: String
    var duration: Double = 0.0
    var position: Double = 0.0
    var isPlaying: Boolean = false
    var coverUrl: String? = null
}

@InvokeArg
internal class MediaAccentArgs {
    var r: Int = 0
    var g: Int = 0
    var b: Int = 0
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
@UnstableApi
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

    // MediaSession + foreground service drive the lock-screen / notification
    // controls and keep audio alive across screen lock. Built once per
    // `mpvInit`; released in `onDestroy` to release the session-level
    // audio focus and let the service shut down.
    private var mediaSession: MediaSession? = null
    // Foreground service is started on the FIRST `isPlaying=true` transition
    // (not in mpvInit) so Media3's `DefaultMediaNotificationProvider` can
    // call `startForeground` with a real MediaStyle notification within the
    // 5s `startForegroundService` window — calling startForegroundService
    // before any track is playing crashes with
    // ForegroundServiceDidNotStartInTimeException because Media3 only
    // promotes once playback is active.
    private var foregroundServiceStarted = false
    // Cache the last-pushed metadata so back-to-back `nowPlayingUpdate`
    // calls with the same payload (transport ticks, etc.) skip the
    // expensive `replaceMediaItem` round-trip — and so the artwork bytes
    // are only re-read from disk when the cover URL actually changes.
    private var lastTitle: String? = null
    private var lastArtist: String? = null
    private var lastAlbum: String? = null
    private var lastCoverUrl: String? = null
    private var lastCoverBytes: ByteArray? = null
    // Monotonic counter bumped on every `nowPlayingUpdate` that enters
    // the cover-loading path. The background IO callback compares its
    // captured gen against the live value on apply — if anything newer
    // has landed (another update, or `nowPlayingClear`), the stale
    // bytes are dropped. Fixes a first-call bug where the original
    // guard compared against `lastCoverUrl` (still null on first load),
    // silently skipping every first `applyMetadata` when the art cache
    // was warm and the IO branch was taken.
    private var coverRequestGen: Int = 0
    // Worker for off-main artwork reads. The file IO is small (~30KB
    // jpeg from the local image cache) but main-thread reads stack up
    // when nowPlayingUpdate fires several times per track.
    private val ioHandler = Handler(android.os.HandlerThread("MpvBridgeIO").apply { start() }.looper)

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
                val p = ExoPlayer.Builder(activity.applicationContext)
                    // `true` here lets ExoPlayer manage audio focus + ducking
                    // automatically (request on play, release on pause/stop).
                    .setAudioAttributes(audioAttrs, /* handleAudioFocus = */ true)
                    .setHandleAudioBecomingNoisy(true)
                    .build()
                    .apply {
                        addListener(playerListener)
                    }
                player = p
                mainHandler.post(positionPoller)

                // Build the MediaSession around the player so the
                // foreground service can publish lock-screen controls.
                // Wrapped in try/catch so a Media3 quirk can't take
                // playback down with it — without the session you lose
                // lock-screen controls but audio still works.
                if (mediaSessionEnabled()) {
                    try {
                        val session = MediaSession.Builder(activity.applicationContext, p).build()
                        mediaSession = session
                        MpvForegroundService.attachSession(session)
                    } catch (e: Throwable) {
                        Log.e(TAG, "MediaSession build failed; falling back to plain ExoPlayer", e)
                    }
                } else {
                    Log.w(TAG, "MediaSession disabled via $MEDIA_SESSION_PROP")
                }

                ensurePostNotificationsPermission()
                Log.i(TAG, "ExoPlayer initialised (mediaSession=${mediaSession != null})")
            }
            invoke.resolve()
        }
    }

    private fun mediaSessionEnabled(): Boolean {
        // Default ON. Set the prop to "0" / "false" / "off" to disable.
        return try {
            val cls = Class.forName("android.os.SystemProperties")
            val get = cls.getMethod("get", String::class.java, String::class.java)
            val raw = (get.invoke(null, MEDIA_SESSION_PROP, "") as? String).orEmpty()
            !raw.equals("0", true) && !raw.equals("false", true) && !raw.equals("off", true)
        } catch (_: Throwable) {
            true
        }
    }

    private fun ensureForegroundServiceStarted() {
        if (foregroundServiceStarted) return
        if (mediaSession == null) return
        try {
            ContextCompat.startForegroundService(
                activity,
                Intent(activity, MpvForegroundService::class.java),
            )
            foregroundServiceStarted = true
            Log.i(TAG, "MpvForegroundService started")
        } catch (e: Exception) {
            // App is backgrounded → can't start a foreground service from
            // here on Android 12+. Audio continues playing but no
            // lock-screen controls until the next foreground transition.
            Log.w(TAG, "startForegroundService failed", e)
        }
    }

    private fun ensurePostNotificationsPermission() {
        if (Build.VERSION.SDK_INT < Build.VERSION_CODES.TIRAMISU) return
        if (activity.isFinishing || activity.isDestroyed) return
        if (ContextCompat.checkSelfPermission(
                activity,
                Manifest.permission.POST_NOTIFICATIONS,
            ) == PackageManager.PERMISSION_GRANTED
        ) return

        AlertDialog.Builder(activity)
            .setTitle("Lock Screen Controls")
            .setMessage(
                "Ramus needs notification permission to show album art " +
                    "and playback controls on your lock screen."
            )
            .setPositiveButton("Allow") { _, _ ->
                ActivityCompat.requestPermissions(
                    activity,
                    arrayOf(Manifest.permission.POST_NOTIFICATIONS),
                    POST_NOTIFICATIONS_REQUEST_CODE,
                )
            }
            .setNegativeButton("Not Now", null)
            .show()
    }

    @Command
    fun initAudio(invoke: Invoke) {
        // Audio focus is handled by ExoPlayer (`setAudioAttributes(.., true)`
        // in `mpvInit`). Foreground service + MediaSession are wired in
        // `mpvInit` too — this remains a no-op so the cross-platform
        // init order (`mpv_init` → `init_audio`, see `mpv_android.rs`)
        // doesn't error.
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
            val idx = args.index.toInt()
            if (idx < 0 || idx >= p.mediaItemCount) return@runOnMain invoke.resolve()
            p.seekTo(idx, 0L)
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
            val idx = args.index.toInt()
            if (idx < 0 || idx >= p.mediaItemCount) return@runOnMain invoke.resolve()
            p.removeMediaItem(idx)
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
            if (p.mediaItemCount == 0) return@runOnMain invoke.resolve()
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
            lastReportedIndex = -1
            lastReportedDuration = C.TIME_UNSET
            fileLoadedEmitted = false
            foregroundServiceStarted = false
            invoke.resolve()
        }
    }

    @Command
    fun nowPlayingUpdate(invoke: Invoke) {
        val args = invoke.parseArgs(NowPlayingArgs::class.java)
        // Resolve the IPC immediately — the actual MediaSession update
        // is best-effort and we don't want Rust callers to block on
        // disk IO or main-thread queue depth.
        invoke.resolve()

        // Everything after this point must be on the main looper: reads
        // of lastTitle etc. need to be serialized with main-thread
        // writers, and `applyMetadata` touches the ExoPlayer (which is
        // strictly main-thread only). Tauri dispatches @Command on an
        // IPC worker, so hop back to main before doing any work.
        mainHandler.post {
            val titleChanged = args.title != lastTitle
            val artistChanged = args.artist != lastArtist
            val albumChanged = args.album != lastAlbum
            val coverChanged = args.coverUrl != lastCoverUrl
            if (!titleChanged && !artistChanged && !albumChanged && !coverChanged) return@post
            Log.i(
                TAG,
                "nowPlayingUpdate: title='${args.title}' artist='${args.artist}' album='${args.album}' cover=${args.coverUrl != null} coverChanged=$coverChanged",
            )

            lastTitle = args.title
            lastArtist = args.artist
            lastAlbum = args.album

            // With MediaSession disabled there's no session for the
            // lock-screen widget to refresh from. Skip the whole
            // replaceMediaItem dance — it'd just churn the player
            // without changing anything observable.
            if (mediaSession == null) {
                Log.w(TAG, "nowPlayingUpdate: mediaSession null, skipping apply")
                return@post
            }

            if (coverChanged) {
                val coverUrl = args.coverUrl
                // Bump the generation so any already-in-flight cover
                // load's result is dropped on arrival — only the most
                // recent request's bytes get applied. Set this BEFORE
                // dispatching to the IO thread so a racing second call
                // sees the new gen before its own IO returns.
                val myGen = ++coverRequestGen
                ioHandler.post {
                    val bytes = loadArtworkBytes(coverUrl)
                    mainHandler.post {
                        // Drop stale result if a newer cover request has
                        // landed, OR if everything was cleared via
                        // `nowPlayingClear` while we were loading.
                        if (myGen != coverRequestGen) {
                            Log.i(TAG, "applyMetadata: cover gen stale ($myGen != $coverRequestGen), dropping")
                            return@post
                        }
                        lastCoverUrl = coverUrl
                        lastCoverBytes = bytes
                        applyMetadata()
                    }
                }
            } else {
                applyMetadata()
            }
        }
    }

    private fun applyMetadata() {
        val p = player
        if (p == null) {
            Log.w(TAG, "applyMetadata: player null")
            return
        }
        if (p.mediaItemCount == 0) {
            Log.w(TAG, "applyMetadata: mediaItemCount=0 (player not loaded yet)")
            return
        }
        val current = p.currentMediaItem
        if (current == null) {
            Log.w(TAG, "applyMetadata: currentMediaItem null")
            return
        }
        val currentUri = current.localConfiguration?.uri
        if (currentUri == null) {
            Log.w(TAG, "applyMetadata: currentUri null")
            return
        }
        Log.i(
            TAG,
            "applyMetadata: idx=${p.currentMediaItemIndex} title='$lastTitle' artist='$lastArtist' album='$lastAlbum' hasCover=${lastCoverBytes != null}",
        )

        val builder = MediaMetadata.Builder()
            .setTitle(lastTitle)
            .setArtist(lastArtist)
            .setAlbumTitle(lastAlbum)
        lastCoverBytes?.let {
            // `setArtworkData` (vs `setArtworkUri`) sidesteps the
            // file:// permission issue (androidx/media#2331) where
            // system processes can't open app-private files.
            builder.setArtworkData(it, MediaMetadata.PICTURE_TYPE_FRONT_COVER)
        }

        // `replaceMediaItem` with the same URI keeps playback
        // position seamless; only the metadata flips. Same-index
        // transitions are filtered in `onMediaItemTransition` so
        // they don't echo back through the event pipeline.
        val newItem = MediaItem.Builder()
            .setUri(currentUri)
            .setMediaMetadata(builder.build())
            .build()
        try {
            p.replaceMediaItem(p.currentMediaItemIndex, newItem)
            Log.i(TAG, "applyMetadata: replaceMediaItem ok")
        } catch (e: Exception) {
            Log.w(TAG, "replaceMediaItem failed", e)
        }
    }

    @Command
    fun setMediaAccent(invoke: Invoke) {
        val args = invoke.parseArgs(MediaAccentArgs::class.java)
        invoke.resolve()
        mainHandler.post {
            // Pack sRGB into ARGB with full alpha. Clamp each channel
            // — Tauri deserialises `u8` as `Int` and an out-of-range
            // value from a buggy caller would corrupt adjacent channels.
            val r = args.r.coerceIn(0, 255)
            val g = args.g.coerceIn(0, 255)
            val b = args.b.coerceIn(0, 255)
            val argb = (0xFF shl 24) or (r shl 16) or (g shl 8) or b
            MpvForegroundService.setAccent(argb)

            // Nudge Media3 to rebuild the notification: calling the
            // provider's `setAccent` only stashes the new colour — the
            // notification itself is only re-created on specific player
            // events. Replacing the current media item with an identical
            // one fires `onMediaItemTransition` with
            // `MEDIA_ITEM_TRANSITION_REASON_PLAYLIST_CHANGED`, which the
            // provider treats as a refresh trigger. Position is preserved
            // because the URI is unchanged; the same-index guard in
            // `onMediaItemTransition` prevents the cascade back into
            // `fileLoaded` / session_reporter.
            val p = player ?: return@post
            if (p.mediaItemCount == 0) return@post
            if (lastReportedIndex < 0) return@post
            val current = p.currentMediaItem ?: return@post
            try {
                p.replaceMediaItem(p.currentMediaItemIndex, current)
            } catch (e: Exception) {
                Log.w(TAG, "accent nudge replaceMediaItem failed", e)
            }
        }
    }

    @Command
    fun nowPlayingClear(invoke: Invoke) {
        runOnMain {
            // Bump the cover generation so an in-flight background
            // artwork load doesn't overwrite the cleared state when it
            // finally lands on main.
            coverRequestGen++
            lastTitle = null
            lastArtist = null
            lastAlbum = null
            lastCoverUrl = null
            lastCoverBytes = null
            invoke.resolve()
        }
    }

    private fun loadArtworkBytes(coverUrl: String?): ByteArray? {
        if (coverUrl.isNullOrEmpty()) return null
        return try {
            val uri = Uri.parse(coverUrl)
            val path = if (uri.scheme == "file") uri.path else coverUrl
            if (path.isNullOrEmpty()) return null
            val file = File(path)
            if (!file.exists()) return null
            val appDataDir = activity.filesDir.canonicalPath
            if (!file.canonicalPath.startsWith(appDataDir)) return null
            file.readBytes()
        } catch (e: Exception) {
            Log.w(TAG, "loadArtworkBytes failed for $coverUrl", e)
            null
        }
    }

    private var pollerActive = true

    private val positionPoller = object : Runnable {
        override fun run() {
            if (!pollerActive) return
            try {
                val p = player
                if (p != null && p.isPlaying) {
                    val seconds = p.currentPosition / 1000.0
                    trigger("mpvPositionChange", JSObject().put("position", seconds))
                }
            } catch (e: Throwable) {
                Log.e(TAG, "poller: threw", e)
            } finally {
                if (pollerActive) mainHandler.postDelayed(this, POSITION_POLL_MS)
            }
        }
    }

    private inner class PlayerListener : Player.Listener {
        override fun onIsPlayingChanged(isPlaying: Boolean) {
            trigger("mpvPauseChange", JSObject().put("paused", !isPlaying))
            if (isPlaying) ensureForegroundServiceStarted()
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
            // Only treat genuine index changes as fresh tracks. A
            // metadata-only `replaceMediaItem(currentIndex, …)` from
            // `nowPlayingUpdate` ALSO fires this callback (with reason
            // `MEDIA_ITEM_TRANSITION_REASON_PLAYLIST_CHANGED`), and
            // resetting the latch on those would re-emit `mpvFileLoaded`,
            // kicking session_reporter back into nowPlayingUpdate.
            if (idx != lastReportedIndex) {
                lastReportedIndex = idx
                trigger("mpvPlaylistPosChange", JSObject().put("index", idx.toLong()))
                lastReportedDuration = C.TIME_UNSET
                fileLoadedEmitted = false
                // A prebuffered auto-advance doesn't re-enter `STATE_READY`
                // (the player was already ready), so relying on
                // `onPlaybackStateChanged` to emit `mpvDurationChange` +
                // `mpvFileLoaded` misses every natural track change. The
                // new track's duration is already known at this point
                // (it was prepared as part of the queue), so emit
                // straight from the transition. STATE_READY still covers
                // the "needed to buffer first" case via the same dedupe
                // guards.
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
        pollerActive = false
        mainHandler.removeCallbacks(positionPoller)

        // Tear the session down before the player so the foreground
        // service stops cleanly: release frees the session-level audio
        // focus and the service's `onTaskRemoved` hook stops itself
        // once `player == null || mediaItemCount == 0`.
        MpvForegroundService.detachSession()
        try {
            activity.stopService(Intent(activity, MpvForegroundService::class.java))
        } catch (e: Exception) {
            Log.w(TAG, "stopService failed", e)
        }
        mediaSession?.release()
        mediaSession = null

        val p = player
        player = null
        p?.removeListener(playerListener)
        p?.release()

        ioHandler.looper.quitSafely()
    }

    // Stubs for iOS-specific commands that the Rust IPC surface declares
    // in `mobile.rs`. These are gated to `cfg(target_os = "ios")` on the
    // Rust call side, so they should never be invoked on Android — but
    // having them present keeps the plugin contract complete and prevents
    // a crash if the gate is ever removed.

    @Command
    fun keychainRead(invoke: Invoke) {
        invoke.resolve(JSObject().put("value", ""))
    }

    @Command
    fun keychainWrite(invoke: Invoke) {
        invoke.resolve(JSObject().put("ok", true))
    }

    @Command
    fun keychainDelete(invoke: Invoke) {
        invoke.resolve(JSObject().put("ok", true))
    }

    @Command
    fun excludeFromBackup(invoke: Invoke) {
        invoke.resolve(JSObject().put("ok", true))
    }

    @Command
    fun dismissKeyboard(invoke: Invoke) {
        invoke.resolve()
    }

    @Command
    fun showNativeSearchBar(invoke: Invoke) {
        invoke.resolve()
    }

    @Command
    fun hideNativeSearchBar(invoke: Invoke) {
        invoke.resolve()
    }

    private fun runOnMain(block: () -> Unit) {
        if (Looper.myLooper() == Looper.getMainLooper()) block()
        else mainHandler.post(block)
    }
}
