package io.zekrima.assistant.render

import android.content.Context
import android.graphics.Canvas
import android.graphics.Color
import android.graphics.Paint
import android.graphics.RectF
import android.view.MotionEvent
import android.view.View
import io.zekrima.assistant.config.cicadaLangBadge
import io.zekrima.assistant.services.AssistantPhase
import io.zekrima.assistant.services.AssistantSnapshot
import io.zekrima.assistant.services.CicadaSnapshot
import io.zekrima.assistant.services.IslandNotification
import io.zekrima.assistant.services.MediaControlAction
import io.zekrima.assistant.services.MediaInfo
import io.zekrima.assistant.services.PomodoroTimer
import io.zekrima.assistant.services.SpeakerChannel
import io.zekrima.assistant.services.SystemStats
import java.text.SimpleDateFormat
import java.util.Date
import java.util.Locale
import kotlin.math.abs
import kotlin.math.sin

enum class ExpandedTab(val label: String) {
    ZEKI("Zeki"),
    CICADA("Live"),
    MEDIA("Media"),
    TIMER("Timer"),
    CLIP("Clip"),
    SYS("Sys"),
    NOTIFS("Notifs")
}

sealed class HitAction {
    data class SelectTab(val tab: ExpandedTab) : HitAction()
    data object Collapse : HitAction()
    data object RevealAutoHide : HitAction()
    data object ZekiPoke : HitAction()
    data class ZekiStateSelect(val state: ZekiState) : HitAction()
    data object ZekiOpenChat : HitAction()
    data object ZekiVoice : HitAction()
    data object ZekiTogglePrivacy : HitAction()
    data object ZekiOpenSettings : HitAction()
    data object ZekiCopyReply : HitAction()
    data object ZekiClearChat : HitAction()
    data object CicadaToggleListen : HitAction()
    data object CicadaToggleBar : HitAction()
    data object CicadaTogglePass : HitAction()
    data object CicadaToggleMic : HitAction()
    data object CicadaQuickReply : HitAction()
    data object CicadaCycleLang : HitAction()
    data object CicadaToggleOrig : HitAction()
    data object CicadaCopyMd : HitAction()
    data object CicadaSummary : HitAction()
    data object CicadaAskZeki : HitAction()
    data object CicadaClear : HitAction()
    data class MediaControl(val action: MediaControlAction) : HitAction()
    data object TimerToggle : HitAction()
    data object TimerReset : HitAction()
    data class TimerPreset(val secs: Int) : HitAction()
    data class ClipCopy(val index: Int) : HitAction()
    data object ClipClear : HitAction()
    data class AdjustVolume(val delta: Int) : HitAction()
    data class AdjustBrightness(val delta: Int) : HitAction()
    data class NotifOpen(val key: String) : HitAction()
    data class NotifDismiss(val key: String) : HitAction()
    data object NotifClearAll : HitAction()
    data object ToastDismiss : HitAction()
}

data class IslandRenderState(
    val widthDp: Float,
    val heightDp: Float,
    val radiusDp: Float,
    val autoHidden: Boolean,
    val expanded: Boolean,
    val tab: ExpandedTab,
    val chatOpen: Boolean,
    val zeki: ZekiEngine,
    val assistant: AssistantSnapshot,
    val cicada: CicadaSnapshot,
    val cicadaAudioLevel: Float,
    val cicadaSubtitleBar: Boolean,
    val cicadaClickThrough: Boolean,
    val media: MediaInfo,
    val timer: PomodoroTimer,
    val clipItems: List<String>,
    val sysStats: SystemStats,
    val notifications: List<IslandNotification>,
    val toast: IslandNotification?,
    val copiedFlash: Boolean
)

/**
 * Pure Android Canvas renderer for Zekrima Dynamic Island with crisp obsidian aesthetics,
 * smooth spring geometry, 7-tab navigation, and zero-overhead interactive hit zones.
 */
