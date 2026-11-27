package io.zekrima.assistant.services

import android.annotation.SuppressLint
import android.content.ClipData
import android.content.ClipboardManager
import android.content.Context
import android.media.AudioAttributes
import android.media.AudioFormat
import android.media.AudioPlaybackCaptureConfiguration
import android.media.AudioRecord
import android.media.MediaRecorder
import android.media.projection.MediaProjection
import android.os.Environment
import android.util.Base64
import io.zekrima.assistant.ZekrimaApp
import io.zekrima.assistant.config.AssistantConfig
import io.zekrima.assistant.config.CICADA_LANGUAGES
import io.zekrima.assistant.config.LlmProvider
import io.zekrima.assistant.config.cicadaLangName
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.launch
import org.json.JSONArray
import org.json.JSONObject
import java.io.BufferedReader
import java.io.File
import java.net.HttpURLConnection
import java.net.URL
import java.net.URLEncoder
import java.text.SimpleDateFormat
import java.util.Date
import java.util.Locale
import java.util.concurrent.CopyOnWriteArrayList
import java.util.concurrent.atomic.AtomicBoolean
import java.util.concurrent.atomic.AtomicLong
import kotlin.math.abs
import kotlin.math.sqrt

enum class SpeakerChannel(val label: String) {
    ONLINE("Online"),
    YOU("You"),
    QUICK_REPLY("Reply")
}

data class TranscriptEntry(
    val id: Long,
    val timestamp: String,
    val speaker: SpeakerChannel,
    val sourceLang: String,
    val original: String,
    val translation: String
)

data class CicadaSnapshot(
    val active: Boolean = false,
    val captureModeLabel: String = "Idle",
    val micEnabled: Boolean = false,
    val quickReplyRecording: Boolean = false,
    val targetLang: String = "id",
    val replyLang: String = "en",
    val showOriginal: Boolean = true,
    val entries: List<TranscriptEntry> = emptyList(),
    val lastQuickReply: TranscriptEntry? = null,
    val arrivalSeq: Long = 0,
    val statusNote: String = "Siap menerjemahkan meeting live"
) {
    fun hasEntries(): Boolean = entries.isNotEmpty()
    fun latest(): TranscriptEntry? = entries.lastOrNull()
}

/**
 * Pure-Kotlin port of Cicada's SpeechSegmenter:
 * - 30 ms frames (480 samples @ 16 kHz)
 * - 100 Hz first-order high-pass filter (rumble rejection)
 * - Adaptive noise floor & steady-noise rejection
 * - 10 ms onset click rejection
 * - 0.55 s pause endpointing & 7.5 s quietest-frame forced cut
 */
class SpeechSegmenter(private val sampleRate: Int = 16_000) {
    private val frameSize = sampleRate * 30 / 1000 // 480
    private val pauseFramesLimit = 18 // ~540 ms
    private val maxFramesLimit = 250 // ~7.5 s
    private val minSpeechFrames = 6 // ~180 ms

    private var prevIn = 0f
    private var prevOut = 0f
    private var noiseFloor = 0.008f
    private var inSpeech = false
    private var silenceFrames = 0
    private var onsetFrames = 0
    private val speechBuffer = ArrayList<Short>(sampleRate * 4)
    private val preRoll = ArrayDeque<ShortArray>(4)

    fun reset() {
        prevIn = 0f
        prevOut = 0f
        inSpeech = false
        silenceFrames = 0
        onsetFrames = 0
        speechBuffer.clear()
        preRoll.clear()
    }

