package io.zekrima.assistant.services

import android.content.Context
import android.media.AudioAttributes
import android.media.AudioFormat
import android.media.AudioTrack
import android.speech.tts.TextToSpeech
import android.util.Base64
import io.zekrima.assistant.ZekrimaApp
import io.zekrima.assistant.config.AssistantConfig
import io.zekrima.assistant.config.LlmProvider
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.launch
import org.json.JSONArray
import org.json.JSONObject
import java.io.BufferedReader
import java.io.DataOutputStream
import java.io.InputStreamReader
import java.net.HttpURLConnection
import java.net.URL
import java.util.Locale

enum class AssistantPhase {
    IDLE,
    LISTENING,
    TRANSCRIBING,
    THINKING,
    SPEAKING,
    DONE,
    ERROR;

    fun isBusy(): Boolean =
        this == LISTENING || this == TRANSCRIBING || this == THINKING || this == SPEAKING
}

data class ChatMessage(
    val role: String, // "user" | "assistant"
    val content: String
)

data class AssistantSnapshot(
    val phase: AssistantPhase = AssistantPhase.IDLE,
    val messages: List<ChatMessage> = emptyList(),
    val reply: String = "",
    val error: String? = null,
    val screenEnabled: Boolean = true,
    val hasApiKey: Boolean = false
) {
    fun hasConversationView(): Boolean =
        messages.isNotEmpty() || reply.isNotEmpty() || error != null || phase.isBusy()
}

/**
 * Live `/models` discovery parser & fetcher supporting all 6 providers
 * (Gemini, OpenRouter, OpenCode Zen, NVIDIA NIM, Groq, OpenAI/Custom/Ollama).
 */
object ModelDiscovery {
    fun fetchAvailableModels(provider: LlmProvider, apiKey: String, customBaseUrl: String): Result<List<String>> {
        val key = apiKey.trim()
        val base = customBaseUrl.trim().trimEnd('/').ifEmpty { "https://api.openai.com/v1" }
        if (key.isEmpty() && provider != LlmProvider.OPENROUTER && provider != LlmProvider.OPENAI) {
            return Result.failure(IllegalArgumentException("Isi API key terlebih dahulu."))
        }

        val url = when (provider) {
            LlmProvider.GEMINI ->
                "https://generativelanguage.googleapis.com/v1beta/models?key=$key&pageSize=100"
            LlmProvider.OPENROUTER -> "https://openrouter.ai/api/v1/models"
            LlmProvider.OPENCODE -> "https://opencode.ai/zen/v1/models"
            LlmProvider.NVIDIA -> "https://integrate.api.nvidia.com/v1/models"
            LlmProvider.GROQ -> "https://api.groq.com/openai/v1/models"
            LlmProvider.OPENAI -> "$base/models"
        }

        return try {
            val conn = (URL(url).openConnection() as HttpURLConnection).apply {
                requestMethod = "GET"
                connectTimeout = 10_000
                readTimeout = 12_000
                setRequestProperty("Accept", "application/json")
                if (provider != LlmProvider.GEMINI && key.isNotEmpty()) {
                    setRequestProperty("Authorization", "Bearer $key")
                }
                if (provider == LlmProvider.OPENROUTER) {
                    setRequestProperty("HTTP-Referer", "https://github.com/zekrima/zekrima_personal_asisstant")
                    setRequestProperty("X-Title", "Zekrima Android")
                }
            }
            val code = conn.responseCode
            val stream = if (code in 200..299) conn.inputStream else conn.errorStream
            val body = stream?.bufferedReader()?.use(BufferedReader::readText).orEmpty()
            conn.disconnect()
            if (code !in 200..299) {
                return Result.failure(RuntimeException("HTTP $code dari ${provider.shortBadge}"))
            }
            val models = parseModelListJson(provider, body)
            if (models.isEmpty()) {
                Result.failure(RuntimeException("Tidak ada model chat terdeteksi"))
            } else {
                Result.success(models)
            }
        } catch (t: Throwable) {
            Result.failure(RuntimeException("Gagal terhubung ke ${provider.shortBadge}: ${t.message}"))
        }
    }

