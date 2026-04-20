package com.ramus.iosbridge

import android.app.Notification
import android.content.Context
import android.os.Build
import android.os.Bundle
import androidx.media3.common.util.UnstableApi
import androidx.media3.session.CommandButton
import androidx.media3.session.DefaultMediaNotificationProvider
import androidx.media3.session.MediaNotification
import androidx.media3.session.MediaSession
import com.google.common.collect.ImmutableList

/**
 * Wraps [DefaultMediaNotificationProvider] to paint the Media3 lock-screen
 * / shade notification with the app's current accent colour.
 *
 * Media3's default provider gives us the MediaStyle layout, transport
 * buttons, and artwork for free but exposes no hook for tinting. We
 * let it build the notification, then pull the builder back out via
 * [Notification.Builder.recoverBuilder] and layer `setColorized(true)`
 * + `setColor(accent)` on top. Requires API 26+ for `setColorized`;
 * `setColor` is API 21+ and falls back to the launcher-tint colour on
 * pre-O devices (minSdk=24 here).
 *
 * The accent is a volatile int (0xAARRGGBB) so [setAccent] can be called
 * from any thread — the next [createNotification] pass picks it up.
 * Because Media3 only calls [createNotification] on specific player
 * events, a standalone accent change needs a nudge — see
 * `MpvBridgePlugin.setMediaAccent`.
 */
@UnstableApi
class ColorizedMediaNotificationProvider(
    private val context: Context,
) : MediaNotification.Provider {
    private val delegate: DefaultMediaNotificationProvider =
        DefaultMediaNotificationProvider.Builder(context).build()

    @Volatile
    private var accentColor: Int? = null

    fun setAccent(argb: Int?) {
        accentColor = argb
    }

    override fun createNotification(
        mediaSession: MediaSession,
        mediaButtonPreferences: ImmutableList<CommandButton>,
        actionFactory: MediaNotification.ActionFactory,
        onNotificationChangedCallback: MediaNotification.Provider.Callback,
    ): MediaNotification {
        val base = delegate.createNotification(
            mediaSession,
            mediaButtonPreferences,
            actionFactory,
            onNotificationChangedCallback,
        )
        val color = accentColor ?: return base
        return try {
            val builder = Notification.Builder.recoverBuilder(context, base.notification)
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
                builder.setColorized(true)
            }
            builder.setColor(color)
            MediaNotification(base.notificationId, builder.build())
        } catch (e: Throwable) {
            base
        }
    }

    override fun handleCustomCommand(
        session: MediaSession,
        action: String,
        extras: Bundle,
    ): Boolean = delegate.handleCustomCommand(session, action, extras)
}