    fun feedFrame(frame: ShortArray): Pair<Float, ShortArray?> {
        if (frame.isEmpty()) return 0f to null
        // 100 Hz high-pass filter
        val alpha = 0.96f
        var sumSq = 0.0
        for (i in frame.indices) {
            val x = frame[i] / 32768f
            val y = alpha * (prevOut + x - prevIn)
            prevIn = x
            prevOut = y
            sumSq += (y * y).toDouble()
        }
        val rms = sqrt(sumSq / frame.size).toFloat()
        val level = (rms * 8f).coerceIn(0f, 1f)

        val threshold = (noiseFloor * 2.6f).coerceIn(0.014f, 0.08f)
        val isVoice = rms > threshold

        if (!inSpeech) {
            if (!isVoice) {
                noiseFloor = noiseFloor * 0.96f + rms * 0.04f
                onsetFrames = 0
                if (preRoll.size >= 3) preRoll.removeFirst()
                preRoll.addLast(frame.copyOf())
                return level to null
            } else {
                onsetFrames += 1
                if (preRoll.size >= 3) preRoll.removeFirst()
                preRoll.addLast(frame.copyOf())
                if (onsetFrames >= 2) {
                    inSpeech = true
                    silenceFrames = 0
                    speechBuffer.clear()
                    for (pr in preRoll) {
                        for (s in pr) speechBuffer.add(s)
                    }
                    preRoll.clear()
                }
                return level to null
            }
        } else {
            for (s in frame) speechBuffer.add(s)
            if (isVoice) {
                silenceFrames = 0
            } else {
                silenceFrames += 1
                noiseFloor = noiseFloor * 0.985f + rms * 0.015f
            }

            val totalFrames = speechBuffer.size / frameSize
            if (silenceFrames >= pauseFramesLimit || totalFrames >= maxFramesLimit) {
                val out = if (totalFrames >= minSpeechFrames) {
                    ShortArray(speechBuffer.size) { speechBuffer[it] }
                } else null
                inSpeech = false
                silenceFrames = 0
                onsetFrames = 0
                speechBuffer.clear()
                return level to out
            }
            return level to null
        }
    }
}

/**
 * Dual-channel EchoFilter (timing gate + word-bigram overlap) to drop speaker bleed on local mic.
 */
class EchoFilter {
    private val recentRemote = ArrayDeque<Pair<Long, String>>(12)

    fun recordRemote(text: String, nowMs: Long = System.currentTimeMillis()) {
        val norm = normalize(text)
        if (norm.isEmpty()) return
        recentRemote.addLast(nowMs to norm)
        while (recentRemote.size > 12) recentRemote.removeFirst()
    }

    fun isLikelyEcho(localText: String, nowMs: Long = System.currentTimeMillis()): Boolean {
        val norm = normalize(localText)
        if (norm.isEmpty()) return false
        val localBigrams = bigrams(norm)
        if (localBigrams.isEmpty()) return false
        for ((ts, remote) in recentRemote) {
            if (nowMs - ts > 8_000L) continue
            if (remote.contains(norm) || norm.contains(remote)) return true
            val remBigrams = bigrams(remote)
            if (remBigrams.isEmpty()) continue
            val overlap = localBigrams.count { it in remBigrams }
            if (overlap.toFloat() / localBigrams.size.toFloat() >= 0.45f) return true
        }
        return false
    }

    private fun normalize(s: String): String =
        s.lowercase().replace(Regex("""[^\p{L}\p{N}\s]"""), " ").replace(Regex("""\s+"""), " ").trim()

    private fun bigrams(s: String): Set<String> {
        val w = s.split(" ").filter { it.isNotEmpty() }
        if (w.size == 1) return setOf(w[0])
        return (0 until w.size - 1).map { "${w[it]} ${w[it + 1]}" }.toSet()
    }
}

object CicadaTextParser {
    private val FILLERS = setOf(
        "thank you", "thanks for watching", "subscribe", "bye", "you", ".", "...", "mbc", "amara.org"
    )

    fun isHallucinationOrNoise(text: String): Boolean {
        val t = text.trim()
        if (t.length < 2) return true
        val lower = t.lowercase().trim('.', ',', '!', '?')
        if (lower in FILLERS) return true
        if (t.startsWith("[") && t.endsWith("]")) return true
        if (t.startsWith("♪") || t.contains("♫")) return true
        return false
    }

    fun parseThreeLineHeard(raw: String): Triple<String, String, String>? {
        var lang = "AUTO"
        var original = ""
        var translation = ""
        for (line in raw.lines()) {
            val trimmed = line.trim()
            when {
                trimmed.startsWith("LANG:", true) -> lang = trimmed.substringAfter(":").trim().uppercase().take(4)
                trimmed.startsWith("ORIGINAL:", true) -> original = trimmed.substringAfter(":").trim()
                trimmed.startsWith("TRANSLATION:", true) -> translation = trimmed.substringAfter(":").trim()
            }
        }
        if (original.isEmpty() && translation.isEmpty()) return null
        if (translation.isEmpty()) translation = original
        if (original.isEmpty()) original = translation
        return Triple(lang.ifEmpty { "AUTO" }, original, translation)
    }

