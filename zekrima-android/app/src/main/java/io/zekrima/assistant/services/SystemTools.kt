package io.zekrima.assistant.services

import android.app.ActivityManager
import android.content.Context
import android.content.Intent
import android.content.IntentFilter
import android.media.AudioManager
import android.os.BatteryManager
import android.provider.Settings

data class SystemStats(
    val batteryPercent: Int = 100,
    val charging: Boolean = false,
    val ramUsedPercent: Int = 0,
    val ramUsedGb: Float = 0f,
    val ramTotalGb: Float = 0f,
    val volumePercent: Int = 50,
    val brightnessPercent: Int = 50
)

sealed class ToolAction {
    data class SetVolume(val level: Int?, val mute: Boolean?) : ToolAction()
    data class SetBrightness(val level: Int) : ToolAction()
    data class OpenApp(val name: String) : ToolAction()
    data object GetNotifications : ToolAction()
}

data class ToolParseResult(
    val cleanReply: String,
    val actions: List<ToolAction>
)

object ToolProtocol {
    private val TAG_REGEX = Regex("""\[\[TOOL:([^\]\n]+)\]\]""")

    fun parseAndClean(raw: String): ToolParseResult {
        val actions = mutableListOf<ToolAction>()
        for (m in TAG_REGEX.findAll(raw)) {
            val body = m.groupValues[1].trim()
            val parts = body.split(":")
            val cmd = parts.firstOrNull()?.trim()?.lowercase() ?: continue
            val arg = parts.drop(1).joinToString(":").trim()
            when (cmd) {
                "set_volume", "volume" -> {
                    when {
                        arg.equals("mute", true) -> actions.add(ToolAction.SetVolume(null, true))
                        arg.equals("unmute", true) -> actions.add(ToolAction.SetVolume(null, false))
                        else -> arg.filter { it.isDigit() }.toIntOrNull()?.let {
                            actions.add(ToolAction.SetVolume(it.coerceIn(0, 100), null))
                        }
                    }
                }
                "set_brightness", "brightness" -> {
                    arg.filter { it.isDigit() }.toIntOrNull()?.let {
                        actions.add(ToolAction.SetBrightness(it.coerceIn(5, 100)))
                    }
                }
                "open_app", "open" -> {
                    if (arg.isNotEmpty()) actions.add(ToolAction.OpenApp(arg))
                }
                "get_notifications", "notifications" -> {
                    actions.add(ToolAction.GetNotifications)
                }
            }
        }
        val cleaned = TAG_REGEX.replace(raw, "").replace(Regex("""\n{3,}"""), "\n\n").trim()
        return ToolParseResult(cleaned, actions)
    }

    fun systemInstructionBlock(): String = """
You can control Android device functions by appending one or more tags at the end of your reply:
- [[TOOL:set_volume:30]] or [[TOOL:set_volume:mute]] or [[TOOL:set_volume:unmute]]
- [[TOOL:set_brightness:80]]
- [[TOOL:open_app:Spotify]]
Only include a [[TOOL:...]] tag when the user clearly asks to perform that action.
""".trim()
}

object SystemTools {

