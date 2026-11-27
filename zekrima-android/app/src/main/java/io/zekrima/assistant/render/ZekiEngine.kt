package io.zekrima.assistant.render

import android.graphics.Canvas
import android.graphics.Color
import android.graphics.Paint
import android.graphics.Path
import android.graphics.RectF
import kotlin.math.PI
import kotlin.math.abs
import kotlin.math.cos
import kotlin.math.sin

enum class ZekiState(val label: String, val accentColor: Int) {
    IDLE("Idle", Color.rgb(56, 189, 248)),
    SLEEPY("Sleepy", Color.rgb(129, 140, 248)),
    LISTENING("Listening", Color.rgb(16, 185, 129)),
    THINKING("Thinking", Color.rgb(192, 132, 252)),
    SPEAKING("Speaking", Color.rgb(34, 211, 238)),
    HAPPY("Happy", Color.rgb(250, 204, 21)),
    ERROR("Error", Color.rgb(248, 113, 113));

    fun isAssistantActive(): Boolean =
        this == LISTENING || this == THINKING || this == SPEAKING || this == ERROR
}

enum class ZekiCicadaAccessory {
    NONE,
    HEADSET,
    TAKING_NOTES
}

data class ZekiSpark(
    var x: Float,
    var y: Float,
    var vx: Float,
    var vy: Float,
    var life: Float,
    val color: Int
)

/**
 * Pure procedural mascot "Zeki" — exact feature parity with Windows Direct2D & Linux Cairo:
 * 7 states, 15-minute auto-sleep, poke sparks, music dance, audio-reactive Cicada headset,
 * gentle nod on new translated lines, and TakingNotes animated notebook & pen.
 */
class ZekiEngine {
    var state: ZekiState = ZekiState.IDLE
        private set
    var cicadaAccessory: ZekiCicadaAccessory = ZekiCicadaAccessory.NONE
        private set
    var cicadaAudioLevel: Float = 0f
        private set
    var liveVoiceLevel: Float? = null
    var dancing: Boolean = false

    private var elapsed: Float = 0f
    private var blinkTimer: Float = 2.6f
    private var blinkProgress: Float = 0f
    private var glanceOffset: Float = 0f
    private var glanceTarget: Float = 0f
    private var glanceTimer: Float = 3.2f
    private var idleSeconds: Int = 0
    private var happyRevertSecs: Float = 0f
    private var nodRemaining: Float = 0f
    private var pokeBounce: Float = 0f
    private val sparks = mutableListOf<ZekiSpark>()

    private val fillPaint = Paint(Paint.ANTI_ALIAS_FLAG).apply { style = Paint.Style.FILL }
    private val strokePaint = Paint(Paint.ANTI_ALIAS_FLAG).apply {
        style = Paint.Style.STROKE
        strokeCap = Paint.Cap.ROUND
        strokeJoin = Paint.Join.ROUND
    }
    private val textPaint = Paint(Paint.ANTI_ALIAS_FLAG).apply {
        textAlign = Paint.Align.CENTER
        isFakeBoldText = true
    }

    fun recordInteraction() {
        idleSeconds = 0
        if (state == ZekiState.SLEEPY) {
            state = ZekiState.IDLE
        }
    }

    fun setStateManual(next: ZekiState) {
        recordInteraction()
        state = next
        happyRevertSecs = 0f
    }

    fun setStateFromAssistant(next: ZekiState) {
        recordInteraction()
        state = next
        happyRevertSecs = if (next == ZekiState.HAPPY) 2.4f else 0f
    }

    fun triggerActionHappy() {
        recordInteraction()
        if (!state.isAssistantActive()) {
            state = ZekiState.HAPPY
            happyRevertSecs = 2.2f
        }
        pokeBounce = 1f
    }

    fun triggerNod() {
        nodRemaining = 0.55f
    }

    fun setCicadaMode(accessory: ZekiCicadaAccessory, audioLevel: Float) {
        cicadaAccessory = accessory
        cicadaAudioLevel = audioLevel.coerceIn(0f, 1f)
    }

    fun poke() {
        recordInteraction()
        state = ZekiState.HAPPY
        happyRevertSecs = 2.6f
        pokeBounce = 1.2f
        sparks.clear()
        val palette = intArrayOf(
            Color.rgb(56, 189, 248),
            Color.rgb(250, 204, 21),
            Color.rgb(16, 185, 129),
            Color.rgb(192, 132, 252)
        )
        for (i in 0 until 8) {
            val angle = (i * (2.0 * PI / 8.0)).toFloat()
            val speed = 1.4f + (i % 3) * 0.45f
            sparks.add(
                ZekiSpark(
                    x = 0f,
                    y = 0f,
                    vx = cos(angle) * speed,
                    vy = sin(angle) * speed,
                    life = 1f,
                    color = palette[i % palette.size]
                )
            )
        }
    }