    fun parseGoogleGtx(json: String): Pair<String, String>? {
        return try {
            val root = JSONArray(json)
            val segs = root.optJSONArray(0) ?: return null
            val sb = StringBuilder()
            for (i in 0 until segs.length()) {
                val part = segs.optJSONArray(i)?.optString(0)
                if (!part.isNullOrEmpty()) sb.append(part)
            }
            val detected = root.optString(2, "auto").uppercase().take(4)
            val translated = sb.toString().trim()
            if (translated.isEmpty()) null else detected to translated
        } catch (_: Throwable) {
            null
        }
    }
}

/**
 * Cicada Mode Live Meeting Translator for Android 10+:
 * - Primary: AudioPlaybackCaptureConfiguration (Direct System Audio Loopback via MediaProjection)
 * - Automatic Fallback: Speakerphone / Mic Capture if app blocks playback capture or no projection yet
 * - Dual-Channel Mic ("You") + EchoFilter + Bidirectional Quick-Reply + Markdown Export
 */
class CicadaService(
    private val context: Context,
    private val assistant: AssistantService
) {
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.IO)
    private val running = AtomicBoolean(false)
    private val micOn = AtomicBoolean(false)
    private val entryIdGen = AtomicLong(1L)
    private val entries = CopyOnWriteArrayList<TranscriptEntry>()
    private val echoFilter = EchoFilter()
    private val quickRecorder = VoiceRecorder()

    @Volatile
    var liveAudioLevel: Float = 0f
        private set

    @Volatile
    private var snapshot = loadInitialSnapshot()

    private var loopbackThread: Thread? = null
    private var micThread: Thread? = null

    private fun loadInitialSnapshot(): CicadaSnapshot {
        val cfg = AssistantConfig.load(context)
        return CicadaSnapshot(
            targetLang = cfg.cicadaTargetLang,
            replyLang = cfg.cicadaReplyLang,
            showOriginal = cfg.cicadaShowOriginal
        )
    }

    fun snapshot(): CicadaSnapshot = snapshot

    fun refreshFromConfig() {
        val cfg = AssistantConfig.load(context)
        snapshot = snapshot.copy(
            targetLang = cfg.cicadaTargetLang,
            replyLang = cfg.cicadaReplyLang,
            showOriginal = cfg.cicadaShowOriginal
        )
        ZekrimaApp.notifyStateChanged()
    }

    fun start(projection: MediaProjection?) {
        if (!running.compareAndSet(false, true)) return
        val cfg = AssistantConfig.load(context)
        val useLoopback = projection != null
        snapshot = snapshot.copy(
            active = true,
            captureModeLabel = if (useLoopback) "System Loopback" else "Speakerphone Mic",
            targetLang = cfg.cicadaTargetLang,
            replyLang = cfg.cicadaReplyLang,
            showOriginal = cfg.cicadaShowOriginal,
            statusNote = if (useLoopback) {
                "Mendengarkan audio sistem meeting secara langsung..."
            } else {
                "Mode Speakerphone (Mic) — aktifkan Loopback di Dashboard untuk audio sistem"
            }
        )
        ZekrimaApp.notifyStateChanged()

        loopbackThread = Thread({
            runCaptureLoop(
                projection = projection,
                channel = SpeakerChannel.ONLINE,
                fallbackToMic = true
            )
        }, "zekrima-cicada-remote").apply { start() }

        if (micOn.get() && useLoopback) {
            startDualMicThread()
        }
    }

    fun stop() {
        if (!running.compareAndSet(true, false)) return
        liveAudioLevel = 0f
        if (entries.isNotEmpty()) {
            saveMarkdownFile()
        }
        snapshot = snapshot.copy(
            active = false,
            captureModeLabel = "Idle",
            statusNote = "Cicada dihentikan"
        )
        ZekrimaApp.notifyStateChanged()
    }

    fun toggleMic() {
        val next = !micOn.get()
        micOn.set(next)
        snapshot = snapshot.copy(micEnabled = next)
        if (next && running.get() && ZekrimaApp.activeMediaProjection != null) {
            startDualMicThread()
        }
        ZekrimaApp.notifyStateChanged()
    }

    fun cycleTargetLanguage() {
        val cur = snapshot.targetLang
        val idx = CICADA_LANGUAGES.indexOfFirst { it.code.equals(cur, true) }.coerceAtLeast(0)
        val next = CICADA_LANGUAGES[(idx + 1) % CICADA_LANGUAGES.size].code
        val cfg = AssistantConfig.load(context)
        AssistantConfig.save(context, cfg.copy(cicadaTargetLang = next))
        snapshot = snapshot.copy(targetLang = next)
        ZekrimaApp.notifyStateChanged()
    }

    fun toggleShowOriginal() {
        val next = !snapshot.showOriginal
        val cfg = AssistantConfig.load(context)
        AssistantConfig.save(context, cfg.copy(cicadaShowOriginal = next))
        snapshot = snapshot.copy(showOriginal = next)
        ZekrimaApp.notifyStateChanged()
    }

    fun toggleQuickReply() {
        if (quickRecorder.isRecording()) {
            quickRecorder.stop()
            return
        }
        snapshot = snapshot.copy(
            quickReplyRecording = true,
            statusNote = "Bicara balasan Anda -> diterjemahkan ke ${cicadaLangName(snapshot.replyLang)}..."
        )
        ZekrimaApp.notifyStateChanged()

        quickRecorder.startHandsFree(maxDurationMs = 12_000L, silenceEndMs = 1_100L) { wav ->
            snapshot = snapshot.copy(quickReplyRecording = false)
            ZekrimaApp.notifyStateChanged()
            if (wav != null) {
                scope.launch {
                    processSegment(wav, SpeakerChannel.QUICK_REPLY, snapshot.replyLang)
                }
            }
        }
    }

    fun clearTranscript() {
        entries.clear()
        snapshot = snapshot.copy(entries = emptyList(), lastQuickReply = null)
        ZekrimaApp.notifyStateChanged()
    }

    fun formatMarkdown(): String {
        if (entries.isEmpty()) return ""
        val sb = StringBuilder()
        sb.append("# Zekrima Cicada Meeting Transcript\n\n")
        for (e in entries) {
            sb.append("- **[").append(e.timestamp).append("] ").append(e.speaker.label).append("**")
            if (e.sourceLang.isNotBlank()) sb.append(" (`").append(e.sourceLang).append("`)")
            sb.append(": ").append(e.translation).append("\n")
            if (e.original.isNotBlank() && e.original != e.translation) {
                sb.append("  > _").append(e.original).append("_\n")
            }
        }
        return sb.toString()
    }

    fun saveMarkdownFile(): File? {
        val md = formatMarkdown()
        if (md.isEmpty()) return null
        return try {
            val dir = File(
                context.getExternalFilesDir(Environment.DIRECTORY_DOCUMENTS) ?: context.filesDir,
                "meetings"
            )
            dir.mkdirs()
            val stamp = SimpleDateFormat("yyyy-MM-dd_HH-mm", Locale.US).format(Date())
            val file = File(dir, "meeting_$stamp.md")
            file.writeText(md)
            file
        } catch (_: Throwable) {
            null
        }
    }

    fun buildQaContext(): String? {
        if (entries.isEmpty()) return null
        return entries.takeLast(24).joinToString("\n") { e ->
            "[${e.timestamp}] ${e.speaker.label}: ${e.translation}" +
                if (e.original != e.translation) " (orig: ${e.original})" else ""
        }
    }

    fun buildSummaryPrompt(): String? {
        val ctx = buildQaContext() ?: return null
        return "Buatkan ringkasan rapat yang terstruktur dalam format Markdown (Poin Utama, Keputusan, dan Action Items) berdasarkan transkrip berikut:\n\n$ctx"
    }

    private fun startDualMicThread() {
        if (micThread?.isAlive == true) return
        micThread = Thread({
            runCaptureLoop(projection = null, channel = SpeakerChannel.YOU, fallbackToMic = false)
        }, "zekrima-cicada-mic").apply { start() }
    }

    @SuppressLint("MissingPermission")
    private fun runCaptureLoop(
        projection: MediaProjection?,
        channel: SpeakerChannel,
        fallbackToMic: Boolean
    ) {
        val minBuf = AudioRecord.getMinBufferSize(
            WavEncoder.SAMPLE_RATE,
            AudioFormat.CHANNEL_IN_MONO,
            AudioFormat.ENCODING_PCM_16BIT
        ).coerceAtLeast(3200)

        var recorder: AudioRecord? = null
        var usingLoopback = false

        if (projection != null) {
            try {
                val pbConfig = AudioPlaybackCaptureConfiguration.Builder(projection)
                    .addMatchingUsage(AudioAttributes.USAGE_MEDIA)
                    .addMatchingUsage(AudioAttributes.USAGE_VOICE_COMMUNICATION)
                    .addMatchingUsage(AudioAttributes.USAGE_GAME)
                    .addMatchingUsage(AudioAttributes.USAGE_UNKNOWN)
                    .build()
                val fmt = AudioFormat.Builder()
                    .setEncoding(AudioFormat.ENCODING_PCM_16BIT)
                    .setSampleRate(WavEncoder.SAMPLE_RATE)
                    .setChannelMask(AudioFormat.CHANNEL_IN_MONO)
                    .build()
                recorder = AudioRecord.Builder()
                    .setAudioFormat(fmt)
                    .setBufferSizeInBytes(minBuf * 2)
                    .setAudioPlaybackCaptureConfig(pbConfig)
                    .build()
                if (recorder.state == AudioRecord.STATE_INITIALIZED) {
                    usingLoopback = true
                } else {
                    recorder.release()
                    recorder = null
                }
            } catch (_: Throwable) {
                recorder = null
            }
        }

        if (recorder == null && (fallbackToMic || channel == SpeakerChannel.YOU)) {
            try {
                recorder = AudioRecord(
                    MediaRecorder.AudioSource.VOICE_RECOGNITION,
                    WavEncoder.SAMPLE_RATE,
                    AudioFormat.CHANNEL_IN_MONO,
                    AudioFormat.ENCODING_PCM_16BIT,
                    minBuf * 2
                )
            } catch (_: Throwable) {
                recorder = null
            }
        }

        if (recorder == null || recorder.state != AudioRecord.STATE_INITIALIZED) {
            if (channel == SpeakerChannel.ONLINE) {
                snapshot = snapshot.copy(
                    active = false,
                    statusNote = "Izin mikrofon/loopback belum aktif"
                )
                running.set(false)
                ZekrimaApp.notifyStateChanged()
            }
            return
        }

        try {
            recorder.startRecording()
            val segmenter = SpeechSegmenter(WavEncoder.SAMPLE_RATE)
            val frame = ShortArray(480)
            var silentLoopbackFrames = 0

            while (running.get() && (channel == SpeakerChannel.ONLINE || micOn.get())) {
                val read = recorder.read(frame, 0, frame.size)
                if (read <= 0) continue
                val (level, segment) = segmenter.feedFrame(if (read == frame.size) frame else frame.copyOf(read))
                if (channel == SpeakerChannel.ONLINE) {
                    liveAudioLevel = level
                    if (usingLoopback && level < 0.002f) {
                        silentLoopbackFrames++
                        if (silentLoopbackFrames == 300) { // ~9s pure silence hint
                            snapshot = snapshot.copy(
                                statusNote = "Menunggu suara meeting (jika aplikasi memblokir DRM audio, aktifkan Mic)"
                            )
                            ZekrimaApp.notifyStateChanged()
                        }
                    } else if (level >= 0.01f) {
                        silentLoopbackFrames = 0
                    }
                }
                if (segment != null) {
                    val wav = WavEncoder.encodePcm16Mono(segment)
                    val tgt = snapshot.targetLang
                    scope.launch {
                        processSegment(wav, channel, tgt)
                    }
                }
            }
        } catch (_: Throwable) {
        } finally {
            try {
                recorder.stop()
            } catch (_: Throwable) {
            }
            try {
                recorder.release()
            } catch (_: Throwable) {
            }
        }
    }

    private fun processSegment(wavBytes: ByteArray, channel: SpeakerChannel, targetLangCode: String) {
        val cfg = AssistantConfig.load(context)
        val targetName = cicadaLangName(targetLangCode)
        val recentContext = entries.takeLast(4).joinToString(" | ") { it.translation }
        val glossary = cfg.cicadaGlossary.trim()

        val heard: Triple<String, String, String>? = when {
            cfg.geminiApiKey.isNotBlank() -> translateViaGeminiAudio(cfg, wavBytes, targetName, recentContext, glossary)
            else -> {
                val stt = assistant.transcribeWhisper(cfg, wavBytes)
                if (stt.isNullOrBlank()) null
                else {
                    val translated = translateTextWithFallback(cfg, stt, targetLangCode, targetName, recentContext, glossary)
                    Triple("AUTO", stt, translated ?: stt)
                }
            }
        }

        if (heard == null) return
        val (srcLang, original, translation) = heard
        if (CicadaTextParser.isHallucinationOrNoise(original) && CicadaTextParser.isHallucinationOrNoise(translation)) {
            return
        }

        if (channel == SpeakerChannel.ONLINE) {
            echoFilter.recordRemote(original)
            echoFilter.recordRemote(translation)
        } else if (channel == SpeakerChannel.YOU && echoFilter.isLikelyEcho(original)) {
            return
        }

        val ts = SimpleDateFormat("HH:mm:ss", Locale.US).format(Date())
        val entry = TranscriptEntry(
            id = entryIdGen.getAndIncrement(),
            timestamp = ts,
            speaker = channel,
            sourceLang = srcLang,
            original = original,
            translation = translation
        )
        entries.add(entry)
        while (entries.size > 120) entries.removeAt(0)

        if (channel == SpeakerChannel.QUICK_REPLY) {
            try {
                val cm = context.getSystemService(Context.CLIPBOARD_SERVICE) as? ClipboardManager
                cm?.setPrimaryClip(ClipData.newPlainText("Zekrima Quick-Reply", translation))
            } catch (_: Throwable) {
            }
        }

        snapshot = snapshot.copy(
            entries = entries.toList(),
            lastQuickReply = if (channel == SpeakerChannel.QUICK_REPLY) entry else snapshot.lastQuickReply,
            arrivalSeq = snapshot.arrivalSeq + 1,
            statusNote = "Live • ${entries.size} baris"
        )
        ZekrimaApp.notifyStateChanged()
    }

    private fun translateViaGeminiAudio(
        cfg: AssistantConfig,
        wavBytes: ByteArray,
        targetName: String,
        contextHint: String,
        glossary: String
    ): Triple<String, String, String>? {
        return try {
            val model = cfg.modelFor(LlmProvider.GEMINI)
            val url = "https://generativelanguage.googleapis.com/v1beta/models/$model:generateContent?key=${cfg.geminiApiKey.trim()}"
            val prompt = buildString {
                append("Transcribe the spoken speech and translate it naturally into $targetName.\n")
                if (contextHint.isNotBlank()) append("Recent context: $contextHint\n")
                if (glossary.isNotBlank()) append("Preserve these custom glossary terms exactly: $glossary\n")
                append("If audio is silence/noise/music only, return empty.\n")
                append("Respond STRICTLY in 3 lines:\nLANG: <2-letter code>\nORIGINAL: <transcript>\nTRANSLATION: <translation in $targetName>")
            }
            val parts = JSONArray()
                .put(JSONObject().put("text", prompt))
                .put(
                    JSONObject().put(
                        "inlineData",
                        JSONObject()
                            .put("mimeType", "audio/wav")
                            .put("data", Base64.encodeToString(wavBytes, Base64.NO_WRAP))
                    )
                )
            val payload = JSONObject().put(
                "contents",
                JSONArray().put(JSONObject().put("role", "user").put("parts", parts))
            )
            val body = assistant.httpPostJson(url, payload.toString(), null, false)
            val text = JSONObject(body)
                .optJSONArray("candidates")?.optJSONObject(0)
                ?.optJSONObject("content")?.optJSONArray("parts")
                ?.optJSONObject(0)?.optString("text").orEmpty()
            CicadaTextParser.parseThreeLineHeard(text)
        } catch (_: Throwable) {
            null
        }
    }

    private fun translateTextWithFallback(
        cfg: AssistantConfig,
        text: String,
        targetCode: String,
        targetName: String,
        contextHint: String,
        glossary: String
    ): String? {
        // Fast Google Translate GTX fallback (always works with zero extra quota!)
        return try {
            val enc = URLEncoder.encode(text, "UTF-8")
            val url = "https://translate.googleapis.com/translate_a/single?client=gtx&sl=auto&tl=$targetCode&dt=t&q=$enc"
            val conn = (URL(url).openConnection() as HttpURLConnection).apply {
                connectTimeout = 6_000
                readTimeout = 8_000
                setRequestProperty("User-Agent", "Mozilla/5.0")
            }
            val body = conn.inputStream.bufferedReader().use(BufferedReader::readText)
            conn.disconnect()
            CicadaTextParser.parseGoogleGtx(body)?.second
        } catch (_: Throwable) {
            null
        }
    }
}
