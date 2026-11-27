package io.zekrima.assistant.services

import android.app.Notification
import android.app.PendingIntent
import android.content.ComponentName
import android.content.Context
import android.media.MediaMetadata
import android.media.session.MediaController
import android.media.session.MediaSessionManager
import android.media.session.PlaybackState
import android.service.notification.NotificationListenerService
import android.service.notification.StatusBarNotification
import io.zekrima.assistant.ZekrimaApp
import java.util.concurrent.CopyOnWriteArrayList

data class IslandNotification(
    val key: String,
    val appName: String,
    val title: String,
    val body: String,
    val postedAtMs: Long,
    val contentIntent: PendingIntent? = null
)

data class MediaInfo(
    val hasSession: Boolean = false,
    val isPlaying: Boolean = false,
    val title: String = "",
    val artist: String = "",
    val appName: String = ""
)

enum class MediaControlAction { PREVIOUS, PLAY_PAUSE, NEXT }

/**
 * Shared live store updated by ZekrimaNotificationListener and local app events (Pomodoro, etc.).
 */
object LiveNotificationStore {
    private val items = CopyOnWriteArrayList<IslandNotification>()

    @Volatile
    var latestToast: IslandNotification? = null

    @Volatile
    var toastUntilMs: Long = 0L

    @Volatile
    var mediaInfo: MediaInfo = MediaInfo()

    @Volatile
    var activeMediaController: MediaController? = null

    @Volatile
    var listenerConnected: Boolean = false

    fun list(): List<IslandNotification> = items.toList()

    fun unreadCount(): Int = items.size

    fun isToastVisible(nowMs: Long = System.currentTimeMillis()): Boolean =
        latestToast != null && nowMs < toastUntilMs

    fun pushToast(app: String, title: String, body: String, key: String = "local-${System.currentTimeMillis()}", intent: PendingIntent? = null) {
        val item = IslandNotification(key, app, title, body, System.currentTimeMillis(), intent)
        items.removeAll { it.key == key }
        items.add(0, item)
        while (items.size > 15) {
            items.removeAt(items.lastIndex)
        }
        latestToast = item
        toastUntilMs = System.currentTimeMillis() + 5_500L
        ZekrimaApp.notifyStateChanged()
    }

    fun dismiss(key: String) {
        items.removeAll { it.key == key }
        if (latestToast?.key == key) {
            latestToast = null
            toastUntilMs = 0L
        }
        ZekrimaApp.notifyStateChanged()
    }

    fun clearToast() {
        latestToast = null
        toastUntilMs = 0L
        ZekrimaApp.notifyStateChanged()
    }

    fun clearAll() {
        items.clear()
        clearToast()
    }

    fun controlMedia(action: MediaControlAction) {
        val c = activeMediaController ?: return
        val tc = c.transportControls
        when (action) {
            MediaControlAction.PREVIOUS -> tc.skipToPrevious()
            MediaControlAction.NEXT -> tc.skipToNext()
            MediaControlAction.PLAY_PAUSE -> {
                val playing = c.playbackState?.state == PlaybackState.STATE_PLAYING
                if (playing) tc.pause() else tc.play()
            }
        }
    }

    fun summaryForAssistant(): String {
        val snap = items.take(8)
        if (snap.isEmpty()) return ""
        return snap.joinToString("\n") { "- [${it.appName}] ${it.title}: ${it.body}" }
    }
}

class ZekrimaNotificationListener : NotificationListenerService() {

    private var sessionManager: MediaSessionManager? = null
    private val sessionsListener = MediaSessionManager.OnActiveSessionsChangedListener { controllers ->
        syncMediaSessions(controllers)
    }

    override fun onListenerConnected() {
        super.onListenerConnected()
        LiveNotificationStore.listenerConnected = true
        try {
            val existing = activeNotifications ?: emptyArray()
            for (sbn in existing.take(10)) {
                ingestNotification(sbn, showToast = false)
            }
        } catch (_: Throwable) {
        }
        startMediaWatcher()
        ZekrimaApp.notifyStateChanged()
    }