    fun isHighEnergy(): Boolean =
        state.isAssistantActive() || state == ZekiState.HAPPY || dancing ||
            cicadaAccessory != ZekiCicadaAccessory.NONE || nodRemaining > 0f || sparks.isNotEmpty()

    fun onOneSecondTick(): Boolean {
        if (state == ZekiState.IDLE && !dancing && cicadaAccessory == ZekiCicadaAccessory.NONE) {
            idleSeconds += 1
            if (idleSeconds >= 900) {
                state = ZekiState.SLEEPY
                return true
            }
        }
        return false
    }

    fun update(dt: Float) {
        elapsed += dt
        if (happyRevertSecs > 0f) {
            happyRevertSecs -= dt
            if (happyRevertSecs <= 0f && state == ZekiState.HAPPY) {
                state = ZekiState.IDLE
            }
        }
        if (nodRemaining > 0f) {
            nodRemaining = (nodRemaining - dt).coerceAtLeast(0f)
        }
        if (pokeBounce > 0f) {
            pokeBounce = (pokeBounce - dt * 2.8f).coerceAtLeast(0f)
        }

        // Natural blinking
        blinkTimer -= dt
        if (blinkTimer <= 0f) {
            blinkProgress += dt * 7.5f
            if (blinkProgress >= 1f) {
                blinkProgress = 0f
                blinkTimer = 2.4f + (abs(sin(elapsed * 1.7f)) * 2.4f)
            }
        }

        // Occasional subtle glance in Idle
        glanceTimer -= dt
        if (glanceTimer <= 0f) {
            glanceTimer = 2.8f + (abs(cos(elapsed)) * 2.5f)
            glanceTarget = when {
                state != ZekiState.IDLE -> 0f
                sin(elapsed * 3.1f) > 0.35f -> 0.14f
                sin(elapsed * 3.1f) < -0.35f -> -0.14f
                else -> 0f
            }
        }
        glanceOffset += (glanceTarget - glanceOffset) * (dt * 6f).coerceAtMost(1f)

        // Sparks physics
        val iter = sparks.iterator()
        while (iter.hasNext()) {
            val s = iter.next()
            s.x += s.vx * dt
            s.y += s.vy * dt
            s.life -= dt * 1.8f
            if (s.life <= 0f) iter.remove()
        }
    }

