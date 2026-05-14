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
import androidx.media3.common.C
import androidx.media3.common.MediaItem
import androidx.media3.common.MediaMetadata
import androidx.media3.common.Player
import androidx.media3.common.util.UnstableApi
import androidx.media3.session.MediaSession
import app.tauri.annotation.Command
import app.tauri.annotation.InvokeArg
import app.tauri.annotation.TauriPlugin
import app.tauri.plugin.Invoke
import app.tauri.plugin.JSArray
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
 * Tauri plugin that owns the libmpv-backed audio player on Android.
 *
 * Playback runs on libmpv via `LibmpvSimplePlayer`, a Media3
 * `SimpleBasePlayer` wrapper around `dev.jdtech.mpv.MPVLib`. The
 * `MediaSession` + `MediaSessionService` are unchanged from the previous
 * ExoPlayer build — they accept any `Player` implementation, so the
 * lock-screen / Bluetooth / Android Auto controls keep working.
 *
 * All Player access is marshalled to the main thread —
 * `SimpleBasePlayer` requires it, and the libmpv event-loop callbacks
 * are already main-thread-marshalled inside `LibmpvSimplePlayer`.
 */
@UnstableApi
@TauriPlugin
class MpvBridgePlugin(private val activity: Activity) : Plugin(activity) {
    private val mainHandler = Handler(Looper.getMainLooper())
    private var player: LibmpvSimplePlayer? = null
    private val playerListener = PlayerListener()
    private var lastReportedDuration: Long = C.TIME_UNSET
    private var lastReportedIndex: Int = -1
    // Tracks the URI of the last MediaItem we emitted `mpvFileLoaded` for.
    // Index alone isn't enough: a same-position queue replace (queue=[A]
    // → queue=[B] with B at idx 0) is a real file change but doesn't trip
    // the idx-changed branch in `onMediaItemTransition`, so the latch
    // would stay set and suppress the event for the new track.
    private var lastReportedUri: Uri? = null
    // mpv emits a real `MPV_EVENT_FILE_LOADED` per file; the latch
    // dedupes against `STATE_READY` re-entries on seek + cache-recovery
    // that don't represent a fresh track load.
    private var fileLoadedEmitted = false

    private var mediaSession: MediaSession? = null
    // Foreground service is started on the FIRST `isPlaying=true` transition
    // (not in mpvInit) so Media3's `DefaultMediaNotificationProvider` can
    // call `startForeground` with a real MediaStyle notification within the
    // 5s `startForegroundService` window.
    private var foregroundServiceStarted = false
    // Cache the last-pushed metadata so back-to-back `nowPlayingUpdate`
    // calls with the same payload skip the round-trip and the artwork
    // bytes are only re-read from disk when the cover URL actually changes.
    private var lastTitle: String? = null
    private var lastArtist: String? = null
    private var lastAlbum: String? = null
    private var lastCoverUrl: String? = null
    private var lastCoverBytes: ByteArray? = null
    private var coverRequestGen: Int = 0
    private val ioHandler = Handler(android.os.HandlerThread("MpvBridgeIO").apply { start() }.looper)

    // ConnectivityManager.NetworkCallback drives the cellular signal that
    // feeds `should_transcode` on the Rust side. iOS gets the same data
    // via NWPathMonitor in MpvBridgePlugin.swift; both emit identical
    // `networkPathChange` events so `mpv_mobile.rs::register_network_listener`
    // doesn't care which platform it's on.
    private var connectivityManager: ConnectivityManager? = null
    private var networkCallback: ConnectivityManager.NetworkCallback? = null
    private var lastNetworkSnapshot: JSObject = JSObject().put("satisfied", false)
    private val snapshotLock = Object()

