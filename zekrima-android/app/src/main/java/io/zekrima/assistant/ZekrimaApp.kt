package io.zekrima.assistant

import android.app.Application
import android.app.NotificationChannel
import android.app.NotificationManager
import android.content.Intent
import android.media.projection.MediaProjection
import android.os.Build

class ZekrimaApp : Application() {

    companion object {
        const val CHANNEL_OVERLAY_ID = "zekrima_overlay"
        const val CHANNEL_ALERTS_ID = "zekrima_alerts"

        @Volatile
        lateinit var instance: ZekrimaApp
            private set

        @Volatile
        var overlayRunning: Boolean = false

        @Volatile
        var mediaProjectionResultCode: Int = 0

        @Volatile
        var mediaProjectionData: Intent? = null

        @Volatile
        var activeMediaProjection: MediaProjection? = null

        @Volatile
        var pendingProjectionPurpose: String = ""

        val listeners = java.util.concurrent.CopyOnWriteArrayList<() -> Unit>()

        fun notifyStateChanged() {
            for (l in listeners) {
                try {
                    l()
                } catch (_: Throwable) {
                }
            }
        }
    }

    override fun onCreate() {
        super.onCreate()
        instance = this
        createNotificationChannels()
    }

    private fun createNotificationChannels() {
        val nm = getSystemService(NotificationManager::class.java) ?: return
        val overlayCh = NotificationChannel(
            CHANNEL_OVERLAY_ID,
            "Zekrima Dynamic Island",
            NotificationManager.IMPORTANCE_LOW
        ).apply {
            description = "Keeps Zekrima Dynamic Island & Cicada Mode running smoothly"
            setShowBadge(false)
        }
        val alertsCh = NotificationChannel(
            CHANNEL_ALERTS_ID,
            "Zekrima Alerts & Pomodoro",
            NotificationManager.IMPORTANCE_DEFAULT
        ).apply {
            description = "Pomodoro timer completions and Zeki alerts"
        }
        nm.createNotificationChannel(overlayCh)
        nm.createNotificationChannel(alertsCh)
    }
}
