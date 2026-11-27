package io.zekrima.assistant.services

import android.app.Notification
import android.app.PendingIntent
import android.app.Service
import android.content.ClipData
import android.content.ClipboardManager
import android.content.Context
import android.content.Intent
import android.content.pm.ServiceInfo
import android.graphics.Color
import android.graphics.PixelFormat
import android.graphics.drawable.GradientDrawable
import android.media.projection.MediaProjectionManager
import android.os.Build
import android.os.Handler
import android.os.IBinder
import android.os.Looper
import android.os.SystemClock
import android.provider.Settings
import android.view.Choreographer
import android.view.Gravity
import android.view.View
import android.view.WindowManager
import android.view.inputmethod.EditorInfo
import android.view.inputmethod.InputMethodManager
import android.widget.Button
import android.widget.EditText
import android.widget.LinearLayout
import androidx.core.app.NotificationCompat
import io.zekrima.assistant.R
import io.zekrima.assistant.ZekrimaApp
import io.zekrima.assistant.config.AssistantConfig
import io.zekrima.assistant.render.ExpandedTab
import io.zekrima.assistant.render.HitAction
import io.zekrima.assistant.render.IslandCanvasView
import io.zekrima.assistant.render.IslandRenderState
import io.zekrima.assistant.render.PillGeometry
import io.zekrima.assistant.render.PillMode
import io.zekrima.assistant.render.PillSprings
import io.zekrima.assistant.render.ZekiCicadaAccessory
import io.zekrima.assistant.render.ZekiEngine
import io.zekrima.assistant.render.ZekiState
import io.zekrima.assistant.ui.MainActivity
import io.zekrima.assistant.ui.MediaProjectionActivity

class IslandOverlayService : Service() {

    companion object {
        const val ACTION_START = "io.zekrima.action.START"
        const val ACTION_STOP = "io.zekrima.action.STOP"
        const val ACTION_TOGGLE_CICADA = "io.zekrima.action.TOGGLE_CICADA"
        const val ACTION_PROJECTION_READY = "io.zekrima.action.PROJECTION_READY"
        const val ACTION_OPEN_CHAT = "io.zekrima.action.OPEN_CHAT"
        const val ACTION_START_VOICE = "io.zekrima.action.START_VOICE"
        const val ACTION_UNLOCK_SUBTITLE = "io.zekrima.action.UNLOCK_SUBTITLE"
        const val ACTION_TEST_TOAST = "io.zekrima.action.TEST_TOAST"
        const val ACTION_REFRESH_CONFIG = "io.zekrima.action.REFRESH_CONFIG"

        private const val NOTIF_ID = 4040
        private const val AUTO_HIDE_DELAY_MS = 10_000L
    }

    private lateinit var wm: WindowManager
    private lateinit var rootContainer: LinearLayout
    private lateinit var canvasView: IslandCanvasView
    private lateinit var chatBar: LinearLayout
    private lateinit var chatInput: EditText
    private lateinit var layoutParams: WindowManager.LayoutParams

    private val springs = PillSprings()
    private val zeki = ZekiEngine()
    private val timer = PomodoroTimer()
    private lateinit var assistant: AssistantService
    private lateinit var cicada: CicadaService

    private val handler = Handler(Looper.getMainLooper())
    private val clipboardItems = mutableListOf<String>()
    private var clipboardManager: ClipboardManager? = null

    private var expanded = false
    private var tab = ExpandedTab.ZEKI
    private var chatBarVisible = false
    private var cicadaSubtitleBar = false
    private var cicadaClickThrough = false
    private var autoHideEnabled = true
    private var autoHidden = false
    private var lastActivityMs = SystemClock.uptimeMillis()
    private var copiedFlashUntilMs = 0L
    private var lastCicadaArrivalSeq = 0L
    private var sysStats = SystemStats()

    private var animating = false
    private var lastFrameNanos = 0L
    private var maxWidthDp = 360f

    private val stateListener: () -> Unit = {
        handler.post { onExternalStateChanged() }
    }

    private val clipListener = ClipboardManager.OnPrimaryClipChangedListener {
        val text = clipboardManager?.primaryClip?.getItemAt(0)?.coerceToText(this)?.toString()?.trim()
        if (!text.isNullOrEmpty()) {
            clipboardItems.remove(text)
            clipboardItems.add(0, text.take(400))
            while (clipboardItems.size > 6) clipboardItems.removeAt(clipboardItems.lastIndex)
            renderNow(0.016f)
        }
    }