    override fun onListenerDisconnected() {
        super.onListenerDisconnected()
        LiveNotificationStore.listenerConnected = false
        try {
            sessionManager?.removeOnActiveSessionsChangedListener(sessionsListener)
        } catch (_: Throwable) {
        }
        ZekrimaApp.notifyStateChanged()
    }

    override fun onNotificationPosted(sbn: StatusBarNotification?) {
        if (sbn == null) return
        if (sbn.packageName == packageName) return
        ingestNotification(sbn, showToast = true)
    }

    override fun onNotificationRemoved(sbn: StatusBarNotification?) {
        if (sbn == null) return
        LiveNotificationStore.dismiss(sbn.key)
    }

    private fun ingestNotification(sbn: StatusBarNotification, showToast: Boolean) {
        val n = sbn.notification ?: return
        if ((n.flags and Notification.FLAG_ONGOING_EVENT) != 0) return
        val extras = n.extras ?: return
        val title = extras.getCharSequence(Notification.EXTRA_TITLE)?.toString()?.trim().orEmpty()
        val text = extras.getCharSequence(Notification.EXTRA_TEXT)?.toString()?.trim().orEmpty()
        if (title.isEmpty() && text.isEmpty()) return

        val appName = try {
            val ai = packageManager.getApplicationInfo(sbn.packageName, 0)
            packageManager.getApplicationLabel(ai).toString()
        } catch (_: Throwable) {
            sbn.packageName.substringAfterLast('.')
        }

        if (showToast) {
            LiveNotificationStore.pushToast(appName, title, text, sbn.key, n.contentIntent)
        } else {
            val item = IslandNotification(sbn.key, appName, title, text, sbn.postTime, n.contentIntent)
            if (LiveNotificationStore.list().none { it.key == sbn.key }) {
                LiveNotificationStore.pushToast(appName, title, text, sbn.key, n.contentIntent)
                LiveNotificationStore.clearToast()
            }
        }
    }

    private fun startMediaWatcher() {
        try {
            val msm = getSystemService(Context.MEDIA_SESSION_SERVICE) as? MediaSessionManager ?: return
            sessionManager = msm
            val comp = ComponentName(this, ZekrimaNotificationListener::class.java)
            msm.addOnActiveSessionsChangedListener(sessionsListener, comp)
            syncMediaSessions(msm.getActiveSessions(comp))
        } catch (_: Throwable) {
        }
    }

    private fun syncMediaSessions(controllers: List<MediaController>?) {
        val list = controllers.orEmpty()
        val chosen = list.firstOrNull { it.playbackState?.state == PlaybackState.STATE_PLAYING }
            ?: list.firstOrNull()
        LiveNotificationStore.activeMediaController = chosen
        if (chosen == null) {
            LiveNotificationStore.mediaInfo = MediaInfo()
        } else {
            val meta = chosen.metadata
            val title = meta?.getString(MediaMetadata.METADATA_KEY_TITLE)
                ?: meta?.getString(MediaMetadata.METADATA_KEY_DISPLAY_TITLE)
                ?: "Playing Media"
            val artist = meta?.getString(MediaMetadata.METADATA_KEY_ARTIST)
                ?: meta?.getString(MediaMetadata.METADATA_KEY_ALBUM_ARTIST)
                ?: ""
            val playing = chosen.playbackState?.state == PlaybackState.STATE_PLAYING
            LiveNotificationStore.mediaInfo = MediaInfo(
                hasSession = true,
                isPlaying = playing,
                title = title,
                artist = artist,
                appName = chosen.packageName.substringAfterLast('.')
            )
            chosen.registerCallback(object : MediaController.Callback() {
                override fun onPlaybackStateChanged(state: PlaybackState?) {
                    syncMediaSessions(listOf(chosen))
                }

                override fun onMetadataChanged(metadata: MediaMetadata?) {
                    syncMediaSessions(listOf(chosen))
                }
            })
        }
        ZekrimaApp.notifyStateChanged()
    }
}
