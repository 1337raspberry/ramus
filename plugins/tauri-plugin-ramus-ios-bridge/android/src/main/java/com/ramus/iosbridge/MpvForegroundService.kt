package com.ramus.iosbridge

import android.content.Intent
import androidx.media3.common.util.UnstableApi
import androidx.media3.session.MediaSession
import androidx.media3.session.MediaSessionService

/**
 * Foreground service that hosts the app's [MediaSession]. Owning the
 * session in a service is what keeps audio playing while the activity
 * is paused (screen lock) and what wires the OS lock-screen / Bluetooth
 * / notification controls.
 *
 * The session is built by [MpvBridgePlugin] (which already owns the
 * underlying `ExoPlayer`) and pushed here via [attachSession]. We
 * deliberately do not own the player ourselves: every Tauri IPC
 * `@Command` needs a synchronous handle on it, and routing every call
 * through service binding would force the plugin to become async. The
 * plugin's lifecycle drives the session — `onGetSession` simply hands
 * back whatever was attached.
 *
 * Media3's `DefaultMediaNotificationProvider` builds the MediaStyle
 * notification and auto-promotes us to foreground when playback
 * starts; we never call `startForeground()` directly.
 */
@UnstableApi
class MpvForegroundService : MediaSessionService() {
    companion object {
        @Volatile
        private var attached: MediaSession? = null

        fun attachSession(session: MediaSession) {
            attached = session
        }

        fun detachSession() {
            attached = null
        }
    }

    override fun onCreate() {
        super.onCreate()
        // Register the plugin-owned session with the service so
        // `DefaultMediaNotificationProvider` can render the notification
        // + lock-screen controls and Media3 can self-promote to
        // foreground when playback starts. The session must be attached
        // (via `attachSession` in the plugin) before this runs — the
        // plugin sequences attach → startForegroundService for that
        // reason.
        attached?.let { addSession(it) }
    }

    override fun onGetSession(controllerInfo: MediaSession.ControllerInfo): MediaSession? =
        attached

    override fun onTaskRemoved(rootIntent: Intent?) {
        // Match the Google Play media-app expectation: when the user
        // swipes the app away from recents, only kill the service if
        // playback is already idle/stopped. If audio is still rolling,
        // honour it as background playback.
        val session = attached
        val player = session?.player
        if (player == null || !player.playWhenReady || player.mediaItemCount == 0) {
            stopSelf()
        }
    }
}
