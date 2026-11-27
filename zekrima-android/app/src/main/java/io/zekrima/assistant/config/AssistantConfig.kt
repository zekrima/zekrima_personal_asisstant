package io.zekrima.assistant.config

import android.content.Context
import android.content.SharedPreferences

enum class LlmProvider(
    val id: String,
    val displayName: String,
    val shortBadge: String,
    val keyHint: String,
    val defaultModel: String,
    val presets: List<String>
) {
    GEMINI(
        id = "gemini",
        displayName = "Google Gemini (Free AI Studio + Native Audio)",
        shortBadge = "Gemini",
        keyHint = "Gemini API Key (AIza... dari aistudio.google.com)",
        defaultModel = "gemini-2.5-flash",
        presets = listOf(
            "gemini-2.5-flash",
            "gemini-2.5-flash-lite",
            "gemini-3.8-flash",
            "gemini-2.0-flash"
        )
    ),
    OPENROUTER(
        id = "openrouter",
        displayName = "OpenRouter (Free Models :free)",
        shortBadge = "OpenRouter",
        keyHint = "OpenRouter API Key (sk-or-v1-...)",
        defaultModel = "openrouter/free",
        presets = listOf(
            "openrouter/free",
            "google/gemini-2.0-flash-exp:free",
            "meta-llama/llama-3.3-70b-instruct:free",
            "qwen/qwen-2.5-72b-instruct:free",
            "deepseek/deepseek-chat-v3-0324:free"
        )
    ),
    OPENCODE(
        id = "opencode",
        displayName = "OpenCode Zen (Free Models)",
        shortBadge = "OpenCode",
        keyHint = "OpenCode Zen API Key",
        defaultModel = "big-pickle",
        presets = listOf("big-pickle", "glm-4.7-free", "grok-code")
    ),
    NVIDIA(
        id = "nvidia",
        displayName = "NVIDIA NIM (Free Credits)",
        shortBadge = "NVIDIA",
        keyHint = "NVIDIA API Key (nvapi-...)",
        defaultModel = "meta/llama-3.3-70b-instruct",
        presets = listOf(
            "meta/llama-3.3-70b-instruct",
            "deepseek-ai/deepseek-r1",
            "qwen/qwen2.5-72b-instruct"
        )
    ),
    GROQ(
        id = "groq",
        displayName = "Groq (Ultra-Fast Llama + Whisper STT)",
        shortBadge = "Groq",
        keyHint = "Groq API Key (gsk_...)",
        defaultModel = "llama-3.3-70b-versatile",
        presets = listOf(
            "llama-3.3-70b-versatile",
            "llama-3.1-8b-instant",
            "gemma2-9b-it"
        )
    ),
    OPENAI(
        id = "openai",
        displayName = "OpenAI / Custom / Ollama",
        shortBadge = "OpenAI",
        keyHint = "API Key (opsional untuk Ollama lokal)",
        defaultModel = "gpt-4o-mini",
        presets = listOf("gpt-4o-mini", "gpt-4.1-mini", "llama3.2", "qwen2.5")
    );

    companion object {
        fun fromId(id: String?): LlmProvider =
            entries.firstOrNull { it.id.equals(id, ignoreCase = true) } ?: GEMINI
    }
}

data class CicadaLanguage(val code: String, val label: String, val badge: String)

val CICADA_LANGUAGES = listOf(
    CicadaLanguage("id", "Indonesia", "ID"),
    CicadaLanguage("en", "English", "EN"),
    CicadaLanguage("ja", "Japanese (日本語)", "JA"),
    CicadaLanguage("ko", "Korean (한국어)", "KO"),
    CicadaLanguage("zh", "Chinese (中文)", "ZH"),
    CicadaLanguage("es", "Spanish (Español)", "ES"),
    CicadaLanguage("fr", "French (Français)", "FR"),
    CicadaLanguage("de", "German (Deutsch)", "DE"),
    CicadaLanguage("ar", "Arabic (العربية)", "AR")
)

fun cicadaLangBadge(code: String): String =
    CICADA_LANGUAGES.firstOrNull { it.code.equals(code, ignoreCase = true) }?.badge
        ?: code.uppercase().take(2).ifEmpty { "ID" }