    @Command
    fun mpvInit(invoke: Invoke) {
        // Always post (never inline-execute) — Tauri can dispatch this
        // command on a background IPC thread, and an inline path here
        // followed by a queued path for the next command would let
        // `mpvLoadFile` race ahead of `mpvInit`. Posting unconditionally
        // serialises init + first load on the main looper FIFO.
        mainHandler.post {
            if (player == null) {
                val p = try {
                    LibmpvSimplePlayer(activity.applicationContext)
                } catch (e: Throwable) {
                    Log.e(TAG, "LibmpvSimplePlayer construction failed", e)
                    invoke.reject("libmpv init failed: ${e.message}")
                    return@post
                }
                p.addListener(playerListener)
                player = p
                mainHandler.post(positionPoller)

                if (mediaSessionEnabled()) {
                    try {
                        val session = MediaSession.Builder(activity.applicationContext, p).build()
                        mediaSession = session
                        MpvForegroundService.attachSession(session)
                    } catch (e: Throwable) {
                        Log.e(TAG, "MediaSession build failed; falling back to plain player", e)
                    }
                } else {
                    Log.w(TAG, "MediaSession disabled via $MEDIA_SESSION_PROP")
                }

                ensurePostNotificationsPermission()
                startNetworkMonitor()
                Log.i(TAG, "libmpv player initialised (mediaSession=${mediaSession != null})")
            }
            invoke.resolve()
        }
    }