class IslandCanvasView(
    context: Context,
    private val onAction: (HitAction) -> Unit
) : View(context) {

    private val density = resources.displayMetrics.density
    private fun dp(v: Float): Float = v * density

    private val fillPaint = Paint(Paint.ANTI_ALIAS_FLAG).apply { style = Paint.Style.FILL }
    private val strokePaint = Paint(Paint.ANTI_ALIAS_FLAG).apply { style = Paint.Style.STROKE }
    private val textPaint = Paint(Paint.ANTI_ALIAS_FLAG)
    private val timeFmt = SimpleDateFormat("HH:mm", Locale.getDefault())

    private val hitTargets = mutableListOf<Pair<RectF, HitAction>>()
    private var state: IslandRenderState? = null
    private var animPhase = 0f
    private var downX = 0f
    private var downY = 0f

    fun updateState(next: IslandRenderState, dt: Float) {
        state = next
        animPhase += dt
        invalidate()
    }

    override fun onTouchEvent(event: MotionEvent): Boolean {
        val st = state ?: return false
        when (event.actionMasked) {
            MotionEvent.ACTION_DOWN -> {
                downX = event.x
                downY = event.y
                return true
            }
            MotionEvent.ACTION_UP -> {
                val dx = event.x - downX
                val dy = event.y - downY
                if (st.autoHidden) {
                    onAction(HitAction.RevealAutoHide)
                    return true
                }
                // Swipe up anywhere on expanded island -> collapse smoothly
                if (st.expanded && dy < -dp(36f) && abs(dy) > abs(dx)) {
                    onAction(HitAction.Collapse)
                    return true
                }
                // Check registered hit targets (last added wins)
                for (i in hitTargets.indices.reversed()) {
                    val (rect, act) = hitTargets[i]
                    if (rect.contains(event.x, event.y)) {
                        onAction(act)
                        return true
                    }
                }
                // Tapping empty space on collapsed pill expands; tapping header collapses
                if (!st.expanded) {
                    if (st.toast != null) {
                        onAction(HitAction.ToastDismiss)
                    } else {
                        onAction(HitAction.SelectTab(st.tab))
                    }
                } else if (event.y < dp(PillGeometry.PILL_TOP_DP + 24f)) {
                    onAction(HitAction.Collapse)
                }
                return true
            }
        }
        return super.onTouchEvent(event)
    }

    override fun onDraw(canvas: Canvas) {
        super.onDraw(canvas)
        val st = state ?: return
        hitTargets.clear()

        val wPx = dp(st.widthDp).coerceAtMost(width.toFloat() - dp(8f)).coerceAtLeast(dp(40f))
        val hPx = dp(st.heightDp).coerceAtLeast(dp(4.5f))
        val rPx = dp(st.radiusDp).coerceAtMost(minOf(wPx, hPx) / 2f)
        val left = (width - wPx) / 2f

        val tuck = ((PillGeometry.COLLAPSED_HEIGHT - st.heightDp) /
            (PillGeometry.COLLAPSED_HEIGHT - PillGeometry.AUTO_HIDDEN_HEIGHT)).coerceIn(0f, 1f)
        val top = dp(PillGeometry.PILL_TOP_DP) * (1f - tuck)
        val pillRect = RectF(left, top, left + wPx, top + hPx)

        // Tucked Auto-Hidden top-edge handle
        if (st.heightDp < 18f) {
            fillPaint.color = Color.argb(220, 12, 13, 18)
            canvas.drawRoundRect(pillRect, rPx, rPx, fillPaint)
            strokePaint.color = Color.argb(55, 255, 255, 255)
            strokePaint.strokeWidth = dp(0.8f)
            canvas.drawRoundRect(pillRect, rPx, rPx, strokePaint)

            val gripW = (wPx * 0.34f).coerceIn(dp(26f), dp(46f))
            val gripLeft = left + (wPx - gripW) / 2f
            val gripY = top + (hPx - dp(1.5f)) * 0.55f
            fillPaint.color = Color.argb(115, 255, 255, 255)
            canvas.drawRoundRect(RectF(gripLeft, gripY, gripLeft + gripW, gripY + dp(1.5f)), dp(0.75f), dp(0.75f), fillPaint)
            return
        }

        // Main capsule background
        val bgAlpha = if (st.cicadaSubtitleBar && !st.expanded) {
            if (st.cicadaClickThrough) 205 else 228
        } else 246
        fillPaint.color = Color.argb(bgAlpha, 10, 10, 14)
        canvas.drawRoundRect(pillRect, rPx, rPx, fillPaint)

        // Subtle outer border
        strokePaint.color = when {
            st.cicada.active -> Color.argb(95, 16, 185, 129)
            st.assistant.phase.isBusy() -> Color.argb(110, 56, 189, 248)
            else -> Color.argb(32, 255, 255, 255)
        }
        strokePaint.strokeWidth = dp(1f)
        canvas.drawRoundRect(pillRect, rPx, rPx, strokePaint)

        // Top accent highlight line
        val accentInset = rPx.coerceAtLeast(dp(16f))
        if (wPx > accentInset * 2f + dp(8f)) {
            fillPaint.color = when {
                st.cicada.active -> Color.argb(180, 16, 185, 129)
                st.assistant.phase.isBusy() -> Color.argb(190, 56, 189, 248)
                st.toast != null -> Color.argb(180, 235, 0, 40)
                else -> Color.argb(40, 255, 255, 255)
            }
            canvas.drawRoundRect(
                RectF(left + accentInset, top + dp(1f), left + wPx - accentInset, top + dp(2.4f)),
                dp(0.8f), dp(0.8f), fillPaint
            )
        }

        canvas.save()
        canvas.clipRect(pillRect)
        when {
            st.expanded && st.heightDp > 120f -> drawExpanded(canvas, pillRect, st)
            st.cicadaSubtitleBar && !st.expanded && st.heightDp > 60f -> drawCicadaSubtitleBar(canvas, pillRect, st)
            st.toast != null && !st.expanded && st.heightDp > 56f -> drawToast(canvas, pillRect, st)
            else -> drawCollapsed(canvas, pillRect, st)
        }
        canvas.restore()
    }

    private fun drawCollapsed(canvas: Canvas, pill: RectF, st: IslandRenderState) {
        val cy = pill.centerY()
        val zekiR = dp(10.5f)
        st.zeki.draw(canvas, pill.left + dp(20f), cy, zekiR)

        // Live Cicada collapsed indicator
        if (st.cicada.active || st.cicada.quickReplyRecording) {
            val pulse = (140 + (115 * abs(sin(animPhase * 4f))).toInt()).coerceIn(80, 255)
            fillPaint.color = Color.argb(pulse, 16, 185, 129)
            canvas.drawCircle(pill.left + dp(40f), cy, dp(3.5f), fillPaint)

            textPaint.color = Color.rgb(167, 243, 208)
            textPaint.textSize = dp(11f)
            textPaint.isFakeBoldText = true
            textPaint.textAlign = Paint.Align.LEFT
            val badge = if (st.cicada.quickReplyRecording) "REPLY" else "LIVE ${cicadaLangBadge(st.cicada.targetLang)}"
            canvas.drawText(badge, pill.left + dp(49f), cy + dp(4f), textPaint)

            drawMiniWaveform(canvas, pill.right - dp(40f), cy, st.cicadaAudioLevel, Color.rgb(16, 185, 129))
            return
        }

        // Active assistant state pill
        if (st.assistant.phase.isBusy()) {
            textPaint.color = Color.WHITE
            textPaint.textSize = dp(11.5f)
            textPaint.isFakeBoldText = true
            textPaint.textAlign = Paint.Align.LEFT
            val lbl = when (st.assistant.phase) {
                AssistantPhase.LISTENING -> "Listening..."
                AssistantPhase.TRANSCRIBING -> "Transcribing..."
                AssistantPhase.THINKING -> "Thinking..."
                AssistantPhase.SPEAKING -> "Speaking..."
                else -> "Zeki"
            }
            canvas.drawText(lbl, pill.left + dp(39f), cy + dp(4f), textPaint)
            return
        }

        // Default collapsed: Clock + Pomodoro / Media EQ / Unread Badge
        textPaint.color = Color.WHITE
        textPaint.textSize = dp(12.5f)
        textPaint.isFakeBoldText = true
        textPaint.textAlign = Paint.Align.LEFT
        val centerText = if (st.timer.running) st.timer.formatted() else timeFmt.format(Date())
        canvas.drawText(centerText, pill.left + dp(40f), cy + dp(4.5f), textPaint)

        if (st.media.hasSession && st.media.isPlaying) {
            drawMiniWaveform(canvas, pill.right - dp(38f), cy, 0.65f, Color.rgb(56, 189, 248))
        } else if (st.notifications.isNotEmpty()) {
            val badgeR = dp(8.5f)
            val bx = pill.right - dp(18f)
            fillPaint.color = Color.rgb(239, 68, 68)
            canvas.drawCircle(bx, cy, badgeR, fillPaint)
            textPaint.color = Color.WHITE
            textPaint.textSize = dp(9.5f)
            textPaint.textAlign = Paint.Align.CENTER
            canvas.drawText(st.notifications.size.coerceAtMost(9).toString(), bx, cy + dp(3.5f), textPaint)
        }
    }

    private fun drawMiniWaveform(canvas: Canvas, startX: Float, cy: Float, level: Float, color: Int) {
        fillPaint.color = color
        val barW = dp(2.4f)
        val gap = dp(2.6f)
        for (i in 0 until 5) {
            val wave = 0.25f + (level.coerceAtLeast(0.18f)) * abs(sin(animPhase * 9f + i * 1.15f))
            val barH = dp(4f + wave * 12f)
            val x = startX + i * (barW + gap)
            canvas.drawRoundRect(RectF(x, cy - barH / 2f, x + barW, cy + barH / 2f), barW / 2f, barW / 2f, fillPaint)
        }
    }

    private fun drawToast(canvas: Canvas, pill: RectF, st: IslandRenderState) {
        val toast = st.toast ?: return
        val avatarCx = pill.left + dp(28f)
        val avatarCy = pill.centerY()
        fillPaint.color = Color.rgb(30, 41, 59)
        canvas.drawCircle(avatarCx, avatarCy, dp(16f), fillPaint)
        textPaint.color = Color.rgb(56, 189, 248)
        textPaint.textSize = dp(13f)
        textPaint.isFakeBoldText = true
        textPaint.textAlign = Paint.Align.CENTER
        canvas.drawText(toast.appName.take(1).uppercase(), avatarCx, avatarCy + dp(5f), textPaint)

        val textX = pill.left + dp(54f)
        val maxW = pill.width() - dp(92f)
        textPaint.textAlign = Paint.Align.LEFT
        textPaint.color = Color.rgb(148, 163, 184)
        textPaint.textSize = dp(9.5f)
        textPaint.isFakeBoldText = true
        canvas.drawText(ellipsize(toast.appName, maxW), textX, pill.top + dp(22f), textPaint)

        textPaint.color = Color.WHITE
        textPaint.textSize = dp(12f)
        canvas.drawText(ellipsize(toast.title.ifEmpty { toast.body }, maxW), textX, pill.top + dp(40f), textPaint)

        textPaint.color = Color.rgb(203, 213, 225)
        textPaint.textSize = dp(10.5f)
        textPaint.isFakeBoldText = false
        canvas.drawText(ellipsize(toast.body, maxW), textX, pill.top + dp(57f), textPaint)

        val xBtn = RectF(pill.right - dp(32f), pill.top + dp(10f), pill.right - dp(10f), pill.top + dp(32f))
        drawPillButton(canvas, xBtn, "×", Color.argb(40, 255, 255, 255), Color.WHITE, 12f)
        hitTargets.add(xBtn to HitAction.ToastDismiss)
    }

    private fun drawCicadaSubtitleBar(canvas: Canvas, pill: RectF, st: IslandRenderState) {
        st.zeki.draw(canvas, pill.left + dp(24f), pill.centerY(), dp(14f))

        // Top-right compact controls: Pass | Expand | Stop
        val btnH = dp(20f)
        val btnY = pill.top + dp(8f)
        val stopRect = RectF(pill.right - dp(44f), btnY, pill.right - dp(8f), btnY + btnH)
        val expRect = RectF(stopRect.left - dp(44f), btnY, stopRect.left - dp(4f), btnY + btnH)
        val passRect = RectF(expRect.left - dp(46f), btnY, expRect.left - dp(4f), btnY + btnH)

        drawPillButton(
            canvas, passRect, if (st.cicadaClickThrough) "Lock" else "Pass",
            if (st.cicadaClickThrough) Color.argb(170, 245, 158, 11) else Color.argb(55, 255, 255, 255),
            Color.WHITE, 9f
        )
        drawPillButton(canvas, expRect, "Open", Color.argb(55, 56, 189, 248), Color.WHITE, 9f)
        drawPillButton(
            canvas, stopRect, if (st.cicada.active) "Stop" else "Go",
            if (st.cicada.active) Color.argb(180, 239, 68, 68) else Color.argb(180, 16, 185, 129),
            Color.WHITE, 9f
        )
        hitTargets.add(passRect to HitAction.CicadaTogglePass)
        hitTargets.add(expRect to HitAction.CicadaToggleBar)
        hitTargets.add(stopRect to HitAction.CicadaToggleListen)

        val tx = pill.left + dp(48f)
        val maxW = pill.width() - dp(58f)
        val latest = st.cicada.latest()

        textPaint.textAlign = Paint.Align.LEFT
        if (latest == null) {
            textPaint.color = Color.rgb(167, 243, 208)
            textPaint.textSize = dp(11.5f)
            textPaint.isFakeBoldText = true
            canvas.drawText(ellipsize(st.cicada.statusNote, maxW), tx, pill.centerY() + dp(6f), textPaint)
        } else {
            val tagColor = when (latest.speaker) {
                SpeakerChannel.ONLINE -> Color.rgb(52, 211, 153)
                SpeakerChannel.YOU -> Color.rgb(56, 189, 248)
                SpeakerChannel.QUICK_REPLY -> Color.rgb(250, 204, 21)
            }
            textPaint.color = tagColor
            textPaint.textSize = dp(9.5f)
            textPaint.isFakeBoldText = true
            canvas.drawText("${latest.speaker.label} • ${latest.sourceLang}→${cicadaLangBadge(st.cicada.targetLang)}", tx, pill.top + dp(22f), textPaint)

            // Translated primary subtitle (up to 2 wrapped lines)
            textPaint.color = Color.WHITE
            textPaint.textSize = dp(12.5f)
            textPaint.isFakeBoldText = true
            val lines = wrapLines(latest.translation, maxW, 2)
            lines.forEachIndexed { idx, line ->
                canvas.drawText(line, tx, pill.top + dp(42f + idx * 15.5f), textPaint)
            }
            if (st.cicada.showOriginal && latest.original != latest.translation) {
                textPaint.color = Color.rgb(148, 163, 184)
                textPaint.textSize = dp(10f)
                textPaint.isFakeBoldText = false
                canvas.drawText(ellipsize(latest.original, maxW), tx, pill.bottom - dp(10f), textPaint)
            }
        }
    }

    private fun drawExpanded(canvas: Canvas, pill: RectF, st: IslandRenderState) {
        // 7-Tab navigation bar across the top
        val tabs = ExpandedTab.entries
        val padX = dp(10f)
        val barTop = pill.top + dp(10f)
        val barH = dp(24f)
        val tabW = (pill.width() - padX * 2f) / tabs.size

        tabs.forEachIndexed { idx, tab ->
            val r = RectF(pill.left + padX + idx * tabW + dp(1.5f), barTop, pill.left + padX + (idx + 1) * tabW - dp(1.5f), barTop + barH)
            val active = st.tab == tab
            val bg = when {
                active && tab == ExpandedTab.CICADA -> Color.argb(185, 16, 185, 129)
                active -> Color.argb(200, 56, 189, 248)
                else -> Color.argb(26, 255, 255, 255)
            }
            drawPillButton(canvas, r, tab.label, bg, if (active) Color.rgb(8, 10, 14) else Color.rgb(203, 213, 225), 9.5f)
            hitTargets.add(r to HitAction.SelectTab(tab))
        }

        val contentTop = barTop + barH + dp(8f)
        val content = RectF(pill.left + dp(12f), contentTop, pill.right - dp(12f), pill.bottom - dp(10f))

        when (st.tab) {
            ExpandedTab.ZEKI -> drawZekiTab(canvas, content, st)
            ExpandedTab.CICADA -> drawCicadaTab(canvas, content, st)
            ExpandedTab.MEDIA -> drawMediaTab(canvas, content, st)
            ExpandedTab.TIMER -> drawTimerTab(canvas, content, st)
            ExpandedTab.CLIP -> drawClipTab(canvas, content, st)
            ExpandedTab.SYS -> drawSysTab(canvas, content, st)
            ExpandedTab.NOTIFS -> drawNotifsTab(canvas, content, st)
        }
    }

    private fun drawZekiTab(canvas: Canvas, area: RectF, st: IslandRenderState) {
        // Top row: Zeki mascot stage + quick action pills
        val zekiCx = area.left + dp(28f)
        val zekiCy = area.top + dp(26f)
        st.zeki.draw(canvas, zekiCx, zekiCy, dp(20f))
        hitTargets.add(RectF(area.left, area.top, area.left + dp(56f), area.top + dp(54f)) to HitAction.ZekiPoke)

        val btnY = area.top + dp(4f)
        val btnH = dp(24f)
        val bx = area.left + dp(62f)
        val bw = (area.right - bx - dp(12f)) / 3f

        val chatR = RectF(bx, btnY, bx + bw, btnY + btnH)
        val micR = RectF(chatR.right + dp(6f), btnY, chatR.right + dp(6f) + bw, btnY + btnH)
        val cfgR = RectF(micR.right + dp(6f), btnY, area.right, btnY + btnH)

        drawPillButton(canvas, chatR, "Chat", Color.argb(180, 56, 189, 248), Color.rgb(8, 10, 14), 10f)
        drawPillButton(
            canvas, micR, if (st.assistant.phase == AssistantPhase.LISTENING) "Stop" else "Voice",
            if (st.assistant.phase == AssistantPhase.LISTENING) Color.argb(200, 239, 68, 68) else Color.argb(180, 16, 185, 129),
            Color.WHITE, 10f
        )
        drawPillButton(canvas, cfgR, "Config", Color.argb(42, 255, 255, 255), Color.WHITE, 10f)
        hitTargets.add(chatR to HitAction.ZekiOpenChat)
        hitTargets.add(micR to HitAction.ZekiVoice)
        hitTargets.add(cfgR to HitAction.ZekiOpenSettings)

        val row2Y = btnY + btnH + dp(6f)
        val privR = RectF(bx, row2Y, bx + bw * 1.45f, row2Y + dp(22f))
        val copyR = RectF(privR.right + dp(6f), row2Y, privR.right + dp(6f) + bw * 0.78f, row2Y + dp(22f))
        val clrR = RectF(copyR.right + dp(6f), row2Y, area.right, row2Y + dp(22f))

        drawPillButton(
            canvas, privR,
            if (st.assistant.screenEnabled) "● Screen ON" else "Incognito",
            if (st.assistant.screenEnabled) Color.argb(75, 16, 185, 129) else Color.argb(40, 255, 255, 255),
            Color.WHITE, 9.5f
        )
        drawPillButton(canvas, copyR, if (st.copiedFlash) "✓" else "Copy", Color.argb(40, 255, 255, 255), Color.WHITE, 9.5f)
        drawPillButton(canvas, clrR, "Clear", Color.argb(45, 239, 68, 68), Color.rgb(252, 165, 165), 9.5f)
        hitTargets.add(privR to HitAction.ZekiTogglePrivacy)
        hitTargets.add(copyR to HitAction.ZekiCopyReply)
        hitTargets.add(clrR to HitAction.ZekiClearChat)

        // Conversation card or mood showcase
        val card = RectF(area.left, area.top + dp(60f), area.right, area.bottom)
        fillPaint.color = Color.argb(140, 20, 24, 34)
        canvas.drawRoundRect(card, dp(14f), dp(14f), fillPaint)

        val hasConvo = st.assistant.hasConversationView() || st.chatOpen
        if (!hasConvo) {
            textPaint.textAlign = Paint.Align.LEFT
            textPaint.color = Color.rgb(203, 213, 225)
            textPaint.textSize = dp(11f)
            textPaint.isFakeBoldText = true
            val intro = if (st.assistant.hasApiKey) {
                "Zeki siap! Tap Chat untuk mengetik atau Voice untuk bicara."
            } else {
                "Tap Config untuk isi API key gratis (Gemini / OpenRouter / Groq)."
            }
            val lines = wrapLines(intro, card.width() - dp(20f), 2)
            lines.forEachIndexed { idx, l ->
                canvas.drawText(l, card.left + dp(10f), card.top + dp(22f + idx * 16f), textPaint)
            }

            // 7 Mood chips along the bottom
            val moods = ZekiState.entries
            val chipW = (card.width() - dp(16f)) / moods.size
            val chipY = card.bottom - dp(28f)
            moods.forEachIndexed { i, mood ->
                val mr = RectF(card.left + dp(8f) + i * chipW + dp(1f), chipY, card.left + dp(8f) + (i + 1) * chipW - dp(1f), chipY + dp(20f))
                val active = st.zeki.state == mood
                drawPillButton(
                    canvas, mr, mood.label.take(4),
                    if (active) mood.accentColor else Color.argb(30, 255, 255, 255),
                    if (active) Color.rgb(10, 12, 16) else Color.rgb(203, 213, 225),
                    8.5f
                )
                hitTargets.add(mr to HitAction.ZekiStateSelect(mood))
            }
        } else {
            // Conversation feed
            textPaint.textAlign = Paint.Align.LEFT
            var y = card.top + dp(18f)
            val maxW = card.width() - dp(20f)
            if (st.assistant.error != null) {
                textPaint.color = Color.rgb(248, 113, 113)
                textPaint.textSize = dp(11f)
                textPaint.isFakeBoldText = true
                for (l in wrapLines("Error: ${st.assistant.error}", maxW, 4)) {
                    canvas.drawText(l, card.left + dp(10f), y, textPaint)
                    y += dp(15f)
                }
            } else {
                val recent = st.assistant.messages.takeLast(4)
                for (m in recent) {
                    if (y > card.bottom - dp(14f)) break
                    val isUser = m.role == "user"
                    textPaint.color = if (isUser) Color.rgb(56, 189, 248) else Color.rgb(241, 245, 249)
                    textPaint.textSize = dp(10.5f)
                    textPaint.isFakeBoldText = isUser
                    val prefix = if (isUser) "You: " else "Zeki: "
                    for (l in wrapLines(prefix + m.content, maxW, if (isUser) 2 else 5)) {
                        if (y > card.bottom - dp(8f)) break
                        canvas.drawText(l, card.left + dp(10f), y, textPaint)
                        y += dp(14.5f)
                    }
                    y += dp(4f)
                }
                if (st.assistant.phase.isBusy() && y <= card.bottom - dp(8f)) {
                    textPaint.color = Color.rgb(192, 132, 252)
                    textPaint.textSize = dp(10.5f)
                    textPaint.isFakeBoldText = true
                    canvas.drawText("● ${st.assistant.phase.name.lowercase()}...", card.left + dp(10f), y, textPaint)
                }
            }
            hitTargets.add(card to HitAction.ZekiOpenChat)
        }
    }

    private fun drawCicadaTab(canvas: Canvas, area: RectF, st: IslandRenderState) {
        // Header: Zeki TakingNotes + Capture badge + Start/Stop + Bar
        st.zeki.draw(canvas, area.left + dp(22f), area.top + dp(20f), dp(15f))

        val b1Y = area.top + dp(2f)
        val bH = dp(22f)
        val sx = area.left + dp(46f)
        val startR = RectF(sx, b1Y, sx + dp(62f), b1Y + bH)
        val barR = RectF(startR.right + dp(4f), b1Y, startR.right + dp(52f), b1Y + bH)
        val micR = RectF(barR.right + dp(4f), b1Y, barR.right + dp(58f), b1Y + bH)
        val repR = RectF(micR.right + dp(4f), b1Y, area.right, b1Y + bH)

        drawPillButton(
            canvas, startR, if (st.cicada.active) "■ Stop" else "▶ Start",
            if (st.cicada.active) Color.argb(200, 239, 68, 68) else Color.argb(205, 16, 185, 129),
            Color.WHITE, 9.5f
        )
        drawPillButton(canvas, barR, "Bar", Color.argb(65, 56, 189, 248), Color.WHITE, 9.5f)
        drawPillButton(
            canvas, micR, if (st.cicada.micEnabled) "Mic:ON" else "Mic:OFF",
            if (st.cicada.micEnabled) Color.argb(150, 16, 185, 129) else Color.argb(38, 255, 255, 255),
            Color.WHITE, 9f
        )
        drawPillButton(
            canvas, repR, if (st.cicada.quickReplyRecording) "●Rec" else "Reply",
            if (st.cicada.quickReplyRecording) Color.argb(200, 245, 158, 11) else Color.argb(55, 192, 132, 252),
            Color.WHITE, 9.5f
        )
        hitTargets.add(startR to HitAction.CicadaToggleListen)
        hitTargets.add(barR to HitAction.CicadaToggleBar)
        hitTargets.add(micR to HitAction.CicadaToggleMic)
        hitTargets.add(repR to HitAction.CicadaQuickReply)

        // Row 2 controls: Lang | Orig | Copy MD | Summary | Ask Zeki | Clear
        val b2Y = b1Y + bH + dp(5f)
        val r2Btns = listOf(
            "→${cicadaLangBadge(st.cicada.targetLang)}" to HitAction.CicadaCycleLang,
            (if (st.cicada.showOriginal) "Orig:ON" else "Orig:OFF") to HitAction.CicadaToggleOrig,
            (if (st.copiedFlash) "✓ MD" else "Copy MD") to HitAction.CicadaCopyMd,
            "Summary" to HitAction.CicadaSummary,
            "Ask Zeki" to HitAction.CicadaAskZeki,
            "Clr" to HitAction.CicadaClear
        )
        val w2 = (area.width() - dp(46f)) / r2Btns.size
        r2Btns.forEachIndexed { i, (label, act) ->
            val r = RectF(sx + i * w2 + dp(1.5f), b2Y, sx + (i + 1) * w2 - dp(1.5f), b2Y + dp(20f))
            drawPillButton(canvas, r, label, Color.argb(36, 255, 255, 255), Color.rgb(226, 232, 240), 8.5f)
            hitTargets.add(r to act)
        }

        // Bilingual Transcript Card
        val card = RectF(area.left, b2Y + dp(26f), area.right, area.bottom)
        fillPaint.color = Color.argb(150, 16, 20, 30)
        canvas.drawRoundRect(card, dp(14f), dp(14f), fillPaint)

        textPaint.textAlign = Paint.Align.LEFT
        val maxW = card.width() - dp(18f)
        if (st.cicada.entries.isEmpty()) {
            textPaint.color = Color.rgb(148, 163, 184)
            textPaint.textSize = dp(11f)
            textPaint.isFakeBoldText = false
            var y = card.top + dp(24f)
            for (l in wrapLines("${st.cicada.captureModeLabel}: ${st.cicada.statusNote}", maxW, 3)) {
                canvas.drawText(l, card.left + dp(10f), y, textPaint)
                y += dp(16f)
            }
        } else {
            var y = card.top + dp(16f)
            for (e in st.cicada.entries.takeLast(6)) {
                if (y > card.bottom - dp(12f)) break
                textPaint.color = when (e.speaker) {
                    SpeakerChannel.ONLINE -> Color.rgb(52, 211, 153)
                    SpeakerChannel.YOU -> Color.rgb(56, 189, 248)
                    SpeakerChannel.QUICK_REPLY -> Color.rgb(250, 204, 21)
                }
                textPaint.textSize = dp(9f)
                textPaint.isFakeBoldText = true
                canvas.drawText("[${e.timestamp}] ${e.speaker.label} (${e.sourceLang})", card.left + dp(10f), y, textPaint)
                y += dp(13f)

                textPaint.color = Color.WHITE
                textPaint.textSize = dp(11f)
                textPaint.isFakeBoldText = true
                for (l in wrapLines(e.translation, maxW, 2)) {
                    if (y > card.bottom - dp(8f)) break
                    canvas.drawText(l, card.left + dp(10f), y, textPaint)
                    y += dp(14f)
                }
                if (st.cicada.showOriginal && e.original != e.translation && y <= card.bottom - dp(8f)) {
                    textPaint.color = Color.rgb(148, 163, 184)
                    textPaint.textSize = dp(9.5f)
                    textPaint.isFakeBoldText = false
                    canvas.drawText(ellipsize(e.original, maxW), card.left + dp(10f), y, textPaint)
                    y += dp(13f)
                }
                y += dp(4f)
            }
        }
    }

    private fun drawMediaTab(canvas: Canvas, area: RectF, st: IslandRenderState) {
        st.zeki.draw(canvas, area.left + dp(30f), area.top + dp(40f), dp(22f))
        textPaint.textAlign = Paint.Align.LEFT
        val tx = area.left + dp(66f)
        val maxW = area.right - tx - dp(8f)

        textPaint.color = Color.WHITE
        textPaint.textSize = dp(13.5f)
        textPaint.isFakeBoldText = true
        canvas.drawText(
            ellipsize(if (st.media.hasSession) st.media.title else "Tidak ada media aktif", maxW),
            tx, area.top + dp(30f), textPaint
        )
        textPaint.color = Color.rgb(148, 163, 184)
        textPaint.textSize = dp(11f)
        textPaint.isFakeBoldText = false
        canvas.drawText(
            ellipsize(if (st.media.hasSession) st.media.artist.ifEmpty { st.media.appName } else "Putar Spotify / YouTube Music", maxW),
            tx, area.top + dp(48f), textPaint
        )

        val btnY = area.bottom - dp(44f)
        val btnW = dp(76f)
        val btnH = dp(34f)
        val cx = area.centerX()
        val prevR = RectF(cx - btnW * 1.6f, btnY, cx - btnW * 0.6f, btnY + btnH)
        val playR = RectF(cx - btnW * 0.5f, btnY, cx + btnW * 0.5f, btnY + btnH)
        val nextR = RectF(cx + btnW * 0.6f, btnY, cx + btnW * 1.6f, btnY + btnH)

        drawPillButton(canvas, prevR, "⏮", Color.argb(40, 255, 255, 255), Color.WHITE, 14f)
        drawPillButton(
            canvas, playR, if (st.media.isPlaying) "⏸ Pause" else "▶ Play",
            Color.argb(210, 56, 189, 248), Color.rgb(8, 10, 14), 11f
        )
        drawPillButton(canvas, nextR, "⏭", Color.argb(40, 255, 255, 255), Color.WHITE, 14f)
        hitTargets.add(prevR to HitAction.MediaControl(MediaControlAction.PREVIOUS))
        hitTargets.add(playR to HitAction.MediaControl(MediaControlAction.PLAY_PAUSE))
        hitTargets.add(nextR to HitAction.MediaControl(MediaControlAction.NEXT))
    }

    private fun drawTimerTab(canvas: Canvas, area: RectF, st: IslandRenderState) {
        textPaint.textAlign = Paint.Align.CENTER
        textPaint.color = Color.WHITE
        textPaint.textSize = dp(32f)
        textPaint.isFakeBoldText = true
        canvas.drawText(st.timer.formatted(), area.centerX(), area.top + dp(46f), textPaint)

        // Progress bar
        val bar = RectF(area.left + dp(16f), area.top + dp(60f), area.right - dp(16f), area.top + dp(66f))
        fillPaint.color = Color.argb(40, 255, 255, 255)
        canvas.drawRoundRect(bar, dp(3f), dp(3f), fillPaint)
        fillPaint.color = Color.rgb(16, 185, 129)
        canvas.drawRoundRect(RectF(bar.left, bar.top, bar.left + bar.width() * st.timer.progress(), bar.bottom), dp(3f), dp(3f), fillPaint)

        // Start / Reset
        val rY = area.top + dp(78f)
        val toggleR = RectF(area.left + dp(16f), rY, area.centerX() - dp(6f), rY + dp(28f))
        val resetR = RectF(area.centerX() + dp(6f), rY, area.right - dp(16f), rY + dp(28f))
        drawPillButton(
            canvas, toggleR, if (st.timer.running) "Pause" else "Start Focus",
            if (st.timer.running) Color.argb(200, 245, 158, 11) else Color.argb(200, 16, 185, 129),
            Color.rgb(8, 10, 14), 11f
        )
        drawPillButton(canvas, resetR, "Reset", Color.argb(42, 255, 255, 255), Color.WHITE, 11f)
        hitTargets.add(toggleR to HitAction.TimerToggle)
        hitTargets.add(resetR to HitAction.TimerReset)

        // Presets
        val pY = rY + dp(36f)
        val pw = (area.width() - dp(32f)) / PomodoroTimer.PRESETS_SECS.size
        PomodoroTimer.PRESETS_SECS.forEachIndexed { i, secs ->
            val pr = RectF(area.left + dp(16f) + i * pw + dp(3f), pY, area.left + dp(16f) + (i + 1) * pw - dp(3f), pY + dp(24f))
            val active = st.timer.totalSecs == secs
            drawPillButton(
                canvas, pr, "${secs / 60}m",
                if (active) Color.argb(190, 56, 189, 248) else Color.argb(32, 255, 255, 255),
                if (active) Color.rgb(8, 10, 14) else Color.WHITE, 10f
            )
            hitTargets.add(pr to HitAction.TimerPreset(secs))
        }
    }

    private fun drawClipTab(canvas: Canvas, area: RectF, st: IslandRenderState) {
        val clrR = RectF(area.right - dp(64f), area.top, area.right, area.top + dp(22f))
        drawPillButton(canvas, clrR, "Clear", Color.argb(55, 239, 68, 68), Color.rgb(252, 165, 165), 9.5f)
        hitTargets.add(clrR to HitAction.ClipClear)

        textPaint.textAlign = Paint.Align.LEFT
        textPaint.color = Color.rgb(148, 163, 184)
        textPaint.textSize = dp(10.5f)
        textPaint.isFakeBoldText = true
        canvas.drawText(if (st.copiedFlash) "✓ Disalin ke Clipboard!" else "Riwayat Clipboard (tap untuk salin)", area.left, area.top + dp(15f), textPaint)

        if (st.clipItems.isEmpty()) {
            textPaint.color = Color.rgb(100, 116, 139)
            textPaint.isFakeBoldText = false
            canvas.drawText("Belum ada teks yang disalin.", area.left, area.top + dp(52f), textPaint)
            return
        }
        var y = area.top + dp(30f)
        st.clipItems.take(4).forEachIndexed { idx, txt ->
            val row = RectF(area.left, y, area.right, y + dp(28f))
            fillPaint.color = Color.argb(42, 255, 255, 255)
            canvas.drawRoundRect(row, dp(8f), dp(8f), fillPaint)
            textPaint.color = Color.WHITE
            textPaint.textSize = dp(10.5f)
            textPaint.isFakeBoldText = false
            canvas.drawText(ellipsize(txt.replace('\n', ' '), row.width() - dp(16f)), row.left + dp(8f), row.centerY() + dp(4f), textPaint)
            hitTargets.add(row to HitAction.ClipCopy(idx))
            y += dp(33f)
        }
    }

    private fun drawSysTab(canvas: Canvas, area: RectF, st: IslandRenderState) {
        val s = st.sysStats
        textPaint.textAlign = Paint.Align.LEFT
        textPaint.color = Color.WHITE
        textPaint.textSize = dp(12f)
        textPaint.isFakeBoldText = true
        val battLabel = "Battery: ${s.batteryPercent}%" + if (s.charging) " ⚡" else ""
        val ramLabel = "RAM: ${s.ramUsedPercent}% (${"%.1f".format(s.ramUsedGb)}/${"%.1f".format(s.ramTotalGb)} GB)"
        canvas.drawText(battLabel, area.left + dp(6f), area.top + dp(22f), textPaint)
        canvas.drawText(ramLabel, area.left + dp(6f), area.top + dp(44f), textPaint)

        // Volume row
        drawAdjustRow(canvas, area.left, area.top + dp(62f), area.width(), "Volume: ${s.volumePercent}%", HitAction.AdjustVolume(-10), HitAction.AdjustVolume(10))
        // Brightness row
        drawAdjustRow(canvas, area.left, area.top + dp(104f), area.width(), "Brightness: ${s.brightnessPercent}%", HitAction.AdjustBrightness(-15), HitAction.AdjustBrightness(15))
    }

    private fun drawAdjustRow(canvas: Canvas, x: Float, y: Float, w: Float, label: String, dec: HitAction, inc: HitAction) {
        textPaint.textAlign = Paint.Align.LEFT
        textPaint.color = Color.rgb(203, 213, 225)
        textPaint.textSize = dp(11.5f)
        textPaint.isFakeBoldText = true
        canvas.drawText(label, x + dp(6f), y + dp(20f), textPaint)

        val plusR = RectF(x + w - dp(42f), y, x + w, y + dp(28f))
        val minusR = RectF(plusR.left - dp(48f), y, plusR.left - dp(6f), y + dp(28f))
        drawPillButton(canvas, minusR, "−", Color.argb(48, 255, 255, 255), Color.WHITE, 14f)
        drawPillButton(canvas, plusR, "+", Color.argb(180, 56, 189, 248), Color.rgb(8, 10, 14), 14f)
        hitTargets.add(minusR to dec)
        hitTargets.add(plusR to inc)
    }

    private fun drawNotifsTab(canvas: Canvas, area: RectF, st: IslandRenderState) {
        val clrR = RectF(area.right - dp(72f), area.top, area.right, area.top + dp(22f))
        drawPillButton(canvas, clrR, "Clear All", Color.argb(55, 239, 68, 68), Color.rgb(252, 165, 165), 9.5f)
        hitTargets.add(clrR to HitAction.NotifClearAll)

        textPaint.textAlign = Paint.Align.LEFT
        textPaint.color = Color.rgb(148, 163, 184)
        textPaint.textSize = dp(10.5f)
        textPaint.isFakeBoldText = true
        canvas.drawText("Notifikasi (${st.notifications.size})", area.left, area.top + dp(15f), textPaint)

        if (st.notifications.isEmpty()) {
            textPaint.color = Color.rgb(100, 116, 139)
            textPaint.isFakeBoldText = false
            canvas.drawText("Semua bersih — belum ada notifikasi masuk.", area.left, area.top + dp(52f), textPaint)
            return
        }

        var y = area.top + dp(28f)
        for (n in st.notifications.take(3)) {
            val row = RectF(area.left, y, area.right, y + dp(38f))
            fillPaint.color = Color.argb(42, 255, 255, 255)
            canvas.drawRoundRect(row, dp(10f), dp(10f), fillPaint)

            val xBtn = RectF(row.right - dp(26f), row.top + dp(8f), row.right - dp(6f), row.top + dp(28f))
            drawPillButton(canvas, xBtn, "×", Color.argb(40, 255, 255, 255), Color.WHITE, 11f)

            textPaint.color = Color.rgb(56, 189, 248)
            textPaint.textSize = dp(9.5f)
            textPaint.isFakeBoldText = true
            canvas.drawText(ellipsize("${n.appName} • ${n.title}", row.width() - dp(42f)), row.left + dp(8f), row.top + dp(15f), textPaint)
            textPaint.color = Color.rgb(226, 232, 240)
            textPaint.textSize = dp(10f)
            textPaint.isFakeBoldText = false
            canvas.drawText(ellipsize(n.body, row.width() - dp(42f)), row.left + dp(8f), row.top + dp(30f), textPaint)

            hitTargets.add(row to HitAction.NotifOpen(n.key))
            hitTargets.add(xBtn to HitAction.NotifDismiss(n.key))
            y += dp(42f)
        }
    }

    private fun drawPillButton(
        canvas: Canvas,
        rect: RectF,
        text: String,
        bgColor: Int,
        fgColor: Int,
        textSp: Float
    ) {
        fillPaint.color = bgColor
        val rad = rect.height() / 2f
        canvas.drawRoundRect(rect, rad, rad, fillPaint)
        textPaint.color = fgColor
        textPaint.textSize = dp(textSp)
        textPaint.isFakeBoldText = true
        textPaint.textAlign = Paint.Align.CENTER
        val fm = textPaint.fontMetrics
        val baseline = rect.centerY() - (fm.ascent + fm.descent) / 2f
        canvas.drawText(text, rect.centerX(), baseline, textPaint)
    }

    private fun ellipsize(text: String, maxWidthPx: Float): String {
        if (text.isEmpty() || textPaint.measureText(text) <= maxWidthPx) return text
        val ell = "…"
        var end = text.length
        while (end > 1 && textPaint.measureText(text, 0, end) + textPaint.measureText(ell) > maxWidthPx) {
            end--
        }
        return text.substring(0, end) + ell
    }

    private fun wrapLines(text: String, maxWidthPx: Float, maxLines: Int): List<String> {
        val words = text.trim().split(Regex("\\s+")).filter { it.isNotEmpty() }
        if (words.isEmpty()) return emptyList()
        val lines = mutableListOf<String>()
        var current = ""
        for (w in words) {
            val candidate = if (current.isEmpty()) w else "$current $w"
            if (textPaint.measureText(candidate) <= maxWidthPx) {
                current = candidate
            } else {
                if (current.isNotEmpty()) lines.add(current)
                current = w
                if (lines.size >= maxLines) break
            }
        }
        if (current.isNotEmpty() && lines.size < maxLines) {
            lines.add(current)
        }
        if (lines.size == maxLines) {
            lines[maxLines - 1] = ellipsize(lines[maxLines - 1], maxWidthPx)
        }
        return lines
    }
}
