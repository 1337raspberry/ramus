package com.ramus.iosbridge

import android.Manifest
import android.app.Activity
import android.app.AlertDialog
import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import android.net.ConnectivityManager
import android.net.LinkProperties
import android.net.Network
import android.net.NetworkCapabilities
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
import androidx.media3.exoplayer.DefaultLoadControl
import androidx.media3.exoplayer.ExoPlayer
import androidx.media3.session.MediaSession
import app.tauri.annotation.Command
import app.tauri.annotation.InvokeArg
import app.tauri.annotation.TauriPlugin
import app.tauri.plugin.Invoke
import app.tauri.plugin.JSArray
import app.tauri.plugin.JSObject
import app.tauri.plugin.Plugin
import android.media.audiofx.Equalizer
import java.io.File
import java.io.FileOutputStream
import java.net.HttpURLConnection
import java.net.URL

private const val TAG = "MpvBridge"
private const val POSITION_POLL_MS = 500L
private const val POST_NOTIFICATIONS_REQUEST_CODE = 1001
// Debug toggle: when false, the MediaSession + foreground service are skipped
// entirely. Lets us A/B whether MediaSession is the cause of a playback issue
// without rebuilding. Flip via `adb shell setprop debug.ramus.media_session 0`
// before launching, then back to anything else (or unset) to re-enable.
private const val MEDIA_SESSION_PROP = "debug.ramus.media_session"
private val GAIN_REGEX = Regex("""g=([-\d.]+)""")
// Plex chunked-Opus transcode endpoint. Used to recognise URLs that need
// the pre-download dance (see `mpvLoadFile` for the why).
private const val TRANSCODE_URL_MARKER = "/transcode/universal/start"
private const val TRANSCODE_BUFFER_DIR_NAME = "transcode-buffer"
private const val TRANSCODE_CONNECT_TIMEOUT_MS = 15_000
private const val TRANSCODE_READ_TIMEOUT_MS = 30_000

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
    // Set by the transcode pre-download path before calling `player.stop()`
    // to silence the previous track. `stop()` transitions ExoPlayer to
    // `STATE_IDLE`, which would normally fire `mpvIdleActive` and cause
    // Rust to flip status=Stopped + drop current_track — wiping the
    // synthetic `playback-state` event we just used to hydrate the UI
    // for the new track. The flag swallows that one IDLE notification;
    // any subsequent `stop()` (e.g. an actual playback end) fires
    // through normally.
    private var suppressNextIdleEvent = false

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
    private var equalizer: Equalizer? = null
    // Cache of the most recent mpvSetAudioFilters payload. The system
    // Equalizer can't attach until ExoPlayer reports a real
    // audioSessionId (which only happens once audio rendering starts),
    // so any user-initiated EQ change before that point lands here and
    // is replayed in onAudioSessionIdChanged.
    private var pendingAudioFilters: String? = null

    // ConnectivityManager.NetworkCallback drives the cellular signal that
    // feeds `should_transcode` on the Rust side. iOS gets the same data
    // via NWPathMonitor in MpvBridgePlugin.swift; both emit identical
    // `networkPathChange` events so `mpv_mobile.rs::register_network_listener`
    // doesn't care which platform it's on. Snapshot is locked because
    // `getNetworkInfo` reads it from whichever thread Tauri's IPC dispatch
    // picks, while `handleCaps` writes it on the binder thread the callback
    // fires on.
    private var connectivityManager: ConnectivityManager? = null
    private var networkCallback: ConnectivityManager.NetworkCallback? = null
    private var lastNetworkSnapshot: JSObject = JSObject().put("satisfied", false)
    private val snapshotLock = Object()

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
                // Buffer the entire track into RAM, mirroring mpv's
                // `demuxer-readahead-secs=1200` / `demuxer-max-bytes=2GiB`
                // on desktop and iOS. Plex's chunked transcode endpoint
                // (`/audio/:/transcode/universal/start`) responds with
                // `Accept-Ranges: none`, so a `seekTo` that lands outside
                // the buffered range closes the connection and re-GETs the
                // URL — Plex generates a fresh transcode from byte 0 and
                // the player snaps to 0:00. Slurping the whole stream
                // (~3-5 MB for an Opus track) up-front sidesteps that:
                // the network GET completes in seconds, the connection
                // drains, and every subsequent seek is in-RAM. Back-buffer
                // mirrors the forward buffer so backward seeks stay in
                // RAM too. Costs ~5 MB per active player; negligible.
                val loadControl = DefaultLoadControl.Builder()
                    .setBufferDurationsMs(
                        /* minBufferMs = */ 600_000,
                        /* maxBufferMs = */ 600_000,
                        /* bufferForPlaybackMs = */ 1_000,
                        /* bufferForPlaybackAfterRebufferMs = */ 5_000,
                    )
                    .setBackBuffer(
                        /* backBufferDurationMs = */ 600_000,
                        /* retainBackBufferFromKeyframe = */ true,
                    )
                    .setTargetBufferBytes(50_000_000)
                    .setPrioritizeTimeOverSizeThresholds(false)
                    .build()
                val p = ExoPlayer.Builder(activity.applicationContext)
                    // `true` here lets ExoPlayer manage audio focus + ducking
                    // automatically (request on play, release on pause/stop).
                    .setAudioAttributes(audioAttrs, /* handleAudioFocus = */ true)
                    .setHandleAudioBecomingNoisy(true)
                    .setLoadControl(loadControl)
                    .build()
                    .apply {
                        addListener(playerListener)
                    }
                player = p
                mainHandler.post(positionPoller)

                // Drop any leftover `.part`/full files from a previous
                // process (crash, force-stop, OS kill). The dir lives
                // under cacheDir so the OS can also clear it under
                // storage pressure; this just guarantees a clean start.
                try {
                    transcodeBufferDir.listFiles()?.forEach { it.delete() }
                } catch (e: Throwable) {
                    Log.w(TAG, "failed to clear transcode buffer dir", e)
                }

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

                // Equalizer attach is deferred to onAudioSessionIdChanged.
                // Constructing it now would bind to audioSessionId == 0
                // (AUDIO_SESSION_ID_UNSET, since prepare() hasn't run),
                // which the AudioFlinger treats as "the global output
                // mix" — meaning EQ adjustments would affect every app's
                // audio on the device, not just Ramus.

                ensurePostNotificationsPermission()
                startNetworkMonitor()
                Log.i(TAG, "ExoPlayer initialised (mediaSession=${mediaSession != null})")
            }
            invoke.resolve()
        }
    }

    /// Register a ConnectivityManager.NetworkCallback that emits the same
    /// `networkPathChange` event the iOS NWPathMonitor handler sends, plus
    /// caches a snapshot for the synchronous `getNetworkInfo` reader. The
    /// Rust side (`mpv_mobile.rs`) is platform-agnostic — both platforms
    /// flow into the same `register_network_listener` path.
    private fun startNetworkMonitor() {
        if (networkCallback != null) return
        val cm = activity.getSystemService(Context.CONNECTIVITY_SERVICE) as? ConnectivityManager
        if (cm == null) {
            Log.w(TAG, "ConnectivityManager unavailable; cellular detection disabled")
            return
        }
        connectivityManager = cm
        val cb = object : ConnectivityManager.NetworkCallback() {
            override fun onCapabilitiesChanged(network: Network, caps: NetworkCapabilities) {
                handleCaps(network, caps, satisfied = true)
            }
            override fun onLost(network: Network) {
                handleCaps(network, null, satisfied = false)
            }
        }
        try {
            cm.registerDefaultNetworkCallback(cb)
            networkCallback = cb
            Log.i(TAG, "ConnectivityManager NetworkCallback registered")
        } catch (e: Throwable) {
            // SecurityException if ACCESS_NETWORK_STATE is missing from the
            // merged manifest, or RuntimeException on some old vendors.
            Log.w(TAG, "registerDefaultNetworkCallback failed", e)
        }
    }

    private fun handleCaps(network: Network, caps: NetworkCapabilities?, satisfied: Boolean) {
        // Map active transports to the same string vocabulary Swift emits
        // ("wifi" / "cellular" / "wired" / "loopback" / "other" / "none").
        // Match in priority order — a default cellular network with a
        // bonded wifi STA could plausibly report both transports.
        val type = when {
            !satisfied || caps == null -> "none"
            caps.hasTransport(NetworkCapabilities.TRANSPORT_WIFI) -> "wifi"
            caps.hasTransport(NetworkCapabilities.TRANSPORT_CELLULAR) -> "cellular"
            caps.hasTransport(NetworkCapabilities.TRANSPORT_ETHERNET) -> "wired"
            else -> "other"
        }
        // Android exposes one bound interface name per Network, not iOS's
        // sorted list of all available interfaces. Good enough for the
        // `HashSet<String>` diff in `ConnectionMonitor::handle_path_update`
        // — transport changes (wlan0 → rmnet0) flip the set; same-iface
        // hotspot drops won't, which we accept for v1.
        val ifaces = JSArray()
        try {
            val lp: LinkProperties? = connectivityManager?.getLinkProperties(network)
            lp?.interfaceName?.let { ifaces.put(it) }
        } catch (_: Throwable) {}

        val isExpensive = caps?.hasCapability(NetworkCapabilities.NET_CAPABILITY_NOT_METERED) == false
        val payload = JSObject()
            .put("interfaces", ifaces)
            .put("type", type)
            .put("isExpensive", isExpensive)
            .put("isConstrained", false)
            .put("satisfied", satisfied)

        synchronized(snapshotLock) { lastNetworkSnapshot = payload }
        // Hop to main before triggering — keeps every event emission on
        // the same thread the rest of the plugin uses, matches Swift's
        // `DispatchQueue.main.async { trigger(...) }` pattern.
        mainHandler.post { trigger("networkPathChange", payload) }
    }

    @Command
    fun getNetworkInfo(invoke: Invoke) {
        val snapshot = synchronized(snapshotLock) { lastNetworkSnapshot }
        invoke.resolve(snapshot)
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

        // Pre-download the chunked-Opus transcode response to a local
        // file before handing it to ExoPlayer. Background: Plex's
        // `/audio/:/transcode/universal/start` endpoint serves a chunked
        // stream with `Accept-Ranges: none`, so OggExtractor can't run
        // its seek-to-end probe to recover trailing granule positions,
        // never builds a real SeekMap, and the window stays
        // `isSeekable=false`. Every `seekTo()` then short-circuits to
        // a player reset → close → re-GET → Plex regenerates from
        // offset 0 → audio snaps to 0:00. Local file = known length +
        // proper SeekMap = real seeks.
        //
        // Threading: `@Command` runs on the Android main thread, which
        // forbids network IO. The download runs on `ioHandler` (off-main),
        // and we defer `invoke.resolve()` until the download finishes
        // and `setMediaItem` lands. The Rust caller (`mpv_load_file` in
        // `mpv_android.rs`) blocks on its IPC future until we resolve,
        // so the next queue-append doesn't race ahead — the Rust thread
        // running `load_queue` naturally serialises behind us. Only
        // gated on the `replace` mode (the current track of a fresh
        // play) — appended tracks haven't started playing yet and will
        // be swapped to file:// by the Rust prefetch worker before they
        // would.
        if (args.mode == "replace" && isTranscodeUrl(args.url)) {
            trigger("mpvBufferingChange", JSObject().put("buffering", true))
            // Fire a synthetic `mpvPauseChange(paused=false)` BEFORE the
            // download starts so the frontend gets a `playback-state`
            // event immediately and can hydrate the album view
            // (background, accent, waveform, queue, mini-player) instead
            // of staring at a frozen UI for the 2–4s pre-download
            // window. The Rust side already set `state.status = Playing`
            // and `state.current_track` inside `load_queue` BEFORE this
            // IPC arrived, so:
            //   - `handle_pause_change(false)` is a no-op for the player
            //     state itself (status is Playing, not Paused).
            //   - The listener still emits `playback-state` after the
            //     no-op handler call (`lib.rs:274`), carrying the
            //     already-populated `current_track` + `queue_index`.
            //   - Frontend `onPlaybackState` (`playbackStore.ts:151`)
            //     detects the track change and fans out the per-track
            //     loads (waveform, album, palette, lyrics, queue) in
            //     parallel with our download.
            // The IPC stays unresolved until the download completes, so
            // Rust's `load_queue` still blocks before dispatching the
            // appends — they can't race the deferred `setMediaItem`.
            trigger("mpvPauseChange", JSObject().put("paused", false))

            // Stop the previous track immediately. Without this, ExoPlayer
            // keeps playing whatever was already loaded for the full
            // pre-download window — its position poller keeps firing
            // `mpvPositionChange` with the OLD position, and `lib.rs`'s
            // `on_position_change` handler emits each tick paired with
            // the NEW track's `inner.duration` (which `load_queue` set
            // before this IPC arrived). The frontend renders that as
            // "still playing, advancing through the new track" — exactly
            // the scrub-bar regression the user reported. `p.stop()`
            // halts audio + ends the poller (isPlaying = false). The
            // accompanying `STATE_IDLE` transition is squashed via
            // `suppressNextIdleEvent` so Rust doesn't flip status=Stopped
            // and undo the synthetic state event above.
            runOnMain {
                val p = player
                if (p != null) {
                    suppressNextIdleEvent = true
                    p.stop()
                }
            }

            ioHandler.post {
                val file = downloadTranscodeToTempFile(args.url)
                mainHandler.post {
                    val effectiveUrl = if (file != null) {
                        Log.i(TAG, "transcode pre-download complete (${file.length()} bytes)")
                        Uri.fromFile(file).toString()
                    } else {
                        // Better to hand ExoPlayer the live URL and lose
                        // seek than to refuse playback. The user will
                        // still hear the track; only the seek bar
                        // misbehaves.
                        Log.w(TAG, "transcode pre-download failed; falling back to live URL")
                        args.url
                    }
                    performLoad(invoke, args, effectiveUrl)
                }
            }
        } else {
            runOnMain { performLoad(invoke, args, args.url) }
        }
    }

    // Shared transcode-pre-download dance for paths that already have
    // a playlist entry to swap in place: gapless auto-advance into a
    // transcode URL (via `onMediaItemTransition`), or a manual skip
    // to one (via `mpvPlaylistPlayIndex`). The `mpvLoadFile("replace")`
    // path is structured slightly differently because it owns its own
    // IPC and runs setMediaItem at the end, so it doesn't call this.
    //
    // Halts ExoPlayer immediately (the live URL would otherwise start
    // streaming and the seek bar would be broken from the first
    // second), fires the buffering signal, downloads, then swaps the
    // playlist entry to `file://` and resumes from position 0.
    private fun preDownloadAndSwapAt(p: ExoPlayer, idx: Int, url: String, onDone: () -> Unit = {}) {
        trigger("mpvBufferingChange", JSObject().put("buffering", true))
        suppressNextIdleEvent = true
        p.stop()
        ioHandler.post {
            val file = downloadTranscodeToTempFile(url)
            mainHandler.post {
                val effectiveUri = if (file != null) {
                    Log.i(TAG, "transcode pre-download complete (${file.length()} bytes) for idx=$idx")
                    Uri.fromFile(file).toString()
                } else {
                    Log.w(TAG, "transcode pre-download failed; resuming with live URL at idx=$idx")
                    url
                }
                suppressNextIdleEvent = false
                try {
                    p.replaceMediaItem(idx, MediaItem.fromUri(Uri.parse(effectiveUri)))
                    p.seekTo(idx, 0L)
                    p.prepare()
                    p.play()
                } catch (e: Exception) {
                    Log.w(TAG, "preDownloadAndSwapAt: apply failed", e)
                }
                trigger("mpvBufferingChange", JSObject().put("buffering", false))
                onDone()
            }
        }
    }

    private fun performLoad(invoke: Invoke, args: LoadFileArgs, url: String) {
        val p = player ?: run {
            invoke.reject("player not initialised")
            return
        }
        // Belt-and-braces: if the pre-download path set this but the
        // player was already idle (so `stop()` didn't fire a STATE_IDLE
        // event), the flag would still be set. Clearing it here means
        // the next legitimate idle transition fires normally.
        suppressNextIdleEvent = false
        val item = MediaItem.fromUri(Uri.parse(url))
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
        // Always clear the buffering signal at the end of a load.
        // Cheap no-op on the common (non-transcode) path; necessary
        // on the transcode path because we set it true before the
        // download started. Also covers the auto-advance intercept
        // below — same exit point.
        trigger("mpvBufferingChange", JSObject().put("buffering", false))
        invoke.resolve()
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
            // Manual skip into a not-yet-swapped transcode entry hits the
            // same broken-seek behaviour as a fresh `replace` load —
            // intercept here and pre-download the bytes before letting
            // ExoPlayer start streaming.
            val targetUri = p.getMediaItemAt(idx).localConfiguration?.uri?.toString()
            if (targetUri != null && isTranscodeUrl(targetUri)) {
                preDownloadAndSwapAt(p, idx, targetUri) { invoke.resolve() }
            } else {
                p.seekTo(idx, 0L)
                if (p.playbackState == Player.STATE_IDLE) p.prepare()
                p.play()
                invoke.resolve()
            }
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
    fun mpvGetEqConfig(invoke: Invoke) {
        runOnMain {
            val eq = equalizer
            if (eq == null) {
                val fallback = org.json.JSONArray(listOf(31, 62, 125, 250, 500, 1000, 2000, 4000, 8000, 16000))
                invoke.resolve(JSObject()
                    .put("frequencies", fallback)
                    .put("minGain", -12.0)
                    .put("maxGain", 12.0))
                return@runOnMain
            }
            val freqs = org.json.JSONArray()
            for (i in 0 until eq.numberOfBands.toInt()) {
                freqs.put(eq.getCenterFreq(i.toShort()) / 1000)
            }
            val range = eq.bandLevelRange
            invoke.resolve(JSObject()
                .put("frequencies", freqs)
                .put("minGain", range[0].toDouble() / 100.0)
                .put("maxGain", range[1].toDouble() / 100.0))
        }
    }

    @Command
    fun mpvSetAudioFilters(invoke: Invoke) {
        val args = invoke.parseArgs(AudioFiltersArgs::class.java)
        runOnMain {
            // Cache regardless of whether the equalizer is attached yet
            // — attachEqualizer replays this on first audio session.
            pendingAudioFilters = args.value
            equalizer?.let { applyAudioFiltersToEqualizer(it, args.value) }
            invoke.resolve()
        }
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
            // Only handle local files. The Rust side always passes a
            // file:// path from the image cache, but the scheme guard
            // is defensive: an HTTPS Plex art URL would carry an
            // ?X-Plex-Token=... query param, and falling through would
            // both attempt a doomed file read and (worse) leak the
            // token to logcat in the catch arm below.
            if (uri.scheme != "file") {
                Log.w(TAG, "loadArtworkBytes: refusing non-file scheme '${uri.scheme}'")
                return null
            }
            val path = uri.path
            if (path.isNullOrEmpty()) return null
            val file = File(path)
            if (!file.exists()) return null
            // Sandbox boundary is the package data dir, not filesDir/.
            // The Rust image cache lives at <dataDir>/image_cache/, seeded
            // from Tauri's app_data_dir() which resolves to dataDir on
            // Android — using filesDir.canonicalPath here would reject
            // every legitimate art file and the lock-screen widget would
            // render without artwork.
            val appDataDir = activity.dataDir.canonicalPath
            if (!file.canonicalPath.startsWith(appDataDir)) return null
            file.readBytes()
        } catch (e: Exception) {
            // Strip any query string before logging in case a future
            // code path slips a non-file URL past the guard above.
            val safe = coverUrl.substringBefore('?')
            Log.w(TAG, "loadArtworkBytes failed for $safe", e)
            null
        }
    }

    private val transcodeBufferDir: File
        get() = File(activity.cacheDir, TRANSCODE_BUFFER_DIR_NAME).also { it.mkdirs() }

    private fun isTranscodeUrl(url: String): Boolean {
        return try {
            Uri.parse(url).path?.contains(TRANSCODE_URL_MARKER) == true
        } catch (_: Throwable) {
            false
        }
    }

    // Extract the Plex rating-key from a transcode URL via the `session`
    // query param (`<client-id>-<rating-key>`; see `transcode.rs`). Used
    // as the cache filename so repeated plays of the same track within a
    // process reuse the same buffered copy instead of re-downloading.
    private fun extractRatingKey(url: String): String? {
        return try {
            val session = Uri.parse(url).getQueryParameter("session") ?: return null
            val rk = session.substringAfterLast('-', missingDelimiterValue = "")
            rk.takeIf { it.isNotEmpty() && it != session }
        } catch (_: Throwable) {
            null
        }
    }

    // Synchronously download a Plex transcode response to a local file
    // under `cacheDir/transcode-buffer/`. Returns the finalised file on
    // success or null on any error (caller falls back to the live URL).
    //
    // Must not be called from the main thread — blocking HTTP IO would
    // throw `NetworkOnMainThreadException` (and would ANR if the policy
    // was relaxed). Callers post this onto `ioHandler` and post the
    // result back to main.
    //
    // Token hygiene: caught exceptions don't include the URL; raw HTTP
    // errors from `URLConnection` can stringify the request URL (which
    // carries `X-Plex-Token` in the query), so we log only the exception
    // class name. Mirrors `redact_reqwest_err` in `prefetch.rs` on the
    // Rust side.
    private fun downloadTranscodeToTempFile(url: String): File? {
        val key = extractRatingKey(url) ?: ("h" + Integer.toHexString(url.hashCode()))
        val target = File(transcodeBufferDir, "$key.ogg")

        // Already buffered earlier in this process? Reuse it. Cleared on
        // `mpvInit`, so a stale file from a previous session can't leak.
        if (target.exists() && target.length() > 0) {
            return target
        }

        val tmp = File(transcodeBufferDir, "$key.ogg.part")
        var conn: HttpURLConnection? = null
        return try {
            conn = (URL(url).openConnection() as HttpURLConnection).apply {
                requestMethod = "GET"
                connectTimeout = TRANSCODE_CONNECT_TIMEOUT_MS
                readTimeout = TRANSCODE_READ_TIMEOUT_MS
                // Plex headers ride in the URL query string; no extra
                // request headers required.
            }
            conn.connect()
            val code = conn.responseCode
            if (code !in 200..299) {
                Log.w(TAG, "transcode pre-download: HTTP $code")
                return null
            }
            conn.inputStream.use { input ->
                FileOutputStream(tmp).use { output ->
                    input.copyTo(output)
                }
            }
            if (!tmp.renameTo(target)) {
                Log.w(TAG, "transcode pre-download: rename .part → final failed")
                tmp.delete()
                return null
            }
            target
        } catch (e: Throwable) {
            Log.w(TAG, "transcode pre-download exception: ${e.javaClass.simpleName}")
            try { tmp.delete() } catch (_: Throwable) {}
            null
        } finally {
            try { conn?.disconnect() } catch (_: Throwable) {}
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

    // Attach (or re-attach) the system Equalizer to a real audio session
    // id. Releases any previous instance — switching session id with a
    // live attachment leaves the old AudioFlinger binding in place,
    // which in practice means the EQ keeps affecting whichever output
    // mix it was originally bound to. Replays the most recent
    // mpvSetAudioFilters payload so a user-configured EQ doesn't fall
    // off when audio rendering starts for the first time.
    private fun attachEqualizer(audioSessionId: Int) {
        if (audioSessionId == 0) return
        equalizer?.runCatching { release() }
        try {
            val eq = Equalizer(0, audioSessionId).apply { enabled = false }
            equalizer = eq
            Log.i(
                TAG,
                "System Equalizer attached to session $audioSessionId (${eq.numberOfBands} bands)"
            )
            pendingAudioFilters?.let { applyAudioFiltersToEqualizer(eq, it) }
        } catch (e: Exception) {
            equalizer = null
            Log.w(TAG, "System Equalizer unavailable", e)
        }
    }

    private fun applyAudioFiltersToEqualizer(eq: Equalizer, value: String) {
        if (value.isEmpty()) {
            eq.enabled = false
            return
        }
        val gains = GAIN_REGEX.findAll(value)
            .map { it.groupValues[1].toFloatOrNull() ?: 0f }
            .toList()
        val numBands = eq.numberOfBands.toInt()
        val range = eq.bandLevelRange
        for (band in 0 until numBands) {
            val gainDb = if (band < gains.size) gains[band] else 0f
            val level = (gainDb * 100).toInt().coerceIn(range[0].toInt(), range[1].toInt()).toShort()
            eq.setBandLevel(band.toShort(), level)
        }
        eq.enabled = true
    }

    private inner class PlayerListener : Player.Listener {
        override fun onPlayWhenReadyChanged(playWhenReady: Boolean, reason: Int) {
            // Drive `mpvPauseChange` off `playWhenReady` rather than
            // `isPlaying`. `isPlaying` is a derived flag that flips false
            // whenever the player enters `STATE_BUFFERING` — every
            // `seekTo` call briefly does that even when the seek target
            // is already buffered (the audio renderer needs a moment to
            // re-prime at the new sample position), so wiring the pause
            // event to `isPlaying` produced a one-frame "paused" flicker
            // on every scrub. `playWhenReady` only changes on explicit
            // `play()` / `pause()` calls, matching mpv's `pause` property
            // semantics on desktop/iOS where the analogous event only
            // fires on user-driven pause intent.
            trigger("mpvPauseChange", JSObject().put("paused", !playWhenReady))
        }

        override fun onIsPlayingChanged(isPlaying: Boolean) {
            // Foreground service promotion still keys off real playback
            // (not intent) — starting the service before audio is actually
            // flowing risks `ForegroundServiceDidNotStartInTimeException`
            // because Media3 only attaches its MediaStyle notification
            // once `isPlaying` flips true.
            if (isPlaying) ensureForegroundServiceStarted()
        }

        override fun onAudioSessionIdChanged(audioSessionId: Int) {
            attachEqualizer(audioSessionId)
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
                    if (suppressNextIdleEvent) {
                        suppressNextIdleEvent = false
                    } else {
                        trigger("mpvIdleActive", JSObject())
                    }
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

                // Auto-advance into a not-yet-cached transcode URL: stop
                // the live stream before any audio plays, pre-download the
                // bytes, then resume from the local file. AUTO is the only
                // reason that bypasses our @Command entry points (which
                // already check the URL upfront) — SEEK comes from
                // `mpvPlaylistPlayIndex` and PLAYLIST_CHANGED is fired by
                // our own `replaceMediaItem` inside `preDownloadAndSwapAt`
                // (which carries a `file://` URI, so the gate is false on
                // re-entry).
                if (reason == Player.MEDIA_ITEM_TRANSITION_REASON_AUTO) {
                    val uri = mediaItem?.localConfiguration?.uri?.toString()
                    if (uri != null && isTranscodeUrl(uri)) {
                        preDownloadAndSwapAt(p, idx, uri)
                    }
                }
            }
        }

        override fun onPlayerError(error: PlaybackException) {
            // Don't pass the Throwable to Log.e — its cause chain typically
            // wraps an HttpDataSourceException whose toString() embeds the
            // failed request URI, and Plex direct-play URLs carry the
            // X-Plex-Token query parameter. Mirrors the redaction applied
            // to the Rust mpv error log in player.rs.
            Log.e(TAG, "ExoPlayer error: ${error.errorCodeName} (${error.errorCode})")
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

        equalizer?.release()
        equalizer = null

        // Cancel the network monitor before nulling the manager — letting
        // the kernel callback live on after release leaks the subscription.
        networkCallback?.let { cb ->
            try {
                connectivityManager?.unregisterNetworkCallback(cb)
            } catch (e: Throwable) {
                Log.w(TAG, "unregisterNetworkCallback failed", e)
            }
        }
        networkCallback = null
        connectivityManager = null

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