    fun parseModelListJson(provider: LlmProvider, json: String): List<String> {
        val root = JSONObject(json)
        val out = LinkedHashSet<String>()
        if (provider == LlmProvider.GEMINI) {
            val arr = root.optJSONArray("models") ?: JSONArray()
            for (i in 0 until arr.length()) {
                val obj = arr.optJSONObject(i) ?: continue
                val methods = obj.optJSONArray("supportedGenerationMethods")
                var supportsChat = methods == null
                if (methods != null) {
                    for (j in 0 until methods.length()) {
                        if (methods.optString(j) == "generateContent") {
                            supportsChat = true
                            break
                        }
                    }
                }
                if (!supportsChat) continue
                val rawName = obj.optString("name").removePrefix("models/").trim()
                if (rawName.isNotEmpty() && !rawName.contains("embedding", true)) {
                    out.add(rawName)
                }
            }
        } else {
            val arr = root.optJSONArray("data") ?: root.optJSONArray("models") ?: JSONArray()
            val freeFirst = mutableListOf<String>()
            val rest = mutableListOf<String>()
            for (i in 0 until arr.length()) {
                val obj = arr.optJSONObject(i) ?: continue
                val id = obj.optString("id").ifEmpty { obj.optString("name") }.trim()
                if (id.isEmpty()) continue
                val lower = id.lowercase()
                if (lower.contains("embedding") || lower.contains("whisper") || lower.contains("tts") || lower.contains("moderation")) {
                    continue
                }
                if (provider == LlmProvider.OPENROUTER && (lower.endsWith(":free") || lower == "openrouter/free")) {
                    freeFirst.add(id)
                } else {
                    rest.add(id)
                }
            }
            if (provider == LlmProvider.OPENROUTER) {
                out.add("openrouter/free")
                out.addAll(freeFirst)
                out.addAll(rest.take(80))
            } else {
                out.addAll(rest.take(100))
            }
        }
        return out.toList()
    }
}