    fun draw(canvas: Canvas, cx: Float, cy: Float, radius: Float) {
        val accent = if (cicadaAccessory != ZekiCicadaAccessory.NONE && !state.isAssistantActive()) {
            Color.rgb(16, 185, 129)
        } else {
            state.accentColor
        }

        val floatY = when {
            state == ZekiState.SLEEPY -> sin(elapsed * 1.4f) * radius * 0.05f
            dancing -> -abs(sin(elapsed * 5.8f)) * radius * 0.14f
            state == ZekiState.SPEAKING -> -abs(sin(elapsed * 7.2f)) * radius * 0.10f
            state == ZekiState.HAPPY -> -abs(sin(elapsed * 8.0f)) * radius * 0.15f
            else -> sin(elapsed * 2.4f) * radius * 0.06f
        }
        val nodY = if (nodRemaining > 0f) {
            sin(((0.55f - nodRemaining) / 0.55f) * PI.toFloat()) * radius * 0.18f
        } else 0f
        val bounceScale = 1f + pokeBounce * 0.12f * sin(pokeBounce * PI.toFloat())

        canvas.save()
        canvas.translate(cx, cy + floatY + nodY)
        if (state == ZekiState.ERROR) {
            canvas.rotate(sin(elapsed * 5f) * 8f)
        } else if (dancing) {
            canvas.rotate(sin(elapsed * 5.8f) * 6f)
        }
        canvas.scale(bounceScale, bounceScale)

        // Soft outer aura glow
        val pulse = 0.22f + 0.12f * sin(elapsed * if (state == ZekiState.THINKING) 5.5f else 2.5f)
        fillPaint.color = withAlpha(accent, (pulse * 255).toInt())
        canvas.drawCircle(0f, 0f, radius * 1.24f, fillPaint)

        // Cicada Headset headband (behind body)
        if (cicadaAccessory == ZekiCicadaAccessory.HEADSET) {
            strokePaint.color = Color.rgb(30, 41, 59)
            strokePaint.strokeWidth = (radius * 0.16f).coerceAtLeast(2f)
            val arc = RectF(-radius * 1.05f, -radius * 1.08f, radius * 1.05f, radius * 0.85f)
            canvas.drawArc(arc, 195f, 150f, false, strokePaint)
        }

        // Capsule blob body
        val bodyW = radius * 1.12f
        val bodyH = radius * 0.94f
        val bodyRect = RectF(-bodyW, -bodyH, bodyW, bodyH)
        fillPaint.color = Color.rgb(20, 23, 33)
        canvas.drawRoundRect(bodyRect, radius * 0.72f, radius * 0.72f, fillPaint)

        // Subtle rim border tinted with mood accent
        strokePaint.color = withAlpha(accent, 155)
        strokePaint.strokeWidth = (radius * 0.09f).coerceAtLeast(1.2f)
        canvas.drawRoundRect(bodyRect, radius * 0.72f, radius * 0.72f, strokePaint)

        // Cicada Headset glowing earcups
        if (cicadaAccessory == ZekiCicadaAccessory.HEADSET) {
            val pulseEar = 1f + cicadaAudioLevel * 0.25f
            val earW = radius * 0.22f * pulseEar
            val earH = radius * 0.44f
            fillPaint.color = Color.rgb(16, 185, 129)
            canvas.drawRoundRect(
                RectF(-bodyW - earW * 0.55f, -earH * 0.5f, -bodyW + earW * 0.45f, earH * 0.5f),
                earW * 0.5f, earW * 0.5f, fillPaint
            )
            canvas.drawRoundRect(
                RectF(bodyW - earW * 0.45f, -earH * 0.5f, bodyW + earW * 0.55f, earH * 0.5f),
                earW * 0.5f, earW * 0.5f, fillPaint
            )
        }

        // Expressive eyes
        drawEyes(canvas, radius, accent)

        // Dynamic mouth / audio waveform
        drawMouth(canvas, radius, accent)

        // Sleepy floating "zZz"
        if (state == ZekiState.SLEEPY) {
            val zOff = (elapsed * 0.6f) % 1f
            textPaint.color = withAlpha(accent, ((1f - zOff) * 220).toInt().coerceIn(30, 220))
            textPaint.textSize = (radius * 0.48f).coerceAtLeast(7f)
            canvas.drawText("z", radius * 0.78f, -radius * (0.45f + zOff * 0.45f), textPaint)
        }

        // Taking Notes accessory (Expanded Cicada Transcript mode)
        if (cicadaAccessory == ZekiCicadaAccessory.TAKING_NOTES) {
            val nbX = radius * 0.45f
            val nbY = radius * 0.25f
            val nbW = radius * 0.68f
            val nbH = radius * 0.62f
            fillPaint.color = Color.rgb(241, 245, 249)
            canvas.drawRoundRect(RectF(nbX, nbY, nbX + nbW, nbY + nbH), 3f, 3f, fillPaint)
            strokePaint.color = Color.rgb(16, 185, 129)
            strokePaint.strokeWidth = (radius * 0.08f).coerceAtLeast(1.1f)
            val penJitter = sin(elapsed * 12f) * radius * 0.08f
            canvas.drawLine(
                nbX + nbW * 0.18f,
                nbY + nbH * 0.35f,
                nbX + nbW * 0.78f + penJitter,
                nbY + nbH * 0.35f,
                strokePaint
            )
            canvas.drawLine(
                nbX + nbW * 0.18f,
                nbY + nbH * 0.65f,
                nbX + nbW * 0.62f,
                nbY + nbH * 0.65f,
                strokePaint
            )
        }

        // Poke sparks
        for (s in sparks) {
            fillPaint.color = withAlpha(s.color, (s.life.coerceIn(0f, 1f) * 255).toInt())
            canvas.drawCircle(s.x * radius, s.y * radius, radius * 0.13f * s.life, fillPaint)
        }

        canvas.restore()
    }