    fun sampleStats(context: Context): SystemStats {
        val bm = context.registerReceiver(null, IntentFilter(Intent.ACTION_BATTERY_CHANGED))
        val level = bm?.getIntExtra(BatteryManager.EXTRA_LEVEL, -1) ?: -1
        val scale = bm?.getIntExtra(BatteryManager.EXTRA_SCALE, 100) ?: 100
        val pct = if (level >= 0 && scale > 0) (level * 100 / scale).coerceIn(0, 100) else 100
        val status = bm?.getIntExtra(BatteryManager.EXTRA_STATUS, -1) ?: -1
        val charging = status == BatteryManager.BATTERY_STATUS_CHARGING ||
            status == BatteryManager.BATTERY_STATUS_FULL

        val am = context.getSystemService(Context.ACTIVITY_SERVICE) as? ActivityManager
        val mi = ActivityManager.MemoryInfo()
        var ramPct = 0
        var usedGb = 0f
        var totalGb = 0f
        if (am != null) {
            am.getMemoryInfo(mi)
            if (mi.totalMem > 0) {
                val used = mi.totalMem - mi.availMem
                ramPct = ((used * 100L) / mi.totalMem).toInt().coerceIn(0, 100)
                usedGb = used / (1024f * 1024f * 1024f)
                totalGb = mi.totalMem / (1024f * 1024f * 1024f)
            }
        }

        val audio = context.getSystemService(Context.AUDIO_SERVICE) as? AudioManager
        val volPct = if (audio != null) {
            val max = audio.getStreamMaxVolume(AudioManager.STREAM_MUSIC).coerceAtLeast(1)
            val cur = audio.getStreamVolume(AudioManager.STREAM_MUSIC)
            (cur * 100 / max).coerceIn(0, 100)
        } else 50

        val brPct = try {
            val raw = Settings.System.getInt(context.contentResolver, Settings.System.SCREEN_BRIGHTNESS, 128)
            (raw * 100 / 255).coerceIn(0, 100)
        } catch (_: Throwable) {
            50
        }

        return SystemStats(pct, charging, ramPct, usedGb, totalGb, volPct, brPct)
    }

    fun setVolume(context: Context, percent: Int?, mute: Boolean?): Boolean {
        val audio = context.getSystemService(Context.AUDIO_SERVICE) as? AudioManager ?: return false
        return try {
            when (mute) {
                true -> audio.adjustStreamVolume(AudioManager.STREAM_MUSIC, AudioManager.ADJUST_MUTE, 0)
                false -> audio.adjustStreamVolume(AudioManager.STREAM_MUSIC, AudioManager.ADJUST_UNMUTE, 0)
                null -> if (percent != null) {
                    val max = audio.getStreamMaxVolume(AudioManager.STREAM_MUSIC).coerceAtLeast(1)
                    val target = ((percent.coerceIn(0, 100) / 100f) * max).toInt()
                    audio.setStreamVolume(AudioManager.STREAM_MUSIC, target, 0)
                }
            }
            true
        } catch (_: Throwable) {
            false
        }
    }

    fun setBrightness(context: Context, percent: Int): Boolean {
        if (!Settings.System.canWrite(context)) return false
        return try {
            Settings.System.putInt(
                context.contentResolver,
                Settings.System.SCREEN_BRIGHTNESS_MODE,
                Settings.System.SCREEN_BRIGHTNESS_MODE_MANUAL
            )
            val value = ((percent.coerceIn(5, 100) / 100f) * 255f).toInt().coerceIn(12, 255)
            Settings.System.putInt(context.contentResolver, Settings.System.SCREEN_BRIGHTNESS, value)
            true
        } catch (_: Throwable) {
            false
        }
    }

    fun openApp(context: Context, query: String): Boolean {
        val pm = context.packageManager
        val q = query.trim().lowercase()
        if (q.isEmpty()) return false
        val mainIntent = Intent(Intent.ACTION_MAIN).addCategory(Intent.CATEGORY_LAUNCHER)
        val apps = pm.queryIntentActivities(mainIntent, 0)
        val match = apps.firstOrNull { ri ->
            val label = ri.loadLabel(pm)?.toString()?.lowercase() ?: ""
            val pkg = ri.activityInfo?.packageName?.lowercase() ?: ""
            label == q || label.contains(q) || pkg.contains(q)
        } ?: return false
        val launch = pm.getLaunchIntentForPackage(match.activityInfo.packageName) ?: return false
        launch.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
        return try {
            context.startActivity(launch)
            true
        } catch (_: Throwable) {
            false
        }
    }

    fun executeActions(context: Context, actions: List<ToolAction>): Boolean {
        var didAny = false
        for (a in actions) {
            when (a) {
                is ToolAction.SetVolume -> if (setVolume(context, a.level, a.mute)) didAny = true
                is ToolAction.SetBrightness -> if (setBrightness(context, a.level)) didAny = true
                is ToolAction.OpenApp -> if (openApp(context, a.name)) didAny = true
                ToolAction.GetNotifications -> didAny = true
            }
        }
        return didAny
    }
}