    private val frameCallback = object : Choreographer.FrameCallback {
        override fun doFrame(frameTimeNanos: Long) {
            if (!animating) return
            val dt = if (lastFrameNanos == 0L) 0.016f else {
                ((frameTimeNanos - lastFrameNanos) / 1_000_000_000f).coerceIn(0.004f, 0.08f)
            }
            lastFrameNanos = frameTimeNanos

            val moving = springs.update(dt)
            syncZekiLiveLevels(dt)
            renderNow(dt)
            updateWindowBounds()

            if (needsContinuousAnimation() || moving) {
                Choreographer.getInstance().postFrameCallback(this)
            } else {
                animating = false
                lastFrameNanos = 0L
            }
        }
    }

    private val oneSecondTicker = object : Runnable {
        override fun run() {
            val now = SystemClock.uptimeMillis()
            if (timer.running) {
                val finished = timer.tick()
                if (finished) {
                    zeki.triggerActionHappy()
                    LiveNotificationStore.pushToast(
                        "Pomodoro",
                        "Focus Session Complete!",
                        "Waktunya istirahat sejenak."
                    )
                }
                ensureAnimating()
            }
            if (zeki.onOneSecondTick()) {
                ensureAnimating()
            }
            if (expanded && tab == ExpandedTab.SYS) {
                sysStats = SystemTools.sampleStats(this@IslandOverlayService)
                renderNow(0.016f)
            }

            // 10-second Auto-Hide into top edge when idle
            val canAutoHide = autoHideEnabled &&
                !expanded &&
                !chatBarVisible &&
                !LiveNotificationStore.isToastVisible() &&
                !assistant.snapshot().phase.isBusy() &&
                !timer.running &&
                !cicada.snapshot().active &&
                !cicadaSubtitleBar

            if (!autoHidden && canAutoHide && now - lastActivityMs >= AUTO_HIDE_DELAY_MS) {
                autoHidden = true
                applyPillMode()
                ensureAnimating()
            } else if (!canAutoHide) {
                lastActivityMs = now
            }

            if (!autoHidden) {
                renderNow(0.016f)
            }
            handler.postDelayed(this, 1_000L)
        }
    }

    override fun onBind(intent: Intent?): IBinder? = null