fun cicadaLangName(code: String): String =
    CICADA_LANGUAGES.firstOrNull { it.code.equals(code, ignoreCase = true) }?.label ?: code

val GEMINI_TTS_VOICES = listOf("Kore", "Puck", "Aoede", "Charon", "Fenrir", "Zephyr")

data class AssistantConfig(
    val llmProvider: LlmProvider = LlmProvider.GEMINI,
    val geminiApiKey: String = "",
    val geminiModel: String = LlmProvider.GEMINI.defaultModel,
    val openrouterApiKey: String = "",
    val openrouterModel: String = LlmProvider.OPENROUTER.defaultModel,
    val opencodeApiKey: String = "",
    val opencodeModel: String = LlmProvider.OPENCODE.defaultModel,
    val nvidiaApiKey: String = "",
    val nvidiaModel: String = LlmProvider.NVIDIA.defaultModel,
    val groqApiKey: String = "",
    val groqModel: String = LlmProvider.GROQ.defaultModel,
    val openaiApiKey: String = "",
    val openaiModel: String = LlmProvider.OPENAI.defaultModel,
    val openaiBaseUrl: String = "https://api.openai.com/v1",
    val language: String = "id",
    val systemPrompt: String =
        "Kamu adalah Zeki, asisten AI personal di dalam Dynamic Island Android yang cerdas, ringkas, ramah, dan membantu. Jawab dalam bahasa user secara padat dan jelas.",
    val screenVisionEnabled: Boolean = true,
    val screenHighRes: Boolean = false,
    val ttsEnabled: Boolean = false,
    val ttsVoice: String = "Kore",
    val autoHide: Boolean = true,
    val cicadaTargetLang: String = "id",
    val cicadaReplyLang: String = "en",
    val cicadaShowOriginal: Boolean = true,
    val cicadaGlossary: String = ""
) {
    fun apiKeyFor(provider: LlmProvider = llmProvider): String = when (provider) {
        LlmProvider.GEMINI -> geminiApiKey
        LlmProvider.OPENROUTER -> openrouterApiKey
        LlmProvider.OPENCODE -> opencodeApiKey
        LlmProvider.NVIDIA -> nvidiaApiKey
        LlmProvider.GROQ -> groqApiKey
        LlmProvider.OPENAI -> openaiApiKey
    }

    fun modelFor(provider: LlmProvider = llmProvider): String = when (provider) {
        LlmProvider.GEMINI -> geminiModel.ifBlank { provider.defaultModel }
        LlmProvider.OPENROUTER -> openrouterModel.ifBlank { provider.defaultModel }
        LlmProvider.OPENCODE -> opencodeModel.ifBlank { provider.defaultModel }
        LlmProvider.NVIDIA -> nvidiaModel.ifBlank { provider.defaultModel }
        LlmProvider.GROQ -> groqModel.ifBlank { provider.defaultModel }
        LlmProvider.OPENAI -> openaiModel.ifBlank { provider.defaultModel }
    }

    fun hasActiveApiKey(): Boolean =
        apiKeyFor().isNotBlank() || llmProvider == LlmProvider.OPENAI

    fun withProviderKeyAndModel(provider: LlmProvider, key: String, model: String): AssistantConfig {
        val k = key.trim()
        val m = model.trim().ifEmpty { provider.defaultModel }
        return when (provider) {
            LlmProvider.GEMINI -> copy(geminiApiKey = k, geminiModel = m)
            LlmProvider.OPENROUTER -> copy(openrouterApiKey = k, openrouterModel = m)
            LlmProvider.OPENCODE -> copy(opencodeApiKey = k, opencodeModel = m)
            LlmProvider.NVIDIA -> copy(nvidiaApiKey = k, nvidiaModel = m)
            LlmProvider.GROQ -> copy(groqApiKey = k, groqModel = m)
            LlmProvider.OPENAI -> copy(openaiApiKey = k, openaiModel = m)
        }
    }

    companion object {
        private const val PREFS_NAME = "zekrima_assistant_config"

        private fun prefs(ctx: Context): SharedPreferences =
            ctx.getSharedPreferences(PREFS_NAME, Context.MODE_PRIVATE)

        fun load(ctx: Context): AssistantConfig {
            val p = prefs(ctx)
            val def = AssistantConfig()
            return AssistantConfig(
                llmProvider = LlmProvider.fromId(p.getString("llm_provider", def.llmProvider.id)),
                geminiApiKey = p.getString("gemini_api_key", "") ?: "",
                geminiModel = p.getString("gemini_model", def.geminiModel) ?: def.geminiModel,
                openrouterApiKey = p.getString("openrouter_api_key", "") ?: "",
                openrouterModel = p.getString("openrouter_model", def.openrouterModel) ?: def.openrouterModel,
                opencodeApiKey = p.getString("opencode_api_key", "") ?: "",
                opencodeModel = p.getString("opencode_model", def.opencodeModel) ?: def.opencodeModel,
                nvidiaApiKey = p.getString("nvidia_api_key", "") ?: "",
                nvidiaModel = p.getString("nvidia_model", def.nvidiaModel) ?: def.nvidiaModel,
                groqApiKey = p.getString("groq_api_key", "") ?: "",
                groqModel = p.getString("groq_model", def.groqModel) ?: def.groqModel,
                openaiApiKey = p.getString("openai_api_key", "") ?: "",
                openaiModel = p.getString("openai_model", def.openaiModel) ?: def.openaiModel,
                openaiBaseUrl = p.getString("openai_base_url", def.openaiBaseUrl) ?: def.openaiBaseUrl,
                language = p.getString("language", def.language) ?: def.language,
                systemPrompt = p.getString("system_prompt", def.systemPrompt) ?: def.systemPrompt,
                screenVisionEnabled = p.getBoolean("screen_vision_enabled", def.screenVisionEnabled),
                screenHighRes = p.getBoolean("screen_high_res", def.screenHighRes),
                ttsEnabled = p.getBoolean("tts_enabled", def.ttsEnabled),
                ttsVoice = p.getString("tts_voice", def.ttsVoice) ?: def.ttsVoice,
                autoHide = p.getBoolean("auto_hide", def.autoHide),
                cicadaTargetLang = p.getString("cicada_target_lang", def.cicadaTargetLang) ?: def.cicadaTargetLang,
                cicadaReplyLang = p.getString("cicada_reply_lang", def.cicadaReplyLang) ?: def.cicadaReplyLang,
                cicadaShowOriginal = p.getBoolean("cicada_show_original", def.cicadaShowOriginal),
                cicadaGlossary = p.getString("cicada_glossary", def.cicadaGlossary) ?: def.cicadaGlossary
            )
        }

        fun save(ctx: Context, cfg: AssistantConfig) {
            prefs(ctx).edit()
                .putString("llm_provider", cfg.llmProvider.id)
                .putString("gemini_api_key", cfg.geminiApiKey)
                .putString("gemini_model", cfg.geminiModel)
                .putString("openrouter_api_key", cfg.openrouterApiKey)
                .putString("openrouter_model", cfg.openrouterModel)
                .putString("opencode_api_key", cfg.opencodeApiKey)
                .putString("opencode_model", cfg.opencodeModel)
                .putString("nvidia_api_key", cfg.nvidiaApiKey)
                .putString("nvidia_model", cfg.nvidiaModel)
                .putString("groq_api_key", cfg.groqApiKey)
                .putString("groq_model", cfg.groqModel)
                .putString("openai_api_key", cfg.openaiApiKey)
                .putString("openai_model", cfg.openaiModel)
                .putString("openai_base_url", cfg.openaiBaseUrl)
                .putString("language", cfg.language)
                .putString("system_prompt", cfg.systemPrompt)
                .putBoolean("screen_vision_enabled", cfg.screenVisionEnabled)
                .putBoolean("screen_high_res", cfg.screenHighRes)
                .putBoolean("tts_enabled", cfg.ttsEnabled)
                .putString("tts_voice", cfg.ttsVoice)
                .putBoolean("auto_hide", cfg.autoHide)
                .putString("cicada_target_lang", cfg.cicadaTargetLang)
                .putString("cicada_reply_lang", cfg.cicadaReplyLang)
                .putBoolean("cicada_show_original", cfg.cicadaShowOriginal)
                .putString("cicada_glossary", cfg.cicadaGlossary)
                .apply()
        }
    }
}
