package com.ramus.iosbridge

import android.content.Context
import android.os.Handler
import android.os.Looper
import android.util.Log
import androidx.media3.common.AudioAttributes
import androidx.media3.common.C
import androidx.media3.common.MediaItem
import androidx.media3.common.MediaMetadata
import androidx.media3.common.PlaybackException
import androidx.media3.common.Player
import androidx.media3.common.SimpleBasePlayer
import com.google.common.util.concurrent.Futures
import com.google.common.util.concurrent.ListenableFuture
import dev.jdtech.mpv.MPVLib

/**
 * Wraps `dev.jdtech.mpv.MPVLib` behind Media3's `SimpleBasePlayer` so the
 * existing `MediaSession` / `MediaSessionService` / lock-screen widget keep
 * working unchanged. State mutations must run on the application looper —
 * libmpv property events fire on a background thread, so observer callbacks
 * marshal to main before touching state.
 */
class LibmpvSimplePlayer(
    context: Context,
) : SimpleBasePlayer(Looper.getMainLooper()), MPVLib.EventObserver, MPVLib.LogObserver {

    private val mainHandler = Handler(Looper.getMainLooper())
    private val mpv: MPVLib = MPVLib.create(context.applicationContext)
        ?: throw IllegalStateException("MPVLib.create returned null — libmpv failed to allocate")

    private var queue: MutableList<MediaItem> = mutableListOf()
    private var currentIndex: Int = C.INDEX_UNSET
    private var positionMs: Long = 0L
    private var durationMs: Long = C.TIME_UNSET
    private var paused: Boolean = true
    private var bufferingForCache: Boolean = false
    private var idleActive: Boolean = true
    private var fileLoaded: Boolean = false
    private var lastError: PlaybackException? = null
    private var released: Boolean = false

    init {
        // Audio-only configuration. Mirrors `default_mpv_options()` in
        // `ramus-core/src/playback/mpv.rs` and `MpvController.swift` —
        // every platform sets the same option list so playback behaviour
        // is identical across desktop, iOS, and Android. The only
        // platform override here is `ao=audiotrack,opensles` (Android's
        // audio output backends; desktop uses coreaudio/wasapi/pulse,
        // iOS uses audiounit).
        mpv.setOptionString("vo", "null")
        mpv.setOptionString("vid", "no")
        mpv.setOptionString("ao", "audiotrack,opensles")
        mpv.setOptionString("gapless-audio", "yes")
        mpv.setOptionString("prefetch-playlist", "no")
        mpv.setOptionString("audio-buffer", "0.5")
        mpv.setOptionString("demuxer-readahead-secs", "1200")
        mpv.setOptionString("demuxer-max-bytes", "2GiB")
        mpv.setOptionString("network-timeout", "15")
        mpv.setOptionString(
            "stream-lavf-o",
            "reconnect=1,reconnect_streamed=1,reconnect_on_network_error=1,reconnect_delay_max=4",
        )
        mpv.setOptionString("keep-open", "no")
        mpv.setOptionString("idle", "yes")
        mpv.setOptionString("input-default-bindings", "no")
        mpv.setOptionString("input-vo-keyboard", "no")
        mpv.setOptionString("terminal", "no")
        mpv.setOptionString("load-scripts", "no")
        mpv.setOptionString("msg-level", "all=warn")

        mpv.addObserver(this)
        mpv.addLogObserver(this)

        mpv.init()

        // Observed-property set matches desktop and iOS, plus one
        // Android-only extra: `paused-for-cache` drives Media3's
        // `Player.STATE_BUFFERING` transitions. Desktop and iOS don't
        // need this because their player layers (`AudioPlayer` /
        // `MPNowPlayingInfoCenter`) don't have a buffering state in
        // their state machines. `demuxer-cache-time` is read on demand
        // by the prefetch worker (matching iOS), so it's not observed.
        mpv.observeProperty("time-pos", MPVLib.MpvFormat.MPV_FORMAT_DOUBLE)
        mpv.observeProperty("duration", MPVLib.MpvFormat.MPV_FORMAT_DOUBLE)
        mpv.observeProperty("pause", MPVLib.MpvFormat.MPV_FORMAT_FLAG)
        mpv.observeProperty("playlist-pos", MPVLib.MpvFormat.MPV_FORMAT_INT64)
        mpv.observeProperty("idle-active", MPVLib.MpvFormat.MPV_FORMAT_FLAG)
        mpv.observeProperty("paused-for-cache", MPVLib.MpvFormat.MPV_FORMAT_FLAG)
    }

    // -----------------------------------------------------------------
    // SimpleBasePlayer surface
    // -----------------------------------------------------------------

    override fun getState(): State {
        val items = queue.map { item ->
            MediaItemData.Builder(item.mediaId.ifEmpty { item.hashCode().toString() })
                .setMediaItem(item)
                .setDurationUs(if (durationMs > 0) durationMs * 1000L else C.TIME_UNSET)
                .build()
        }
        val playbackState = when {
            released || queue.isEmpty() || idleActive -> Player.STATE_IDLE
            bufferingForCache || !fileLoaded -> Player.STATE_BUFFERING
            durationMs > 0 && positionMs >= durationMs -> Player.STATE_ENDED
            else -> Player.STATE_READY
        }
        // Media3 enforces `isLoading == false` in IDLE/ENDED states. Setting
        // it true outside STATE_BUFFERING (the only state where it's
        // meaningful for us) throws "isLoading only allowed when not in
        // STATE_IDLE or STATE_ENDED" the moment the MediaSession tries to
        // build the legacy controller view.
        val isLoading = (bufferingForCache || !fileLoaded) &&
            playbackState != Player.STATE_IDLE &&
            playbackState != Player.STATE_ENDED
        val safeIndex = if (queue.isEmpty()) 0
            else currentIndex.coerceIn(0, queue.size - 1)
        return State.Builder()
            .setAvailableCommands(AUDIO_COMMANDS)
            .setPlaylist(items)
            .setCurrentMediaItemIndex(safeIndex)
            .setContentPositionMs(positionMs)
            .setIsLoading(isLoading)
            .setPlayWhenReady(!paused, Player.PLAY_WHEN_READY_CHANGE_REASON_USER_REQUEST)
            .setPlaybackState(playbackState)
            .setPlayerError(lastError)
            .setAudioAttributes(MUSIC_AUDIO_ATTRIBUTES)
            .build()
    }

    override fun handleSetPlayWhenReady(playWhenReady: Boolean): ListenableFuture<*> {
        mpv.setPropertyBoolean("pause", !playWhenReady)
        // State reflection happens via the `pause` property observer.
        return Futures.immediateVoidFuture()
    }

    override fun handlePrepare(): ListenableFuture<*> {
        // mpv begins demuxing as soon as `loadfile` runs in
        // handleSetMediaItems; nothing to do here.
        return Futures.immediateVoidFuture()
    }

    override fun handleStop(): ListenableFuture<*> {
        mpv.command(arrayOf("stop"))
        return Futures.immediateVoidFuture()
    }

    override fun handleRelease(): ListenableFuture<*> {
        if (released) return Futures.immediateVoidFuture()
        released = true
        mpv.removeObserver(this)
        mpv.removeLogObserver(this)
        mpv.destroy()
        return Futures.immediateVoidFuture()
    }

    override fun handleSeek(
        mediaItemIndex: Int,
        positionMs: Long,
        seekCommand: Int,
    ): ListenableFuture<*> {
        if (mediaItemIndex != currentIndex && mediaItemIndex in queue.indices) {
            mpv.command(arrayOf("playlist-play-index", mediaItemIndex.toString()))
        }
        val seconds = positionMs.coerceAtLeast(0L) / 1000.0
        mpv.command(arrayOf("seek", seconds.toString(), "absolute"))
        return Futures.immediateVoidFuture()
    }

    override fun handleSetMediaItems(
        mediaItems: List<MediaItem>,
        startIndex: Int,
        startPositionMs: Long,
    ): ListenableFuture<*> {
        queue = mediaItems.toMutableList()
        currentIndex = startIndex.coerceIn(0, mediaItems.size.coerceAtLeast(1) - 1)
        fileLoaded = false
        positionMs = 0L
        durationMs = C.TIME_UNSET

        mpv.command(arrayOf("playlist-clear"))
        mediaItems.forEachIndexed { idx, item ->
            val url = item.requestMetadata.mediaUri?.toString() ?: item.localConfiguration?.uri?.toString()
            if (url.isNullOrEmpty()) return@forEachIndexed
            val mode = if (idx == 0) "replace" else "append"
            mpv.command(arrayOf("loadfile", url, mode))
        }
        if (currentIndex > 0) {
            mpv.command(arrayOf("playlist-play-index", currentIndex.toString()))
        }
        if (startPositionMs > 0L) {
            mpv.command(arrayOf("seek", (startPositionMs / 1000.0).toString(), "absolute"))
        }
        return Futures.immediateVoidFuture()
    }

    override fun handleAddMediaItems(
        index: Int,
        mediaItems: List<MediaItem>,
    ): ListenableFuture<*> {
        val insertAt = index.coerceIn(0, queue.size)
        mediaItems.forEachIndexed { offset, item ->
            val url = item.requestMetadata.mediaUri?.toString() ?: item.localConfiguration?.uri?.toString()
            if (url.isNullOrEmpty()) return@forEachIndexed
            mpv.command(arrayOf("loadfile", url, "append"))
            queue.add(insertAt + offset, item)
        }
        // Move appended entries into position if not appending to the tail.
        if (insertAt < queue.size - mediaItems.size) {
            val originalTail = queue.size - mediaItems.size
            mediaItems.indices.forEach { i ->
                mpv.command(
                    arrayOf(
                        "playlist-move",
                        (originalTail + i).toString(),
                        (insertAt + i).toString(),
                    ),
                )
            }
        }
        return Futures.immediateVoidFuture()
    }

    override fun handleRemoveMediaItems(
        fromIndex: Int,
        toIndex: Int,
    ): ListenableFuture<*> {
        // mpv expects single-item removes; iterate from the back so the
        // indices don't shift under us.
        for (i in (toIndex - 1) downTo fromIndex) {
            if (i in queue.indices) {
                mpv.command(arrayOf("playlist-remove", i.toString()))
                queue.removeAt(i)
            }
        }
        return Futures.immediateVoidFuture()
    }

    override fun handleMoveMediaItems(
        fromIndex: Int,
        toIndex: Int,
        newIndex: Int,
    ): ListenableFuture<*> {
        val count = toIndex - fromIndex
        for (i in 0 until count) {
            // mpv's playlist-move is single-element; same semantics as our
            // existing playlist-move usage on iOS.
            mpv.command(
                arrayOf(
                    "playlist-move",
                    (fromIndex + i).toString(),
                    (newIndex + i).toString(),
                ),
            )
        }
        val moved = queue.subList(fromIndex, toIndex).toList()
        queue.subList(fromIndex, toIndex).clear()
        queue.addAll(newIndex.coerceIn(0, queue.size), moved)
        return Futures.immediateVoidFuture()
    }

    override fun handleSetAudioAttributes(
        audioAttributes: AudioAttributes,
        handleAudioFocus: Boolean,
    ): ListenableFuture<*> {
        // libmpv owns its AudioTrack; Media3's audio-focus logic still
        // runs around the player. No-op here.
        return Futures.immediateVoidFuture()
    }

    // -----------------------------------------------------------------
    // Plugin-facing helpers (called from MpvBridgePlugin)
    // -----------------------------------------------------------------

    /** Apply an mpv lavfi audio-filter chain (e.g. the 10-band EQ string). */
    fun setAudioFilters(lavfi: String) {
        mpv.setOptionString("af", lavfi)
    }

    /**
     * Set mpv's `volume` property directly (0..100). Distinct from
     * `Player.setVolume(Float)` (inherited from SimpleBasePlayer, 0..1)
     * — that one would route through `handleSetVolume` and adjust the
     * SimpleBasePlayer's gain on top of mpv's internal volume.
     */
    fun setMpvVolume(volume: Double) {
        mpv.setPropertyDouble("volume", volume.coerceIn(0.0, 100.0))
    }

    fun getMpvVolume(): Double = mpv.getPropertyDouble("volume") ?: 100.0

    /** mpv's demuxer-cache-time, used by `prefetch::wait_for_live_drain`. */
    fun demuxerCacheTimeSeconds(): Double? =
        mpv.getPropertyDouble("demuxer-cache-time")?.takeIf { it >= 0.0 }

    /**
     * Replace the current playlist entry's `MediaItem` with one carrying
     * fresh `MediaMetadata` (title / artist / album / artwork). Mirrors
     * the previous `replaceMediaItem(currentIndex, ...)` flow that fed the
     * lock-screen widget. State invalidates so the MediaSession picks up
     * the new metadata.
     */
    fun replaceCurrentMediaItemMetadata(metadata: MediaMetadata) {
        val idx = currentIndex
        if (idx !in queue.indices) return
        val existing = queue[idx]
        queue[idx] = existing.buildUpon().setMediaMetadata(metadata).build()
        runOnMain { invalidateState() }
    }

    fun isReleased(): Boolean = released

    // -----------------------------------------------------------------
    // libmpv event observer (fires off-main; marshal to applicationLooper)
    // -----------------------------------------------------------------

    override fun eventProperty(property: String) { /* no value carried */ }

    override fun eventProperty(property: String, value: Long) {
        runOnMain {
            when (property) {
                "playlist-pos" -> {
                    val idx = value.toInt()
                    if (idx >= 0 && idx != currentIndex) {
                        currentIndex = idx
                        fileLoaded = false
                        positionMs = 0L
                        durationMs = C.TIME_UNSET
                    }
                    invalidateState()
                }
            }
        }
    }

    override fun eventProperty(property: String, value: Double) {
        runOnMain {
            when (property) {
                "time-pos" -> {
                    positionMs = (value * 1000.0).toLong()
                    invalidateState()
                }
                "duration" -> {
                    durationMs = (value * 1000.0).toLong()
                    invalidateState()
                }
            }
        }
    }

    override fun eventProperty(property: String, value: Boolean) {
        runOnMain {
            when (property) {
                "pause" -> {
                    paused = value
                    invalidateState()
                }
                "idle-active" -> {
                    idleActive = value
                    if (value) {
                        fileLoaded = false
                    }
                    invalidateState()
                }
                "paused-for-cache" -> {
                    bufferingForCache = value
                    invalidateState()
                }
            }
        }
    }

    override fun eventProperty(property: String, value: String) { /* unused */ }

    override fun event(eventId: Int) {
        when (eventId) {
            MPVLib.MpvEvent.MPV_EVENT_FILE_LOADED -> {
                runOnMain {
                    fileLoaded = true
                    idleActive = false
                    invalidateState()
                }
            }
            MPVLib.MpvEvent.MPV_EVENT_END_FILE -> {
                runOnMain {
                    fileLoaded = false
                    invalidateState()
                }
            }
        }
    }

    override fun logMessage(prefix: String, level: Int, text: String) {
        when {
            level <= MPVLib.MpvLogLevel.MPV_LOG_LEVEL_ERROR -> Log.e("mpv/$prefix", text.trimEnd())
            level <= MPVLib.MpvLogLevel.MPV_LOG_LEVEL_WARN -> Log.w("mpv/$prefix", text.trimEnd())
            level <= MPVLib.MpvLogLevel.MPV_LOG_LEVEL_INFO -> Log.i("mpv/$prefix", text.trimEnd())
            else -> Log.d("mpv/$prefix", text.trimEnd())
        }
    }

    private fun runOnMain(action: () -> Unit) {
        if (Looper.myLooper() == Looper.getMainLooper()) action()
        else mainHandler.post(action)
    }

    companion object {
        private val AUDIO_COMMANDS: Player.Commands = Player.Commands.Builder()
            .addAll(
                Player.COMMAND_PLAY_PAUSE,
                Player.COMMAND_PREPARE,
                Player.COMMAND_STOP,
                Player.COMMAND_SEEK_TO_DEFAULT_POSITION,
                Player.COMMAND_SEEK_IN_CURRENT_MEDIA_ITEM,
                Player.COMMAND_SEEK_TO_PREVIOUS_MEDIA_ITEM,
                Player.COMMAND_SEEK_TO_PREVIOUS,
                Player.COMMAND_SEEK_TO_NEXT_MEDIA_ITEM,
                Player.COMMAND_SEEK_TO_NEXT,
                Player.COMMAND_SEEK_TO_MEDIA_ITEM,
                Player.COMMAND_SET_REPEAT_MODE,
                Player.COMMAND_GET_CURRENT_MEDIA_ITEM,
                Player.COMMAND_GET_TIMELINE,
                Player.COMMAND_GET_METADATA,
                Player.COMMAND_CHANGE_MEDIA_ITEMS,
                Player.COMMAND_SET_AUDIO_ATTRIBUTES,
                Player.COMMAND_GET_AUDIO_ATTRIBUTES,
                Player.COMMAND_GET_VOLUME,
                Player.COMMAND_SET_VOLUME,
                Player.COMMAND_RELEASE,
            )
            .build()

        private val MUSIC_AUDIO_ATTRIBUTES: AudioAttributes = AudioAttributes.Builder()
            .setContentType(C.AUDIO_CONTENT_TYPE_MUSIC)
            .setUsage(C.USAGE_MEDIA)
            .build()
    }
}