    private fun drawEyes(canvas: Canvas, radius: Float, accent: Int) {
        val eyeSpacing = radius * 0.40f
        val eyeY = -radius * 0.06f
        val eyeR = radius * 0.20f

        when (state) {
            ZekiState.SLEEPY -> {
                strokePaint.color = accent
                strokePaint.strokeWidth = (radius * 0.11f).coerceAtLeast(1.4f)
                val arcL = RectF(-eyeSpacing - eyeR, eyeY - eyeR * 0.4f, -eyeSpacing + eyeR, eyeY + eyeR * 0.7f)
                val arcR = RectF(eyeSpacing - eyeR, eyeY - eyeR * 0.4f, eyeSpacing + eyeR, eyeY + eyeR * 0.7f)
                canvas.drawArc(arcL, 15f, 150f, false, strokePaint)
                canvas.drawArc(arcR, 15f, 150f, false, strokePaint)
            }
            ZekiState.HAPPY -> {
                strokePaint.color = accent
                strokePaint.strokeWidth = (radius * 0.13f).coerceAtLeast(1.6f)
                val arcL = RectF(-eyeSpacing - eyeR, eyeY - eyeR * 0.5f, -eyeSpacing + eyeR, eyeY + eyeR * 0.8f)
                val arcR = RectF(eyeSpacing - eyeR, eyeY - eyeR * 0.5f, eyeSpacing + eyeR, eyeY + eyeR * 0.8f)
                canvas.drawArc(arcL, 195f, 150f, false, strokePaint)
                canvas.drawArc(arcR, 195f, 150f, false, strokePaint)
            }
            ZekiState.ERROR -> {
                strokePaint.color = accent
                strokePaint.strokeWidth = (radius * 0.12f).coerceAtLeast(1.5f)
                val s = eyeR * 0.85f
                canvas.drawLine(-eyeSpacing - s, eyeY - s, -eyeSpacing + s, eyeY + s, strokePaint)
                canvas.drawLine(-eyeSpacing + s, eyeY - s, -eyeSpacing - s, eyeY + s, strokePaint)
                canvas.drawLine(eyeSpacing - s, eyeY - s, eyeSpacing + s, eyeY + s, strokePaint)
                canvas.drawLine(eyeSpacing + s, eyeY - s, eyeSpacing - s, eyeY + s, strokePaint)
            }
            else -> {
                val blinkSquish = if (blinkProgress > 0f) {
                    1f - sin(blinkProgress * PI.toFloat()) * 0.88f
                } else 1f
                val lookX = when (state) {
                    ZekiState.THINKING -> cos(elapsed * 3.8f) * radius * 0.10f
                    else -> glanceOffset * radius
                }
                val lookY = when (state) {
                    ZekiState.THINKING -> -radius * 0.10f + sin(elapsed * 3.8f) * radius * 0.06f
                    else -> 0f
                }
                fillPaint.color = accent
                val h = (eyeR * 1.18f * blinkSquish).coerceAtLeast(1.2f)
                canvas.drawOval(
                    RectF(-eyeSpacing - eyeR + lookX, eyeY - h + lookY, -eyeSpacing + eyeR + lookX, eyeY + h + lookY),
                    fillPaint
                )
                canvas.drawOval(
                    RectF(eyeSpacing - eyeR + lookX, eyeY - h + lookY, eyeSpacing + eyeR + lookX, eyeY + h + lookY),
                    fillPaint
                )
                if (blinkSquish > 0.5f) {
                    fillPaint.color = Color.WHITE
                    val specR = eyeR * 0.34f
                    canvas.drawCircle(-eyeSpacing + lookX - eyeR * 0.25f, eyeY + lookY - h * 0.32f, specR, fillPaint)
                    canvas.drawCircle(eyeSpacing + lookX - eyeR * 0.25f, eyeY + lookY - h * 0.32f, specR, fillPaint)
                }
            }
        }
    }

    private fun drawMouth(canvas: Canvas, radius: Float, accent: Int) {
        val mouthY = radius * 0.38f
        when (state) {
            ZekiState.LISTENING -> {
                val level = liveVoiceLevel ?: 0.25f
                strokePaint.color = accent
                strokePaint.strokeWidth = (radius * 0.09f).coerceAtLeast(1.3f)
                for (i in -2..2) {
                    val x = i * radius * 0.17f
                    val amp = (0.25f + level * 0.85f) * (1f - abs(i) * 0.18f) *
                        (0.65f + 0.35f * sin(elapsed * 14f + i * 1.4f))
                    val halfH = radius * 0.22f * amp
                    canvas.drawLine(x, mouthY - halfH, x, mouthY + halfH, strokePaint)
                }
            }
            ZekiState.SPEAKING -> {
                val open = (0.35f + 0.65f * abs(sin(elapsed * 12f))) * radius * 0.18f
                fillPaint.color = accent
                canvas.drawOval(
                    RectF(-radius * 0.24f, mouthY - open, radius * 0.24f, mouthY + open),
                    fillPaint
                )
            }
            ZekiState.HAPPY -> {
                val p = Path()
                p.moveTo(-radius * 0.22f, mouthY - radius * 0.04f)
                p.quadTo(0f, mouthY + radius * 0.24f, radius * 0.22f, mouthY - radius * 0.04f)
                strokePaint.color = accent
                strokePaint.strokeWidth = (radius * 0.10f).coerceAtLeast(1.4f)
                canvas.drawPath(p, strokePaint)
            }
            else -> Unit
        }
    }

    private fun withAlpha(color: Int, alpha: Int): Int =
        Color.argb(alpha.coerceIn(0, 255), Color.red(color), Color.green(color), Color.blue(color))
}