class AssistantService(private val context: Context) {
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.IO)
    private val recorder = VoiceRecorder()
    private val messages = mutableListOf<ChatMessage>()
    private var activeJob: Job? = null
    private var androidTts: TextToSpeech? = null
    private var ttsReady = false

    @Volatile
    private var snapshot = AssistantSnapshot(
        screenEnabled = AssistantConfig.load(context).screenVisionEnabled,
        hasApiKey = AssistantConfig.load(context).hasActiveApiKey()
    )

    init {
        try {
            androidTts = TextToSpeech(context.applicationContext) { status ->
                ttsReady = status == TextToSpeech.SUCCESS
            }
        } catch (_: Throwable) {
        }
    }

    fun snapshot(): AssistantSnapshot = snapshot

    fun liveVoiceLevel(): Float = recorder.liveLevel

    fun isListening(): Boolean = recorder.isRecording()

    fun refreshConfigStatus() {
        val cfg = AssistantConfig.load(context)
        snapshot = snapshot.copy(
            screenEnabled = cfg.screenVisionEnabled,
            hasApiKey = cfg.hasActiveApiKey()
        )
        ZekrimaApp.notifyStateChanged()
    }

    fun toggleScreenAccess() {
        val cfg = AssistantConfig.load(context)
        val next = !snapshot.screenEnabled
        AssistantConfig.save(context, cfg.copy(screenVisionEnabled = next))
        snapshot = snapshot.copy(screenEnabled = next)
        ZekrimaApp.notifyStateChanged()
    }

    fun clearConversation() {
        cancel()
        synchronized(messages) { messages.clear() }
        snapshot = snapshot.copy(
            phase = AssistantPhase.IDLE,
            messages = emptyList(),
            reply = "",
            error = null
        )
        ZekrimaApp.notifyStateChanged()
    }

    fun cancel() {
        recorder.stop()
        activeJob?.cancel()
        activeJob = null
        try {
            androidTts?.stop()
        } catch (_: Throwable) {
        }
        if (snapshot.phase.isBusy()) {
            snapshot = snapshot.copy(phase = AssistantPhase.IDLE)
            ZekrimaApp.notifyStateChanged()
        }
    }

    fun startVoiceListening(meetingTranscript: String? = null) {
        if (recorder.isRecording()) {
            recorder.stop()
            return
        }
        snapshot = snapshot.copy(phase = AssistantPhase.LISTENING, error = null)
        ZekrimaApp.notifyStateChanged()

        recorder.startHandsFree { wavBytes ->
            if (wavBytes == null) {
                snapshot = snapshot.copy(phase = AssistantPhase.IDLE)
                ZekrimaApp.notifyStateChanged()
                return@startHandsFree
            }
            activeJob?.cancel()
            activeJob = scope.launch {
                runVoiceTurn(wavBytes, meetingTranscript)
            }
        }
    }

    fun sendText(prompt: String, meetingTranscript: String? = null) {
        val clean = prompt.trim()
        if (clean.isEmpty()) return
        activeJob?.cancel()
        synchronized(messages) {
            messages.add(ChatMessage("user", clean))
            if (messages.size > 16) messages.removeAt(0)
        }
        snapshot = snapshot.copy(
            phase = AssistantPhase.THINKING,
            messages = synchronized(messages) { messages.toList() },
            error = null
        )
        ZekrimaApp.notifyStateChanged()

        activeJob = scope.launch {
            runTextTurn(clean, meetingTranscript)
        }
    }

    private fun runVoiceTurn(wavBytes: ByteArray, meetingTranscript: String?) {
        val cfg = AssistantConfig.load(context)
        if (!cfg.hasActiveApiKey()) {
            failTurn("API key belum diatur. Buka aplikasi Zekrima -> Settings.")
            return
        }

        // 1-step Gemini native audio when using Gemini provider
        if (cfg.llmProvider == LlmProvider.GEMINI && cfg.geminiApiKey.isNotBlank()) {
            snapshot = snapshot.copy(phase = AssistantPhase.THINKING)
            ZekrimaApp.notifyStateChanged()
            val screenBytes = if (snapshot.screenEnabled) {
                ScreenCaptureHelper.captureJpeg(context, ZekrimaApp.activeMediaProjection, cfg.screenHighRes)
            } else null

            val res = callGemini(
                cfg = cfg,
                userPrompt = "Dengarkan audio user dan jawab secara langsung dalam bahasa user.",
                audioWav = wavBytes,
                screenJpeg = screenBytes,
                meetingTranscript = meetingTranscript
            )
            handleTurnResult("[Voice Message]", res, cfg)
            return
        }

        // Otherwise STT via Groq Whisper / OpenAI Whisper -> then LLM
        snapshot = snapshot.copy(phase = AssistantPhase.TRANSCRIBING)
        ZekrimaApp.notifyStateChanged()
        val transcript = transcribeWhisper(cfg, wavBytes)
        if (transcript.isNullOrBlank()) {
            failTurn("Gagal transkripsi suara. Isi Gemini API key atau Groq API key di Settings.")
            return
        }
        synchronized(messages) {
            messages.add(ChatMessage("user", transcript))
            if (messages.size > 16) messages.removeAt(0)
        }
        snapshot = snapshot.copy(
            phase = AssistantPhase.THINKING,
            messages = synchronized(messages) { messages.toList() }
        )
        ZekrimaApp.notifyStateChanged()
        runTextTurn(transcript, meetingTranscript, alreadyRecordedUser = true)
    }

    private fun runTextTurn(prompt: String, meetingTranscript: String?, alreadyRecordedUser: Boolean = true) {
        val cfg = AssistantConfig.load(context)
        if (!cfg.hasActiveApiKey()) {
            failTurn("API key belum diatur. Buka aplikasi Zekrima -> Settings.")
            return
        }

        val screenBytes = if (snapshot.screenEnabled) {
            ScreenCaptureHelper.captureJpeg(context, ZekrimaApp.activeMediaProjection, cfg.screenHighRes)
        } else null

        val res = if (cfg.llmProvider == LlmProvider.GEMINI) {
            callGemini(cfg, prompt, null, screenBytes, meetingTranscript)
        } else {
            callOpenAiCompatible(cfg, prompt, screenBytes, meetingTranscript)
        }
        handleTurnResult(if (alreadyRecordedUser) null else prompt, res, cfg)
    }

    private fun handleTurnResult(userLabel: String?, result: Result<String>, cfg: AssistantConfig) {
        result.fold(
            onSuccess = { raw ->
                val parsed = ToolProtocol.parseAndClean(raw)
                if (parsed.actions.isNotEmpty()) {
                    SystemTools.executeActions(context, parsed.actions)
                }
                val finalReply = parsed.cleanReply.ifBlank { "Siap, sudah dilakukan!" }
                synchronized(messages) {
                    if (userLabel != null) messages.add(ChatMessage("user", userLabel))
                    messages.add(ChatMessage("assistant", finalReply))
                    while (messages.size > 16) messages.removeAt(0)
                }
                if (cfg.ttsEnabled) {
                    snapshot = snapshot.copy(
                        phase = AssistantPhase.SPEAKING,
                        messages = synchronized(messages) { messages.toList() },
                        reply = finalReply,
                        error = null
                    )
                    ZekrimaApp.notifyStateChanged()
                    speakReply(finalReply, cfg)
                }
                snapshot = snapshot.copy(
                    phase = AssistantPhase.DONE,
                    messages = synchronized(messages) { messages.toList() },
                    reply = finalReply,
                    error = null
                )
                ZekrimaApp.notifyStateChanged()
            },
            onFailure = { err ->
                failTurn(err.message ?: "Gagal menghubungi AI.")
            }
        )
    }

    private fun failTurn(msg: String) {
        snapshot = snapshot.copy(phase = AssistantPhase.ERROR, error = msg)
        ZekrimaApp.notifyStateChanged()
    }

    private fun buildSystemPrompt(cfg: AssistantConfig, meetingTranscript: String?): String {
        val sb = StringBuilder()
        sb.append(cfg.systemPrompt.trim())
        sb.append("\n\n").append(ToolProtocol.systemInstructionBlock())
        val notifs = LiveNotificationStore.summaryForAssistant()
        if (notifs.isNotEmpty()) {
            sb.append("\n\nRecent Android notifications:\n").append(notifs)
        }
        if (!meetingTranscript.isNullOrBlank()) {
            sb.append("\n\nActive Live Meeting Transcript (Cicada Mode):\n").append(meetingTranscript)
        }
        return sb.toString()
    }

    private fun callGemini(
        cfg: AssistantConfig,
        userPrompt: String,
        audioWav: ByteArray?,
        screenJpeg: ByteArray?,
        meetingTranscript: String?
    ): Result<String> {
        return try {
            val model = cfg.modelFor(LlmProvider.GEMINI)
            val endpoint =
                "https://generativelanguage.googleapis.com/v1beta/models/$model:generateContent?key=${cfg.geminiApiKey.trim()}"
            val parts = JSONArray()
            parts.put(JSONObject().put("text", userPrompt))
            if (audioWav != null) {
                parts.put(
                    JSONObject().put(
                        "inlineData",
                        JSONObject()
                            .put("mimeType", "audio/wav")
                            .put("data", Base64.encodeToString(audioWav, Base64.NO_WRAP))
                    )
                )
            }
            if (screenJpeg != null) {
                parts.put(
                    JSONObject().put(
                        "inlineData",
                        JSONObject()
                            .put("mimeType", "image/jpeg")
                            .put("data", Base64.encodeToString(screenJpeg, Base64.NO_WRAP))
                    )
                )
            }

            val contents = JSONArray()
            val hist = synchronized(messages) { messages.dropLast(1).takeLast(6) }
            for (m in hist) {
                val role = if (m.role == "assistant") "model" else "user"
                contents.put(
                    JSONObject()
                        .put("role", role)
                        .put("parts", JSONArray().put(JSONObject().put("text", m.content)))
                )
            }
            contents.put(JSONObject().put("role", "user").put("parts", parts))

            val payload = JSONObject()
                .put(
                    "systemInstruction",
                    JSONObject().put(
                        "parts",
                        JSONArray().put(JSONObject().put("text", buildSystemPrompt(cfg, meetingTranscript)))
                    )
                )
                .put("contents", contents)

            val resp = httpPostJson(endpoint, payload.toString(), null, false)
            val root = JSONObject(resp)
            val text = root.optJSONArray("candidates")
                ?.optJSONObject(0)
                ?.optJSONObject("content")
                ?.optJSONArray("parts")
                ?.let { pArr ->
                    (0 until pArr.length()).mapNotNull { pArr.optJSONObject(it)?.optString("text") }.joinToString("")
                }
                ?.trim()
            if (text.isNullOrEmpty()) Result.failure(RuntimeException("Respons kosong dari Gemini"))
            else Result.success(text)
        } catch (t: Throwable) {
            Result.failure(t)
        }
    }

    private fun callOpenAiCompatible(
        cfg: AssistantConfig,
        userPrompt: String,
        screenJpeg: ByteArray?,
        meetingTranscript: String?
    ): Result<String> {
        return try {
            val baseUrl = when (cfg.llmProvider) {
                LlmProvider.OPENROUTER -> "https://openrouter.ai/api/v1"
                LlmProvider.OPENCODE -> "https://opencode.ai/zen/v1"
                LlmProvider.NVIDIA -> "https://integrate.api.nvidia.com/v1"
                LlmProvider.GROQ -> "https://api.groq.com/openai/v1"
                LlmProvider.OPENAI -> cfg.openaiBaseUrl.trim().trimEnd('/').ifEmpty { "https://api.openai.com/v1" }
                LlmProvider.GEMINI -> "https://generativelanguage.googleapis.com/v1beta/openai"
            }
            val msgArr = JSONArray()
            msgArr.put(JSONObject().put("role", "system").put("content", buildSystemPrompt(cfg, meetingTranscript)))
            val hist = synchronized(messages) { messages.dropLast(1).takeLast(6) }
            for (m in hist) {
                msgArr.put(JSONObject().put("role", m.role).put("content", m.content))
            }
            if (screenJpeg != null) {
                val contentArr = JSONArray()
                contentArr.put(JSONObject().put("type", "text").put("text", userPrompt))
                val b64 = Base64.encodeToString(screenJpeg, Base64.NO_WRAP)
                contentArr.put(
                    JSONObject()
                        .put("type", "image_url")
                        .put("image_url", JSONObject().put("url", "data:image/jpeg;base64,$b64"))
                )
                msgArr.put(JSONObject().put("role", "user").put("content", contentArr))
            } else {
                msgArr.put(JSONObject().put("role", "user").put("content", userPrompt))
            }

            val payload = JSONObject()
                .put("model", cfg.modelFor())
                .put("messages", msgArr)
                .put("max_tokens", 700)

            val resp = httpPostJson(
                "$baseUrl/chat/completions",
                payload.toString(),
                cfg.apiKeyFor().takeIf { it.isNotBlank() },
                cfg.llmProvider == LlmProvider.OPENROUTER
            )
            val root = JSONObject(resp)
            val text = root.optJSONArray("choices")
                ?.optJSONObject(0)
                ?.optJSONObject("message")
                ?.optString("content")
                ?.trim()
            if (text.isNullOrEmpty()) Result.failure(RuntimeException("Respons kosong dari ${cfg.llmProvider.shortBadge}"))
            else Result.success(text)
        } catch (t: Throwable) {
            Result.failure(t)
        }
    }

    fun transcribeWhisper(cfg: AssistantConfig, wavBytes: ByteArray): String? {
        val (url, key, model) = when {
            cfg.groqApiKey.isNotBlank() -> Triple(
                "https://api.groq.com/openai/v1/audio/transcriptions",
                cfg.groqApiKey.trim(),
                "whisper-large-v3-turbo"
            )
            cfg.openaiApiKey.isNotBlank() -> Triple(
                "${cfg.openaiBaseUrl.trim().trimEnd('/')}/audio/transcriptions",
                cfg.openaiApiKey.trim(),
                "whisper-1"
            )
            else -> return null
        }

        val boundary = "----ZekrimaBoundary${System.currentTimeMillis()}"
        return try {
            val conn = (URL(url).openConnection() as HttpURLConnection).apply {
                requestMethod = "POST"
                doOutput = true
                connectTimeout = 12_000
                readTimeout = 20_000
                setRequestProperty("Authorization", "Bearer $key")
                setRequestProperty("Content-Type", "multipart/form-data; boundary=$boundary")
            }
            DataOutputStream(conn.outputStream).use { out ->
                out.writeBytes("--$boundary\r\n")
                out.writeBytes("Content-Disposition: form-data; name=\"model\"\r\n\r\n")
                out.writeBytes("$model\r\n")
                out.writeBytes("--$boundary\r\n")
                out.writeBytes("Content-Disposition: form-data; name=\"file\"; filename=\"audio.wav\"\r\n")
                out.writeBytes("Content-Type: audio/wav\r\n\r\n")
                out.write(wavBytes)
                out.writeBytes("\r\n--$boundary--\r\n")
            }
            val code = conn.responseCode
            val stream = if (code in 200..299) conn.inputStream else conn.errorStream
            val body = stream?.bufferedReader()?.use(BufferedReader::readText).orEmpty()
            conn.disconnect()
            if (code !in 200..299) return null
            JSONObject(body).optString("text").trim().takeIf { it.isNotEmpty() }
        } catch (_: Throwable) {
            null
        }
    }

    private fun speakReply(text: String, cfg: AssistantConfig) {
        // Prefer Android native low-latency TTS (or Gemini PCM TTS if configured)
        if (ttsReady && androidTts != null) {
            try {
                val loc = if (cfg.language.equals("en", true)) Locale.US else Locale.forLanguageTag("id-ID")
                androidTts?.language = loc
                androidTts?.speak(text.take(400), TextToSpeech.QUEUE_FLUSH, null, "zekrima-tts")
            } catch (_: Throwable) {
            }
        }
    }

    fun httpPostJson(url: String, jsonBody: String, bearerKey: String?, openRouterHeaders: Boolean): String {
        val conn = (URL(url).openConnection() as HttpURLConnection).apply {
            requestMethod = "POST"
            doOutput = true
            connectTimeout = 12_000
            readTimeout = 28_000
            setRequestProperty("Content-Type", "application/json; charset=utf-8")
            if (!bearerKey.isNullOrBlank()) {
                setRequestProperty("Authorization", "Bearer $bearerKey")
            }
            if (openRouterHeaders) {
                setRequestProperty("HTTP-Referer", "https://github.com/zekrima/zekrima_personal_asisstant")
                setRequestProperty("X-Title", "Zekrima Android")
            }
        }
        conn.outputStream.use { it.write(jsonBody.toByteArray(Charsets.UTF_8)) }
        val code = conn.responseCode
        val stream = if (code in 200..299) conn.inputStream else conn.errorStream
        val body = stream?.bufferedReader()?.use(BufferedReader::readText).orEmpty()
        conn.disconnect()
        if (code !in 200..299) {
            throw RuntimeException("HTTP $code: ${body.take(160)}")
        }
        return body
    }
}