    override fun onCreate() {
        super.onCreate()
        wm = getSystemService(Context.WINDOW_SERVICE) as WindowManager
        maxWidthDp = ((resources.displayMetrics.widthPixels / resources.displayMetrics.density) - 12f)
            .coerceIn(300f, 440f)

        assistant = AssistantService(this)
        cicada = CicadaService(this, assistant)
        autoHideEnabled = AssistantConfig.load(this).autoHide
        sysStats = SystemTools.sampleStats(this)

        startForegroundCompat(includeMediaProjection = false)
        buildOverlayViews()
        ZekrimaApp.overlayRunning = true
        ZekrimaApp.listeners.add(stateListener)

        clipboardManager = getSystemService(Context.CLIPBOARD_SERVICE) as? ClipboardManager
        clipboardManager?.addPrimaryClipChangedListener(clipListener)

        applyPillMode()
        renderNow(0.016f)
        ensureAnimating()
        handler.postDelayed(oneSecondTicker, 1_000L)
        ZekrimaApp.notifyStateChanged()
    }

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        when (intent?.action) {
            ACTION_STOP -> {
                stopSelf()
                return START_NOT_STICKY
            }
            ACTION_REFRESH_CONFIG -> {
                val cfg = AssistantConfig.load(this)
                autoHideEnabled = cfg.autoHide
                if (!autoHideEnabled) revealFromAutoHide()
                assistant.refreshConfigStatus()
                cicada.refreshFromConfig()
            }
            ACTION_TEST_TOAST -> {
                revealFromAutoHide()
                LiveNotificationStore.pushToast(
                    "Zekrima",
                    "Dynamic Island v0.4.0 Aktif!",
                    "Zeki & Cicada Mode siap digunakan di Android."
                )
            }
            ACTION_OPEN_CHAT -> {
                revealFromAutoHide()
                tab = ExpandedTab.ZEKI
                expanded = true
                setChatBarVisible(true)
                applyPillMode()
                ensureAnimating()
            }
            ACTION_START_VOICE -> {
                revealFromAutoHide()
                tab = ExpandedTab.ZEKI
                expanded = true
                assistant.startVoiceListening(cicada.buildQaContext())
                applyPillMode()
                ensureAnimating()
            }
            ACTION_TOGGLE_CICADA -> {
                revealFromAutoHide()
                if (cicada.snapshot().active) {
                    cicada.stop()
                    cicadaSubtitleBar = false
                    cicadaClickThrough = false
                } else {
                    tab = ExpandedTab.CICADA
                    cicada.start(ZekrimaApp.activeMediaProjection)
                    cicadaSubtitleBar = true
                    expanded = false
                }
                applyPillMode()
                updateWindowFlags()
                ensureAnimating()
            }
            ACTION_PROJECTION_READY -> {
                // Upgrade ForegroundService type to mediaProjection on Android 14+ BEFORE creating projection!
                startForegroundCompat(includeMediaProjection = true)
                val code = ZekrimaApp.mediaProjectionResultCode
                val data = ZekrimaApp.mediaProjectionData
                if (data != null && code != 0) {
                    try {
                        val mpm = getSystemService(Context.MEDIA_PROJECTION_SERVICE) as MediaProjectionManager
                        ZekrimaApp.activeMediaProjection = mpm.getMediaProjection(code, data)
                    } catch (_: Throwable) {
                    }
                }
                if (ZekrimaApp.pendingProjectionPurpose == "cicada") {
                    revealFromAutoHide()
                    cicada.stop()
                    cicada.start(ZekrimaApp.activeMediaProjection)
                    cicadaSubtitleBar = true
                    expanded = false
                    applyPillMode()
                    ensureAnimating()
                }
                ZekrimaApp.notifyStateChanged()
            }
            ACTION_UNLOCK_SUBTITLE -> {
                cicadaClickThrough = false
                updateWindowFlags()
                renderNow(0.016f)
            }
        }
        return START_STICKY
    }

    override fun onDestroy() {
        handler.removeCallbacks(oneSecondTicker)
        animating = false
        ZekrimaApp.listeners.remove(stateListener)
        clipboardManager?.removePrimaryClipChangedListener(clipListener)
        cicada.stop()
        assistant.cancel()
        try {
            ZekrimaApp.activeMediaProjection?.stop()
            ZekrimaApp.activeMediaProjection = null
        } catch (_: Throwable) {
        }
        try {
            wm.removeView(rootContainer)
        } catch (_: Throwable) {
        }
        ZekrimaApp.overlayRunning = false
        ZekrimaApp.notifyStateChanged()
        super.onDestroy()
    }

    private fun startForegroundCompat(includeMediaProjection: Boolean) {
        val openIntent = PendingIntent.getActivity(
            this, 0, Intent(this, MainActivity::class.java),
            PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT
        )
        val unlockIntent = PendingIntent.getService(
            this, 1, Intent(this, IslandOverlayService::class.java).setAction(ACTION_UNLOCK_SUBTITLE),
            PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT
        )
        val stopIntent = PendingIntent.getService(
            this, 2, Intent(this, IslandOverlayService::class.java).setAction(ACTION_STOP),
            PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT
        )
        val notif: Notification = NotificationCompat.Builder(this, ZekrimaApp.CHANNEL_OVERLAY_ID)
            .setSmallIcon(R.drawable.ic_zekrima_tile)
            .setContentTitle("Zekrima Dynamic Island Aktif")
            .setContentText("Ketuk untuk pengaturan • Cicada & Zeki siap")
            .setContentIntent(openIntent)
            .setOngoing(true)
            .addAction(0, "Unlock Bar", unlockIntent)
            .addAction(0, "Stop", stopIntent)
            .build()

        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.UPSIDE_DOWN_CAKE) {
            var types = ServiceInfo.FOREGROUND_SERVICE_TYPE_SPECIAL_USE
            if (includeMediaProjection) {
                types = types or ServiceInfo.FOREGROUND_SERVICE_TYPE_MEDIA_PROJECTION
            }
            try {
                startForeground(NOTIF_ID, notif, types)
            } catch (_: Throwable) {
                startForeground(NOTIF_ID, notif)
            }
        } else {
            startForeground(NOTIF_ID, notif)
        }
    }

    private fun buildOverlayViews() {
        val d = resources.displayMetrics.density
        rootContainer = LinearLayout(this).apply {
            orientation = LinearLayout.VERTICAL
            gravity = Gravity.CENTER_HORIZONTAL
            setBackgroundColor(Color.TRANSPARENT)
        }

        canvasView = IslandCanvasView(this) { action -> handleHitAction(action) }
        rootContainer.addView(
            canvasView,
            LinearLayout.LayoutParams(
                (maxWidthDp * d).toInt(),
                ((PillGeometry.COLLAPSED_HEIGHT + PillGeometry.PILL_TOP_DP + 6f) * d).toInt()
            )
        )

        // Floating Chat Input bar (only shown when user opens chat keyboard)
        chatBar = LinearLayout(this).apply {
            orientation = LinearLayout.HORIZONTAL
            gravity = Gravity.CENTER_VERTICAL
            visibility = View.GONE
            setPadding((10 * d).toInt(), (6 * d).toInt(), (10 * d).toInt(), (6 * d).toInt())
            background = GradientDrawable().apply {
                setColor(Color.argb(245, 18, 22, 32))
                cornerRadius = 18 * d
                setStroke((1 * d).toInt(), Color.argb(120, 56, 189, 248))
            }
        }
        chatInput = EditText(this).apply {
            hint = "Tanya Zeki..."
            setHintTextColor(Color.rgb(148, 163, 184))
            setTextColor(Color.WHITE)
            textSize = 13f
            setBackgroundColor(Color.TRANSPARENT)
            isSingleLine = true
            imeOptions = EditorInfo.IME_ACTION_SEND
            setOnEditorActionListener { _, actionId, _ ->
                if (actionId == EditorInfo.IME_ACTION_SEND) {
                    submitChat()
                    true
                } else false
            }
        }
        val sendBtn = Button(this).apply {
            text = "Kirim"
            textSize = 11f
            setTextColor(Color.rgb(8, 10, 14))
            background = GradientDrawable().apply {
                setColor(Color.rgb(56, 189, 248))
                cornerRadius = 14 * d
            }
            setOnClickListener { submitChat() }
        }
        val closeBtn = Button(this).apply {
            text = "×"
            textSize = 14f
            setTextColor(Color.WHITE)
            setBackgroundColor(Color.TRANSPARENT)
            setOnClickListener { setChatBarVisible(false) }
        }
        chatBar.addView(chatInput, LinearLayout.LayoutParams(0, LinearLayout.LayoutParams.WRAP_CONTENT, 1f))
        chatBar.addView(sendBtn, LinearLayout.LayoutParams((62 * d).toInt(), (34 * d).toInt()))
        chatBar.addView(closeBtn, LinearLayout.LayoutParams((36 * d).toInt(), (34 * d).toInt()))

        val barLp = LinearLayout.LayoutParams(
            ((maxWidthDp - 12f) * d).toInt(),
            LinearLayout.LayoutParams.WRAP_CONTENT
        ).apply { topMargin = (6 * d).toInt() }
        rootContainer.addView(chatBar, barLp)

        layoutParams = WindowManager.LayoutParams(
            WindowManager.LayoutParams.WRAP_CONTENT,
            WindowManager.LayoutParams.WRAP_CONTENT,
            WindowManager.LayoutParams.TYPE_APPLICATION_OVERLAY,
            WindowManager.LayoutParams.FLAG_NOT_FOCUSABLE or
                WindowManager.LayoutParams.FLAG_LAYOUT_IN_SCREEN or
                WindowManager.LayoutParams.FLAG_NOT_TOUCH_MODAL,
            PixelFormat.TRANSLUCENT
        ).apply {
            gravity = Gravity.TOP or Gravity.CENTER_HORIZONTAL
            y = 0
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.P) {
                layoutInDisplayCutoutMode =
                    WindowManager.LayoutParams.LAYOUT_IN_DISPLAY_CUTOUT_MODE_SHORT_EDGES
            }
        }

        if (Settings.canDrawOverlays(this)) {
            wm.addView(rootContainer, layoutParams)
        }
    }

    private fun submitChat() {
        val txt = chatInput.text?.toString()?.trim().orEmpty()
        if (txt.isEmpty()) return
        chatInput.setText("")
        setChatBarVisible(false)
        assistant.sendText(txt, cicada.buildQaContext())
        applyPillMode()
        ensureAnimating()
    }

    private fun setChatBarVisible(show: Boolean) {
        chatBarVisible = show
        chatBar.visibility = if (show) View.VISIBLE else View.GONE
        updateWindowFlags()
        if (show) {
            chatInput.requestFocus()
            handler.postDelayed({
                val imm = getSystemService(Context.INPUT_METHOD_SERVICE) as? InputMethodManager
                imm?.showSoftInput(chatInput, InputMethodManager.SHOW_IMPLICIT)
            }, 80L)
        } else {
            val imm = getSystemService(Context.INPUT_METHOD_SERVICE) as? InputMethodManager
            imm?.hideSoftInputFromWindow(chatInput.windowToken, 0)
        }
    }

    private fun updateWindowFlags() {
        var flags = WindowManager.LayoutParams.FLAG_LAYOUT_IN_SCREEN or
            WindowManager.LayoutParams.FLAG_NOT_TOUCH_MODAL
        if (!chatBarVisible) {
            flags = flags or WindowManager.LayoutParams.FLAG_NOT_FOCUSABLE
        }
        if (cicadaSubtitleBar && cicadaClickThrough && !expanded) {
            flags = flags or WindowManager.LayoutParams.FLAG_NOT_TOUCHABLE
        }
        if (layoutParams.flags != flags) {
            layoutParams.flags = flags
            try {
                wm.updateViewLayout(rootContainer, layoutParams)
            } catch (_: Throwable) {
            }
        }
    }

    private fun updateWindowBounds() {
        val d = resources.displayMetrics.density
        val targetH = ((springs.height.current + PillGeometry.PILL_TOP_DP + 6f) * d).toInt()
            .coerceAtLeast((24 * d).toInt())
        val lp = canvasView.layoutParams
        if (lp.height != targetH) {
            lp.height = targetH
            canvasView.layoutParams = lp
        }
    }

    private fun revealFromAutoHide() {
        lastActivityMs = SystemClock.uptimeMillis()
        if (autoHidden) {
            autoHidden = false
            applyPillMode()
            ensureAnimating()
        }
    }

    private fun handleHitAction(action: HitAction) {
        revealFromAutoHide()
        zeki.recordInteraction()
        when (action) {
            HitAction.RevealAutoHide -> {
                expanded = true
                applyPillMode()
            }
            HitAction.Collapse -> {
                expanded = false
                setChatBarVisible(false)
                applyPillMode()
            }
            is HitAction.SelectTab -> {
                tab = action.tab
                expanded = true
                if (tab == ExpandedTab.SYS) {
                    sysStats = SystemTools.sampleStats(this)
                }
                if (tab != ExpandedTab.ZEKI) setChatBarVisible(false)
                applyPillMode()
            }
            HitAction.ZekiPoke -> zeki.poke()
            is HitAction.ZekiStateSelect -> zeki.setStateManual(action.state)
            HitAction.ZekiOpenChat -> {
                tab = ExpandedTab.ZEKI
                expanded = true
                setChatBarVisible(!chatBarVisible)
                applyPillMode()
            }
            HitAction.ZekiVoice -> {
                assistant.startVoiceListening(cicada.buildQaContext())
                applyPillMode()
            }
            HitAction.ZekiTogglePrivacy -> assistant.toggleScreenAccess()
            HitAction.ZekiOpenSettings -> {
                expanded = false
                setChatBarVisible(false)
                applyPillMode()
                startActivity(
                    Intent(this, MainActivity::class.java).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
                )
            }
            HitAction.ZekiCopyReply -> {
                val r = assistant.snapshot().reply
                if (r.isNotBlank()) copyToClipboard(r)
            }
            HitAction.ZekiClearChat -> {
                setChatBarVisible(false)
                assistant.clearConversation()
                applyPillMode()
            }
            HitAction.CicadaToggleListen -> {
                if (cicada.snapshot().active) {
                    cicada.stop()
                    cicadaClickThrough = false
                } else {
                    cicada.start(ZekrimaApp.activeMediaProjection)
                }
                applyPillMode()
                updateWindowFlags()
            }
            HitAction.CicadaToggleBar -> {
                cicadaSubtitleBar = !cicadaSubtitleBar
                if (cicadaSubtitleBar) {
                    expanded = false
                    setChatBarVisible(false)
                } else {
                    cicadaClickThrough = false
                    expanded = true
                    tab = ExpandedTab.CICADA
                }
                applyPillMode()
                updateWindowFlags()
            }
            HitAction.CicadaTogglePass -> {
                cicadaClickThrough = !cicadaClickThrough
                updateWindowFlags()
            }
            HitAction.CicadaToggleMic -> cicada.toggleMic()
            HitAction.CicadaQuickReply -> cicada.toggleQuickReply()
            HitAction.CicadaCycleLang -> cicada.cycleTargetLanguage()
            HitAction.CicadaToggleOrig -> cicada.toggleShowOriginal()
            HitAction.CicadaCopyMd -> {
                val md = cicada.formatMarkdown()
                if (md.isNotBlank()) {
                    cicada.saveMarkdownFile()
                    copyToClipboard(md)
                }
            }
            HitAction.CicadaSummary -> {
                val prompt = cicada.buildSummaryPrompt()
                if (prompt != null) {
                    cicada.saveMarkdownFile()
                    cicadaSubtitleBar = false
                    cicadaClickThrough = false
                    tab = ExpandedTab.ZEKI
                    expanded = true
                    assistant.sendText(prompt, cicada.buildQaContext())
                    applyPillMode()
                    updateWindowFlags()
                }
            }
            HitAction.CicadaAskZeki -> {
                cicadaSubtitleBar = false
                cicadaClickThrough = false
                tab = ExpandedTab.ZEKI
                expanded = true
                setChatBarVisible(true)
                applyPillMode()
                updateWindowFlags()
            }
            HitAction.CicadaClear -> cicada.clearTranscript()
            is HitAction.MediaControl -> LiveNotificationStore.controlMedia(action.action)
            HitAction.TimerToggle -> timer.toggleStartPause()
            HitAction.TimerReset -> timer.reset()
            is HitAction.TimerPreset -> timer.selectPreset(action.secs)
            is HitAction.ClipCopy -> clipboardItems.getOrNull(action.index)?.let { copyToClipboard(it) }
            HitAction.ClipClear -> clipboardItems.clear()
            is HitAction.AdjustVolume -> {
                val next = (sysStats.volumePercent + action.delta).coerceIn(0, 100)
                SystemTools.setVolume(this, next, null)
                sysStats = SystemTools.sampleStats(this)
                zeki.triggerActionHappy()
            }
            is HitAction.AdjustBrightness -> {
                val next = (sysStats.brightnessPercent + action.delta).coerceIn(5, 100)
                SystemTools.setBrightness(this, next)
                sysStats = SystemTools.sampleStats(this)
                zeki.triggerActionHappy()
            }
            is HitAction.NotifOpen -> {
                val item = LiveNotificationStore.list().firstOrNull { it.key == action.key }
                try {
                    item?.contentIntent?.send()
                } catch (_: Throwable) {
                }
                LiveNotificationStore.dismiss(action.key)
            }
            is HitAction.NotifDismiss -> LiveNotificationStore.dismiss(action.key)
            HitAction.NotifClearAll -> LiveNotificationStore.clearAll()
            HitAction.ToastDismiss -> {
                LiveNotificationStore.clearToast()
                applyPillMode()
            }
        }
        renderNow(0.016f)
        ensureAnimating()
    }

    private fun copyToClipboard(text: String) {
        clipboardManager?.setPrimaryClip(ClipData.newPlainText("Zekrima", text))
        copiedFlashUntilMs = SystemClock.uptimeMillis() + 1_500L
        zeki.triggerActionHappy()
    }

    private fun onExternalStateChanged() {
        val aSnap = assistant.snapshot()
        val cSnap = cicada.snapshot()

        // Sync Assistant phase -> Zeki state
        when (aSnap.phase) {
            AssistantPhase.LISTENING -> zeki.setStateFromAssistant(ZekiState.LISTENING)
            AssistantPhase.TRANSCRIBING, AssistantPhase.THINKING -> zeki.setStateFromAssistant(ZekiState.THINKING)
            AssistantPhase.SPEAKING -> zeki.setStateFromAssistant(ZekiState.SPEAKING)
            AssistantPhase.DONE -> {
                zeki.setStateFromAssistant(ZekiState.HAPPY)
                if (aSnap.reply.isNotBlank()) {
                    tab = ExpandedTab.ZEKI
                    expanded = true
                }
            }
            AssistantPhase.ERROR -> {
                zeki.setStateFromAssistant(ZekiState.ERROR)
                tab = ExpandedTab.ZEKI
                expanded = true
            }
            AssistantPhase.IDLE -> Unit
        }

        if (cSnap.arrivalSeq != lastCicadaArrivalSeq) {
            lastCicadaArrivalSeq = cSnap.arrivalSeq
            zeki.triggerNod()
        }
        if (LiveNotificationStore.isToastVisible() || aSnap.phase.isBusy() || cSnap.active) {
            revealFromAutoHide()
        }

        zeki.dancing = LiveNotificationStore.mediaInfo.hasSession && LiveNotificationStore.mediaInfo.isPlaying
        applyPillMode()
        renderNow(0.016f)
        ensureAnimating()
    }

    private fun applyPillMode() {
        val aSnap = assistant.snapshot()
        val cSnap = cicada.snapshot()
        val mode = when {
            autoHidden -> PillMode.AUTO_HIDDEN
            expanded && tab == ExpandedTab.ZEKI && (aSnap.hasConversationView() || chatBarVisible) ->
                PillMode.EXPANDED_CHAT
            expanded && tab == ExpandedTab.CICADA -> PillMode.EXPANDED_CICADA
            expanded -> PillMode.EXPANDED
            LiveNotificationStore.isToastVisible() -> PillMode.NOTIFICATION
            cicadaSubtitleBar -> PillMode.CICADA_SUBTITLE
            cSnap.active || cSnap.quickReplyRecording || aSnap.phase.isBusy() -> PillMode.ASSISTANT
            LiveNotificationStore.unreadCount() > 0 -> PillMode.COLLAPSED_WITH_BADGE
            else -> PillMode.COLLAPSED
        }
        springs.setMode(mode, maxWidthDp)
    }

    private fun syncZekiLiveLevels(dt: Float) {
        zeki.liveVoiceLevel = if (assistant.isListening()) assistant.liveVoiceLevel() else null
        val cSnap = cicada.snapshot()
        val acc = when {
            expanded && tab == ExpandedTab.CICADA -> ZekiCicadaAccessory.TAKING_NOTES
            cSnap.active || cSnap.quickReplyRecording || cicadaSubtitleBar -> ZekiCicadaAccessory.HEADSET
            else -> ZekiCicadaAccessory.NONE
        }
        zeki.setCicadaMode(acc, cicada.liveAudioLevel)
        zeki.update(dt)
    }

    private fun needsContinuousAnimation(): Boolean {
        if (autoHidden) return !springs.isSettled()
        val cSnap = cicada.snapshot()
        return !springs.isSettled() ||
            expanded ||
            cicadaSubtitleBar ||
            cSnap.active ||
            cSnap.quickReplyRecording ||
            assistant.snapshot().phase.isBusy() ||
            LiveNotificationStore.isToastVisible() ||
            LiveNotificationStore.mediaInfo.isPlaying ||
            zeki.isHighEnergy()
    }

    private fun ensureAnimating() {
        if (!animating) {
            animating = true
            lastFrameNanos = 0L
            Choreographer.getInstance().postFrameCallback(frameCallback)
        }
    }

    private fun renderNow(dt: Float) {
        val renderState = IslandRenderState(
            widthDp = springs.width.current,
            heightDp = springs.height.current,
            radiusDp = springs.radius.current,
            autoHidden = autoHidden,
            expanded = expanded,
            tab = tab,
            chatOpen = chatBarVisible,
            zeki = zeki,
            assistant = assistant.snapshot(),
            cicada = cicada.snapshot(),
            cicadaAudioLevel = cicada.liveAudioLevel,
            cicadaSubtitleBar = cicadaSubtitleBar,
            cicadaClickThrough = cicadaClickThrough,
            media = LiveNotificationStore.mediaInfo,
            timer = timer,
            clipItems = clipboardItems.toList(),
            sysStats = sysStats,
            notifications = LiveNotificationStore.list(),
            toast = if (LiveNotificationStore.isToastVisible()) LiveNotificationStore.latestToast else null,
            copiedFlash = SystemClock.uptimeMillis() < copiedFlashUntilMs
        )
        canvasView.updateState(renderState, dt)
    }
}
