package io.zekrima.assistant.render

import kotlin.math.abs

/**
 * Damped harmonic spring physics mirroring Windows & Arch Linux PillSprings,
 * scaled cleanly in Android logical dp for phone & tablet viewports.
 */
object PillGeometry {
    const val AUTO_HIDDEN_WIDTH = 112f
    const val AUTO_HIDDEN_HEIGHT = 5f
    const val AUTO_HIDDEN_RADIUS = 2.5f

    const val COLLAPSED_WIDTH = 156f
    const val COLLAPSED_BADGE_WIDTH = 196f
    const val COLLAPSED_HEIGHT = 34f
    const val COLLAPSED_RADIUS = 17f

    const val ASSISTANT_WIDTH = 210f
    const val ASSISTANT_HEIGHT = 38f
    const val ASSISTANT_RADIUS = 19f

    const val NOTIFICATION_WIDTH = 336f
    const val NOTIFICATION_HEIGHT = 82f
    const val NOTIFICATION_RADIUS = 24f

    const val CICADA_SUBTITLE_WIDTH = 348f
    const val CICADA_SUBTITLE_HEIGHT = 92f
    const val CICADA_SUBTITLE_RADIUS = 22f

    const val EXPANDED_WIDTH = 348f
    const val EXPANDED_HEIGHT = 212f
    const val EXPANDED_RADIUS = 26f

    const val EXPANDED_CICADA_WIDTH = 352f
    const val EXPANDED_CICADA_HEIGHT = 320f

    const val EXPANDED_CHAT_WIDTH = 352f
    const val EXPANDED_CHAT_HEIGHT = 310f

    const val PILL_TOP_DP = 6f

    private const val REST_DISTANCE = 0.05f
    private const val REST_VELOCITY = 0.05f

    class Spring(initial: Float, private val stiffness: Float = 280f, private val damping: Float = 26f) {
        var current: Float = initial
            private set
        var target: Float = initial
            private set
        var velocity: Float = 0f
            private set

        fun setTarget(value: Float): Boolean {
            if (target == value) return false
            target = value
            return true
        }

        fun update(dt: Float): Boolean {
            val force = (target - current) * stiffness
            val dampingForce = velocity * damping
            velocity += (force - dampingForce) * dt
            current += velocity * dt
            return if (abs(target - current) < REST_DISTANCE && abs(velocity) < REST_VELOCITY) {
                current = target
                velocity = 0f
                false
            } else {
                true
            }
        }

        fun isSettled(): Boolean = current == target && velocity == 0f
    }
}

enum class PillMode {
    AUTO_HIDDEN,
    COLLAPSED,
    COLLAPSED_WITH_BADGE,
    ASSISTANT,
    CICADA_SUBTITLE,
    NOTIFICATION,
    EXPANDED,
    EXPANDED_CICADA,
    EXPANDED_CHAT
}

class PillSprings {
    val width = PillGeometry.Spring(PillGeometry.COLLAPSED_WIDTH)
    val height = PillGeometry.Spring(PillGeometry.COLLAPSED_HEIGHT)
    val radius = PillGeometry.Spring(PillGeometry.COLLAPSED_RADIUS)

    fun setMode(mode: PillMode, maxWidthDp: Float = 360f): Boolean {
        val (rawW, h, r) = when (mode) {
            PillMode.AUTO_HIDDEN -> Triple(
                PillGeometry.AUTO_HIDDEN_WIDTH,
                PillGeometry.AUTO_HIDDEN_HEIGHT,
                PillGeometry.AUTO_HIDDEN_RADIUS
            )
            PillMode.COLLAPSED -> Triple(
                PillGeometry.COLLAPSED_WIDTH,
                PillGeometry.COLLAPSED_HEIGHT,
                PillGeometry.COLLAPSED_RADIUS
            )
            PillMode.COLLAPSED_WITH_BADGE -> Triple(
                PillGeometry.COLLAPSED_BADGE_WIDTH,
                PillGeometry.COLLAPSED_HEIGHT,
                PillGeometry.COLLAPSED_RADIUS
            )
            PillMode.ASSISTANT -> Triple(
                PillGeometry.ASSISTANT_WIDTH,
                PillGeometry.ASSISTANT_HEIGHT,
                PillGeometry.ASSISTANT_RADIUS
            )
            PillMode.CICADA_SUBTITLE -> Triple(
                PillGeometry.CICADA_SUBTITLE_WIDTH,
                PillGeometry.CICADA_SUBTITLE_HEIGHT,
                PillGeometry.CICADA_SUBTITLE_RADIUS
            )
            PillMode.NOTIFICATION -> Triple(
                PillGeometry.NOTIFICATION_WIDTH,
                PillGeometry.NOTIFICATION_HEIGHT,
                PillGeometry.NOTIFICATION_RADIUS
            )
            PillMode.EXPANDED -> Triple(
                PillGeometry.EXPANDED_WIDTH,
                PillGeometry.EXPANDED_HEIGHT,
                PillGeometry.EXPANDED_RADIUS
            )
            PillMode.EXPANDED_CICADA -> Triple(
                PillGeometry.EXPANDED_CICADA_WIDTH,
                PillGeometry.EXPANDED_CICADA_HEIGHT,
                PillGeometry.EXPANDED_RADIUS
            )
            PillMode.EXPANDED_CHAT -> Triple(
                PillGeometry.EXPANDED_CHAT_WIDTH,
                PillGeometry.EXPANDED_CHAT_HEIGHT,
                PillGeometry.EXPANDED_RADIUS
            )
        }
        val clampedW = rawW.coerceAtMost(maxWidthDp)
        val wChanged = width.setTarget(clampedW)
        val hChanged = height.setTarget(h)
        val rChanged = radius.setTarget(r)
        return wChanged || hChanged || rChanged
    }

    fun update(dt: Float): Boolean {
        val w = width.update(dt)
        val h = height.update(dt)
        val r = radius.update(dt)
        return w || h || r
    }

    fun isSettled(): Boolean = width.isSettled() && height.isSettled() && radius.isSettled()
}
