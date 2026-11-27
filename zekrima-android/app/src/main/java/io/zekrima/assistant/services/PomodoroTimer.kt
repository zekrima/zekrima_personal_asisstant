package io.zekrima.assistant.services

class PomodoroTimer {
    companion object {
        val PRESETS_SECS = intArrayOf(5 * 60, 15 * 60, 25 * 60, 45 * 60)
    }

    var totalSecs: Int = 25 * 60
        private set
    var remainingSecs: Int = 25 * 60
        private set
    var running: Boolean = false
        private set

    fun toggleStartPause() {
        if (remainingSecs <= 0) remainingSecs = totalSecs
        running = !running
    }

    fun reset() {
        running = false
        remainingSecs = totalSecs
    }

    fun selectPreset(secs: Int) {
        running = false
        totalSecs = secs
        remainingSecs = secs
    }

    fun tick(): Boolean {
        if (!running) return false
        if (remainingSecs > 0) {
            remainingSecs -= 1
            if (remainingSecs == 0) {
                running = false
                return true
            }
        }
        return false
    }

    fun progress(): Float =
        if (totalSecs <= 0) 0f else (1f - remainingSecs.toFloat() / totalSecs.toFloat()).coerceIn(0f, 1f)

    fun formatted(): String {
        val m = remainingSecs / 60
        val s = remainingSecs % 60
        return "%02d:%02d".format(m, s)
    }
}