    /// Register a ConnectivityManager.NetworkCallback that emits the same
    /// `networkPathChange` event the iOS NWPathMonitor handler sends, plus
    /// caches a snapshot for the synchronous `getNetworkInfo` reader.
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
            // SecurityException if ACCESS_NETWORK_STATE is missing, or
            // RuntimeException on some old vendors.
            Log.w(TAG, "registerDefaultNetworkCallback failed", e)
        }
    }

    private fun handleCaps(network: Network, caps: NetworkCapabilities?, satisfied: Boolean) {
        val type = when {
            !satisfied || caps == null -> "none"
            caps.hasTransport(NetworkCapabilities.TRANSPORT_WIFI) -> "wifi"
            caps.hasTransport(NetworkCapabilities.TRANSPORT_CELLULAR) -> "cellular"
            caps.hasTransport(NetworkCapabilities.TRANSPORT_ETHERNET) -> "wired"
            else -> "other"
        }
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
        // libmpv configures its own AudioTrack via `ao=audiotrack`; audio
        // focus is delegated to Media3 (the MediaSession requests focus
        // implicitly). This remains a no-op so the cross-platform init
        // order (`mpv_init` → `init_audio`) keeps holding on Android.
        invoke.resolve()
    }

    @Command
    fun mpvLoadFile(invoke: Invoke) {
        val args = invoke.parseArgs(LoadFileArgs::class.java)
        runOnMain {
            val p = player ?: return@runOnMain invoke.reject("player not initialised")
            // No `setMediaId(args.url)`: Plex URLs embed `?X-Plex-Token=…`
            // (direct play) or `X-Plex-Headers=…` (transcode), and
            // `MediaItem.mediaId` is one of the few fields MediaSession
            // serialises across the AIDL boundary to bound MediaController
            // clients. Apps with `BIND_NOTIFICATION_LISTENER_SERVICE` and
            // `adb dumpsys media_session` would otherwise pluck the token
            // out of the published session. `LibmpvSimplePlayer.getState`
            // falls back to `item.hashCode().toString()` for the playlist
            // uid so the empty mediaId is harmless.
            val item = MediaItem.Builder()
                .setUri(Uri.parse(args.url))
                .build()
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
                    // mpv's native `append-play` flag: append, and start
                    // playback only if nothing is currently playing. The
                    // Media3 Player interface has no equivalent —
                    // `addMediaItem` always maps to bare append, and
                    // `play()` on an idle SimpleBasePlayer just flips
                    // `pause` without loading anything. Drive the start
                    // explicitly via seekTo on the appended index, which
                    // `LibmpvSimplePlayer.handleSeek` translates into
                    // `playlist-play-index`.
                    val wasIdle = p.playbackState == Player.STATE_IDLE
                    val newIdx = p.mediaItemCount
                    p.addMediaItem(item)
                    if (wasIdle) {
                        p.seekTo(newIdx, 0L)
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
            val item = MediaItem.Builder()
                .setUri(Uri.parse(args.url))
                .build()
            p.addMediaItem(args.index.toInt(), item)
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
            // mpv expects 0..100; we drive libmpv's `volume` property
            // directly via the plugin helper so we don't have to round-trip
            // through SimpleBasePlayer's 0..1 normalisation.
            p.setMpvVolume(args.volume)
            invoke.resolve()
        }
    }

    @Command
    fun mpvGetVolume(invoke: Invoke) {
        runOnMain {
            val p = player
            val volume = p?.getMpvVolume() ?: 100.0
            invoke.resolve(JSObject().put("volume", volume))
        }
    }

    @Command
    fun mpvGetEqConfig(invoke: Invoke) {
        // 10-band lavfi schema, identical to desktop and iOS. libmpv's
        // `af=equalizer=...` filter chain consumes whatever frequencies
        // we declare here, so the bands are entirely UI-side.
        val freqs = org.json.JSONArray(listOf(50, 100, 200, 400, 800, 1600, 3200, 6400, 12800, 16000))
        invoke.resolve(
            JSObject()
                .put("frequencies", freqs)
                .put("minGain", -12.0)
                .put("maxGain", 12.0),
        )
    }

    @Command
    fun mpvSetAudioFilters(invoke: Invoke) {
        val args = invoke.parseArgs(AudioFiltersArgs::class.java)
        runOnMain {
            player?.setAudioFilters(args.value)
            invoke.resolve()
        }
    }

    @Command
    fun mpvGetDemuxerCacheTime(invoke: Invoke) {
        runOnMain {
            // Negative sentinel matches the iOS bridge — Rust's
            // `prefetch::wait_for_live_drain` treats values < 0 as
            // "not yet available" and falls through to the ceiling.
            val value = player?.demuxerCacheTimeSeconds() ?: -1.0
            invoke.resolve(JSObject().put("value", value))
        }
    }

    @Command
    fun mpvStop(invoke: Invoke) {
        runOnMain {
            val p = player ?: return@runOnMain invoke.resolve()
            p.stop()
            p.clearMediaItems()
            lastReportedIndex = -1
            lastReportedUri = null
            lastReportedDuration = C.TIME_UNSET
            fileLoadedEmitted = false
            foregroundServiceStarted = false
            invoke.resolve()
        }
    }

    @Command
    fun nowPlayingUpdate(invoke: Invoke) {
        val args = invoke.parseArgs(NowPlayingArgs::class.java)
        invoke.resolve()
        mainHandler.post {
            val titleChanged = args.title != lastTitle
            val artistChanged = args.artist != lastArtist
            val albumChanged = args.album != lastAlbum
            val coverChanged = args.coverUrl != lastCoverUrl
            if (!titleChanged && !artistChanged && !albumChanged && !coverChanged) return@post

            lastTitle = args.title
            lastArtist = args.artist
            lastAlbum = args.album

            if (mediaSession == null) {
                Log.w(TAG, "nowPlayingUpdate: mediaSession null, skipping apply")
                return@post
            }

            if (coverChanged) {
                val coverUrl = args.coverUrl
                val myGen = ++coverRequestGen
                ioHandler.post {
                    val bytes = loadArtworkBytes(coverUrl)
                    mainHandler.post {
                        if (myGen != coverRequestGen) return@post
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
        val p = player ?: run {
            Log.w(TAG, "applyMetadata: player null")
            return
        }
        if (p.mediaItemCount == 0) {
            Log.w(TAG, "applyMetadata: mediaItemCount=0 (player not loaded yet)")
            return
        }
        val builder = MediaMetadata.Builder()
            .setTitle(lastTitle)
            .setArtist(lastArtist)
            .setAlbumTitle(lastAlbum)
        lastCoverBytes?.let {
            // `setArtworkData` (vs `setArtworkUri`) sidesteps the file://
            // permission issue (androidx/media#2331) where system processes
            // can't open app-private files.
            builder.setArtworkData(it, MediaMetadata.PICTURE_TYPE_FRONT_COVER)
        }
        p.replaceCurrentMediaItemMetadata(builder.build())
    }

    @Command
    fun setMediaAccent(invoke: Invoke) {
        val args = invoke.parseArgs(MediaAccentArgs::class.java)
        invoke.resolve()
        mainHandler.post {
            val r = args.r.coerceIn(0, 255)
            val g = args.g.coerceIn(0, 255)
            val b = args.b.coerceIn(0, 255)
            val argb = (0xFF shl 24) or (r shl 16) or (g shl 8) or b
            MpvForegroundService.setAccent(argb)

            // Nudge the MediaSession to rebuild the notification with the
            // new accent. Re-pushing the current MediaItem's metadata
            // triggers the provider's refresh path without disturbing
            // playback. Skipped if we haven't reported a track yet.
            val p = player ?: return@post
            if (p.mediaItemCount == 0) return@post
            if (lastReportedIndex < 0) return@post
            applyMetadata()
        }
    }

    @Command
    fun nowPlayingClear(invoke: Invoke) {
        runOnMain {
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
            // file:// path from the image cache; the scheme guard is
            // defensive against a future code path that slips an HTTPS
            // URL (carrying `?X-Plex-Token=...`) through to logcat below.
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
            // every legitimate art file.
            val appDataDir = activity.dataDir.canonicalPath
            if (!file.canonicalPath.startsWith(appDataDir)) return null
            file.readBytes()
        } catch (e: Exception) {
            val safe = coverUrl.substringBefore('?')
            Log.w(TAG, "loadArtworkBytes failed for $safe", e)
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
        override fun onPlayWhenReadyChanged(playWhenReady: Boolean, reason: Int) {
            // Drive `mpvPauseChange` off `playWhenReady` rather than
            // `isPlaying`. `isPlaying` flips false on every transient
            // `STATE_BUFFERING`, which would echo back as a one-frame
            // "paused" flicker on every scrub. `playWhenReady` only
            // changes on explicit `play()` / `pause()` — matches mpv's
            // `pause` property semantics on desktop/iOS.
            trigger("mpvPauseChange", JSObject().put("paused", !playWhenReady))
        }

        override fun onIsPlayingChanged(isPlaying: Boolean) {
            // Foreground service promotion still keys off real playback
            // (not intent) — starting the service before audio is actually
            // flowing risks `ForegroundServiceDidNotStartInTimeException`.
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
                Player.STATE_BUFFERING -> {
                    // No platform-specific signal — desktop/iOS infer
                    // buffering from playback state, so Android does too.
                }
                Player.STATE_ENDED -> {
                    trigger("mpvFileEnded", JSObject().put("reason", "eof"))
                }
                Player.STATE_IDLE -> {
                    trigger("mpvIdleActive", JSObject())
                }
            }
        }

        override fun onMediaItemTransition(mediaItem: MediaItem?, reason: Int) {
            val p = player ?: return
            val idx = p.currentMediaItemIndex
            val uri = mediaItem?.localConfiguration?.uri
            val indexChanged = idx != lastReportedIndex
            val uriChanged = uri != lastReportedUri
            // `replaceCurrentMediaItemMetadata` mutates the queue and
            // calls `invalidateState`, which can fire this callback even
            // though the file hasn't changed. URI-based dedupe lets a
            // metadata-only refresh pass through silently while still
            // catching same-index file swaps.
            if (!indexChanged && !uriChanged) return
            lastReportedIndex = idx
            lastReportedUri = uri
            if (indexChanged) {
                trigger("mpvPlaylistPosChange", JSObject().put("index", idx.toLong()))
            }
            lastReportedDuration = C.TIME_UNSET
            fileLoadedEmitted = false
            // A prebuffered auto-advance doesn't re-enter `STATE_READY`,
            // so emit duration + fileLoaded straight from the transition
            // for natural advance to update the lock-screen widget.
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

        override fun onPlayerError(error: androidx.media3.common.PlaybackException) {
            // Don't pass the Throwable to Log.e — its cause chain can wrap
            // an HTTP error whose toString() embeds the failed URI, and
            // Plex direct-play URLs carry `X-Plex-Token` in the query.
            Log.e(TAG, "Player error: ${error.errorCodeName} (${error.errorCode})")
            trigger("mpvFileEnded", JSObject().put("reason", "error"))
        }
    }

    override fun onDestroy() {
        pollerActive = false
        mainHandler.removeCallbacks(positionPoller)

        // Tear the session down before the player so the foreground
        // service stops cleanly.
        MpvForegroundService.detachSession()
        try {
            activity.stopService(Intent(activity, MpvForegroundService::class.java))
        } catch (e: Exception) {
            Log.w(TAG, "stopService failed", e)
        }
        mediaSession?.release()
        mediaSession = null

        // Cancel the network monitor before nulling the manager.
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
    // in `mobile.rs`. Gated to `cfg(target_os = "ios")` on the Rust call
    // side, so they should never be invoked on Android — having them
    // present keeps the plugin contract complete.

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
