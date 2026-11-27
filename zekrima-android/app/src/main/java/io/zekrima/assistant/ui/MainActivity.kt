package io.zekrima.assistant.ui

import android.Manifest
import android.app.Activity
import android.content.Intent
import android.content.pm.PackageManager
import android.graphics.Color
import android.graphics.Typeface
import android.graphics.drawable.GradientDrawable
import android.net.Uri
import android.os.Build
import android.os.Bundle
import android.os.Handler
import android.os.Looper
import android.provider.Settings
import android.text.Editable
import android.text.InputType
import android.text.TextWatcher
import android.view.Gravity
import android.view.View
import android.widget.AdapterView
import android.widget.ArrayAdapter
import android.widget.Button
import android.widget.CheckBox
import android.widget.EditText
import android.widget.LinearLayout
import android.widget.ScrollView
import android.widget.Spinner
import android.widget.TextView
import android.widget.Toast
import androidx.core.app.ActivityCompat
import androidx.core.content.ContextCompat
import io.zekrima.assistant.ZekrimaApp
import io.zekrima.assistant.config.AssistantConfig
import io.zekrima.assistant.config.CICADA_LANGUAGES
import io.zekrima.assistant.config.GEMINI_TTS_VOICES
import io.zekrima.assistant.config.LlmProvider
import io.zekrima.assistant.services.IslandOverlayService
import io.zekrima.assistant.services.LiveNotificationStore
import io.zekrima.assistant.services.ModelDiscovery
import java.util.concurrent.atomic.AtomicInteger

class MainActivity : Activity() {

    private val handler = Handler(Looper.getMainLooper())
    private var cfg = AssistantConfig()
    private var activeProvider = LlmProvider.GEMINI
    private var availableModels: List<String> = LlmProvider.GEMINI.presets
    private val fetchGen = AtomicInteger(0)
    private var suppressEdit = false
    private var debounceRunnable: Runnable? = null

    private lateinit var overlayBtn: Button
    private lateinit var statusSubtitle: TextView
    private lateinit var permOverlayBtn: Button
    private lateinit var permNotifBtn: Button
    private lateinit var permMicBtn: Button
    private lateinit var permWriteBtn: Button
    private lateinit var permProjectionBtn: Button

    private lateinit var providerSpinner: Spinner
    private lateinit var modelSpinner: Spinner
    private lateinit var modelStatusTv: TextView
    private lateinit var modelCustomEt: EditText
    private lateinit var apiKeyHintTv: TextView
    private lateinit var apiKeyEt: EditText
    private lateinit var baseUrlEt: EditText
    private lateinit var groqKeyEt: EditText
    private lateinit var ttsCheck: CheckBox
    private lateinit var ttsVoiceSpinner: Spinner
    private lateinit var autoHideCheck: CheckBox
    private lateinit var cicadaTargetSpinner: Spinner
    private lateinit var cicadaReplySpinner: Spinner
    private lateinit var cicadaGlossaryEt: EditText

    private val d get() = resources.displayMetrics.density
    private fun dp(v: Int): Int = (v * d).toInt()

    private val stateListener: () -> Unit = {
        handler.post { refreshPermissionBadges() }
    }

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        cfg = AssistantConfig.load(this)
        activeProvider = cfg.llmProvider
        availableModels = activeProvider.presets

        setContentView(buildUi())
        bindCurrentProviderFields(autoFetch = true)
        ZekrimaApp.listeners.add(stateListener)
    }

    override fun onResume() {
        super.onResume()
        refreshPermissionBadges()
    }

    override fun onDestroy() {
        ZekrimaApp.listeners.remove(stateListener)
        debounceRunnable?.let { handler.removeCallbacks(it) }
        super.onDestroy()
    }

    private fun buildUi(): View {
        val scroll = ScrollView(this).apply {
            setBackgroundColor(Color.rgb(10, 10, 14))
        }
        val root = LinearLayout(this).apply {
            orientation = LinearLayout.VERTICAL
            setPadding(dp(18), dp(22), dp(18), dp(36))
        }
        scroll.addView(root)

        // Hero Header
        root.addView(TextView(this).apply {
            text = "Zekrima Dynamic Island"
            textSize = 23f
            setTypeface(null, Typeface.BOLD)
            setTextColor(Color.WHITE)
        })
        statusSubtitle = TextView(this).apply {
            text = "v0.4.0 • AI Personal Assistant & Cicada Live Meeting Translator"
            textSize = 12.5f
            setTextColor(Color.rgb(148, 163, 184))
            setPadding(0, dp(4), 0, dp(16))
        }
        root.addView(statusSubtitle)

        // Master Launch Card
        val heroCard = makeCard()
        overlayBtn = makePrimaryButton("▶ Aktifkan Dynamic Island", Color.rgb(56, 189, 248)) {
            toggleOverlayService()
        }
        heroCard.addView(overlayBtn)

        val quickRow = LinearLayout(this).apply {
            orientation = LinearLayout.HORIZONTAL
            setPadding(0, dp(10), 0, 0)
        }
        val testBtn = makeSecondaryButton("Test Toast") {
            sendOverlayAction(IslandOverlayService.ACTION_TEST_TOAST)
        }
        val cicadaBtn = makeSecondaryButton("Cicada Live") {
            if (ZekrimaApp.activeMediaProjection == null) {
                startActivity(
                    Intent(this, MediaProjectionActivity::class.java)
                        .putExtra(MediaProjectionActivity.EXTRA_PURPOSE, "cicada")
                )
            } else {
                sendOverlayAction(IslandOverlayService.ACTION_TOGGLE_CICADA)
            }
        }
        val chatBtn = makeSecondaryButton("Buka Chat") {
            sendOverlayAction(IslandOverlayService.ACTION_OPEN_CHAT)
        }
        quickRow.addView(testBtn, LinearLayout.LayoutParams(0, dp(38), 1f).apply { rightMargin = dp(6) })
        quickRow.addView(cicadaBtn, LinearLayout.LayoutParams(0, dp(38), 1f).apply { rightMargin = dp(6) })
        quickRow.addView(chatBtn, LinearLayout.LayoutParams(0, dp(38), 1f))
        heroCard.addView(quickRow)
        root.addView(heroCard)

        // Permissions Card
        val permCard = makeCard()
        permCard.addView(makeSectionTitle("Izin Sistem Android"))
        permOverlayBtn = makePermRow(permCard, "1. Floating Dynamic Island (Overlay)") {
            startActivity(Intent(Settings.ACTION_MANAGE_OVERLAY_PERMISSION, Uri.parse("package:$packageName")))
        }
        permNotifBtn = makePermRow(permCard, "2. Notifikasi & Kontrol Media") {
            startActivity(Intent(Settings.ACTION_NOTIFICATION_LISTENER_SETTINGS))
        }
        permMicBtn = makePermRow(permCard, "3. Mikrofon & Notifikasi App") {
            val perms = mutableListOf(Manifest.permission.RECORD_AUDIO)
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
                perms.add(Manifest.permission.POST_NOTIFICATIONS)
            }
            ActivityCompat.requestPermissions(this, perms.toTypedArray(), 2001)
        }
        permWriteBtn = makePermRow(permCard, "4. Kontrol Kecerahan Layar") {
            startActivity(Intent(Settings.ACTION_MANAGE_WRITE_SETTINGS, Uri.parse("package:$packageName")))
        }
        permProjectionBtn = makePermRow(permCard, "5. System Audio Loopback & Screen Vision") {
            startActivity(
                Intent(this, MediaProjectionActivity::class.java)
                    .putExtra(MediaProjectionActivity.EXTRA_PURPOSE, "ready")
            )
        }
        root.addView(permCard)

        // AI Brain & Auto-Detect Model Card
        val aiCard = makeCard()
        aiCard.addView(makeSectionTitle("AI Brain & Auto-Deteksi Model API"))

        aiCard.addView(makeLabel("LLM Provider:"))
        providerSpinner = makeSpinner(LlmProvider.entries.map { it.displayName })
        providerSpinner.setSelection(LlmProvider.entries.indexOf(activeProvider).coerceAtLeast(0))
        providerSpinner.onItemSelectedListener = object : AdapterView.OnItemSelectedListener {
            override fun onItemSelected(parent: AdapterView<*>?, view: View?, pos: Int, id: Long) {
                val next = LlmProvider.entries[pos]
                if (next == activeProvider) return
                cfg = cfg.withProviderKeyAndModel(activeProvider, apiKeyEt.text.toString(), modelCustomEt.text.toString())
                activeProvider = next
                cfg = cfg.copy(llmProvider = next)
                bindCurrentProviderFields(autoFetch = true)
            }

            override fun onNothingSelected(parent: AdapterView<*>?) = Unit
        }
        aiCard.addView(providerSpinner)

        apiKeyHintTv = makeLabel(activeProvider.keyHint)
        aiCard.addView(apiKeyHintTv)
        apiKeyEt = makeEditText("Masukkan API key...", password = true)
        aiCard.addView(apiKeyEt)

        val modelHeader = LinearLayout(this).apply {
            orientation = LinearLayout.HORIZONTAL
            gravity = Gravity.CENTER_VERTICAL
            setPadding(0, dp(8), 0, dp(4))
        }
        modelHeader.addView(
            makeLabel("Model Tersedia (otomatis terdeteksi dari API):"),
            LinearLayout.LayoutParams(0, LinearLayout.LayoutParams.WRAP_CONTENT, 1f)
        )
        val fetchBtn = makeSecondaryButton("↻ Cek Model") { triggerModelDiscovery(manual = true) }
        modelHeader.addView(fetchBtn, LinearLayout.LayoutParams(dp(112), dp(34)))
        aiCard.addView(modelHeader)

        modelSpinner = makeSpinner(availableModels)
        modelSpinner.onItemSelectedListener = object : AdapterView.OnItemSelectedListener {
            override fun onItemSelected(parent: AdapterView<*>?, view: View?, pos: Int, id: Long) {
                if (!suppressEdit && pos in availableModels.indices) {
                    suppressEdit = true
                    modelCustomEt.setText(availableModels[pos])
                    suppressEdit = false
                }
            }

            override fun onNothingSelected(parent: AdapterView<*>?) = Unit
        }
        aiCard.addView(modelSpinner)

        modelStatusTv = TextView(this).apply {
            text = "Isi API key untuk mendeteksi daftar model tersedia secara otomatis."
            textSize = 11f
            setTextColor(Color.rgb(148, 163, 184))
            setPadding(0, dp(4), 0, dp(6))
        }
        aiCard.addView(modelStatusTv)

        aiCard.addView(makeLabel("Model ID aktif (bisa diedit manual):"))
        modelCustomEt = makeEditText("gemini-2.5-flash / openrouter/free", password = false)
        aiCard.addView(modelCustomEt)

        aiCard.addView(makeLabel("Base URL (khusus OpenAI / Custom / Ollama):"))
        baseUrlEt = makeEditText("https://api.openai.com/v1", password = false)
        baseUrlEt.setText(cfg.openaiBaseUrl)
        aiCard.addView(baseUrlEt)

        aiCard.addView(makeLabel("Groq API Key untuk Whisper STT (opsional):"))
        groqKeyEt = makeEditText("gsk_...", password = true)
        groqKeyEt.setText(cfg.groqApiKey)
        aiCard.addView(groqKeyEt)

        // Debounced live model discovery when typing/pasting API key or Base URL
        val watcher = object : TextWatcher {
            override fun beforeTextChanged(s: CharSequence?, start: Int, count: Int, after: Int) = Unit
            override fun onTextChanged(s: CharSequence?, start: Int, before: Int, count: Int) = Unit
            override fun afterTextChanged(s: Editable?) {
                if (suppressEdit) return
                debounceRunnable?.let { handler.removeCallbacks(it) }
                debounceRunnable = Runnable { triggerModelDiscovery(manual = false) }.also {
                    handler.postDelayed(it, 650L)
                }
            }
        }
        apiKeyEt.addTextChangedListener(watcher)
        baseUrlEt.addTextChangedListener(watcher)

        ttsCheck = CheckBox(this).apply {
            text = "Aktifkan Voice Reply (TTS)"
            setTextColor(Color.WHITE)
            isChecked = cfg.ttsEnabled
            setPadding(0, dp(10), 0, dp(4))
        }
        aiCard.addView(ttsCheck)
        ttsVoiceSpinner = makeSpinner(GEMINI_TTS_VOICES)
        ttsVoiceSpinner.setSelection(GEMINI_TTS_VOICES.indexOf(cfg.ttsVoice).coerceAtLeast(0))
        aiCard.addView(ttsVoiceSpinner)
        root.addView(aiCard)

        // Cicada & Behavior Card
        val cicadaCard = makeCard()
        cicadaCard.addView(makeSectionTitle("Cicada Mode & Perilaku Island"))

        cicadaCard.addView(makeLabel("Bahasa Target Terjemahan Meeting:"))
        cicadaTargetSpinner = makeSpinner(CICADA_LANGUAGES.map { it.label })
        cicadaTargetSpinner.setSelection(
            CICADA_LANGUAGES.indexOfFirst { it.code.equals(cfg.cicadaTargetLang, true) }.coerceAtLeast(0)
        )
        cicadaCard.addView(cicadaTargetSpinner)

        cicadaCard.addView(makeLabel("Bahasa Quick-Reply (Bidirectional):"))
        cicadaReplySpinner = makeSpinner(CICADA_LANGUAGES.map { it.label })
        cicadaReplySpinner.setSelection(
            CICADA_LANGUAGES.indexOfFirst { it.code.equals(cfg.cicadaReplyLang, true) }.coerceAtLeast(1)
        )
        cicadaCard.addView(cicadaReplySpinner)

        cicadaCard.addView(makeLabel("Custom Glossary (pisahkan dengan ; contoh: Zekrima; Kubernetes):"))
        cicadaGlossaryEt = makeEditText("Istilah teknis / nama produk...", password = false)
        cicadaGlossaryEt.setText(cfg.cicadaGlossary)
        cicadaCard.addView(cicadaGlossaryEt)

        autoHideCheck = CheckBox(this).apply {
            text = "Auto-Hide ke tepi atas saat idle 10 detik"
            setTextColor(Color.WHITE)
            isChecked = cfg.autoHide
            setPadding(0, dp(8), 0, dp(4))
        }
        cicadaCard.addView(autoHideCheck)
        root.addView(cicadaCard)

        // Save & Apply button
        val saveBtn = makePrimaryButton("Simpan & Terapkan Pengaturan", Color.rgb(16, 185, 129)) {
            saveConfig()
        }
        root.addView(saveBtn)

        refreshPermissionBadges()
        return scroll
    }

    private fun bindCurrentProviderFields(autoFetch: Boolean) {
        suppressEdit = true
        apiKeyHintTv.text = activeProvider.keyHint
        apiKeyEt.setText(cfg.apiKeyFor(activeProvider))
        val curModel = cfg.modelFor(activeProvider)
        availableModels = activeProvider.presets
        updateModelSpinner(availableModels, curModel)
        modelCustomEt.setText(curModel)
        baseUrlEt.isEnabled = activeProvider == LlmProvider.OPENAI
        baseUrlEt.alpha = if (activeProvider == LlmProvider.OPENAI) 1f else 0.45f
        suppressEdit = false
        if (autoFetch) {
            triggerModelDiscovery(manual = false)
        }
    }

    private fun updateModelSpinner(models: List<String>, selectedModel: String) {
        availableModels = models
        val adapter = ArrayAdapter(this, android.R.layout.simple_spinner_item, models).apply {
            setDropDownViewResource(android.R.layout.simple_spinner_dropdown_item)
        }
        modelSpinner.adapter = adapter
        val idx = models.indexOfFirst { it.equals(selectedModel, true) }
        if (idx >= 0) {
            modelSpinner.setSelection(idx)
        }
    }

    private fun triggerModelDiscovery(manual: Boolean) {
        val provider = activeProvider
        val key = apiKeyEt.text.toString().trim()
        val base = baseUrlEt.text.toString().trim()
        if (key.isEmpty() && provider != LlmProvider.OPENROUTER && provider != LlmProvider.OPENAI) {
            modelStatusTv.text = if (manual) {
                "Masukkan API key dulu untuk mendeteksi model dari API."
            } else {
                "Preset bawaan aktif — isi API key untuk auto-deteksi model API."
            }
            return
        }
        val gen = fetchGen.incrementAndGet()
        modelStatusTv.text = "⏳ Mendeteksi model dari ${provider.shortBadge}..."
        Thread {
            val res = ModelDiscovery.fetchAvailableModels(provider, key, base)
            handler.post {
                if (fetchGen.get() != gen || activeProvider != provider) return@post
                res.fold(
                    onSuccess = { models ->
                        suppressEdit = true
                        val cur = modelCustomEt.text.toString().trim()
                        updateModelSpinner(models, cur)
                        if (cur.isEmpty() || !models.contains(cur)) {
                            modelCustomEt.setText(models.first())
                            modelSpinner.setSelection(0)
                        }
                        suppressEdit = false
                        modelStatusTv.text = "✓ ${models.size} model terdeteksi otomatis dari ${provider.shortBadge}!"
                        modelStatusTv.setTextColor(Color.rgb(52, 211, 153))
                    },
                    onFailure = { err ->
                        modelStatusTv.text = "! ${err.message} (preset bawaan tetap bisa dipakai)"
                        modelStatusTv.setTextColor(Color.rgb(251, 191, 36))
                    }
                )
            }
        }.start()
    }

    private fun saveConfig() {
        cfg = cfg.withProviderKeyAndModel(
            activeProvider,
            apiKeyEt.text.toString(),
            modelCustomEt.text.toString()
        ).copy(
            llmProvider = activeProvider,
            openaiBaseUrl = baseUrlEt.text.toString().trim().ifEmpty { "https://api.openai.com/v1" },
            groqApiKey = groqKeyEt.text.toString().trim(),
            ttsEnabled = ttsCheck.isChecked,
            ttsVoice = GEMINI_TTS_VOICES[ttsVoiceSpinner.selectedItemPosition.coerceIn(0, GEMINI_TTS_VOICES.lastIndex)],
            autoHide = autoHideCheck.isChecked,
            cicadaTargetLang = CICADA_LANGUAGES[cicadaTargetSpinner.selectedItemPosition.coerceIn(0, CICADA_LANGUAGES.lastIndex)].code,
            cicadaReplyLang = CICADA_LANGUAGES[cicadaReplySpinner.selectedItemPosition.coerceIn(0, CICADA_LANGUAGES.lastIndex)].code,
            cicadaGlossary = cicadaGlossaryEt.text.toString().trim()
        )
        AssistantConfig.save(this, cfg)
        if (ZekrimaApp.overlayRunning) {
            sendOverlayAction(IslandOverlayService.ACTION_REFRESH_CONFIG)
        }
        Toast.makeText(this, "Pengaturan Zekrima tersimpan & diterapkan!", Toast.LENGTH_SHORT).show()
    }

    private fun toggleOverlayService() {
        if (!Settings.canDrawOverlays(this)) {
            Toast.makeText(this, "Aktifkan izin Floating Overlay dulu ya!", Toast.LENGTH_SHORT).show()
            startActivity(Intent(Settings.ACTION_MANAGE_OVERLAY_PERMISSION, Uri.parse("package:$packageName")))
            return
        }
        val intent = Intent(this, IslandOverlayService::class.java).setAction(
            if (ZekrimaApp.overlayRunning) IslandOverlayService.ACTION_STOP else IslandOverlayService.ACTION_START
        )
        if (ZekrimaApp.overlayRunning) {
            startService(intent)
        } else {
            ContextCompat.startForegroundService(this, intent)
        }
        handler.postDelayed({ refreshPermissionBadges() }, 250L)
    }

    private fun sendOverlayAction(action: String) {
        if (!Settings.canDrawOverlays(this)) {
            Toast.makeText(this, "Aktifkan Dynamic Island & izin Overlay dulu.", Toast.LENGTH_SHORT).show()
            return
        }
        val intent = Intent(this, IslandOverlayService::class.java).setAction(action)
        ContextCompat.startForegroundService(this, intent)
    }

    private fun refreshPermissionBadges() {
        val running = ZekrimaApp.overlayRunning
        overlayBtn.text = if (running) "■ Matikan Dynamic Island" else "▶ Aktifkan Dynamic Island"
        (overlayBtn.background as? GradientDrawable)?.setColor(
            if (running) Color.rgb(239, 68, 68) else Color.rgb(56, 189, 248)
        )

        stylePermBtn(permOverlayBtn, Settings.canDrawOverlays(this))
        stylePermBtn(permNotifBtn, LiveNotificationStore.listenerConnected)
        val hasMic = ContextCompat.checkSelfPermission(this, Manifest.permission.RECORD_AUDIO) == PackageManager.PERMISSION_GRANTED
        stylePermBtn(permMicBtn, hasMic)
        stylePermBtn(permWriteBtn, Settings.System.canWrite(this))
        stylePermBtn(permProjectionBtn, ZekrimaApp.activeMediaProjection != null)
    }

    private fun stylePermBtn(btn: Button, granted: Boolean) {
        btn.text = if (granted) "✓ Aktif" else "Aktifkan"
        btn.setTextColor(if (granted) Color.rgb(8, 10, 14) else Color.WHITE)
        btn.background = GradientDrawable().apply {
            setColor(if (granted) Color.rgb(16, 185, 129) else Color.rgb(51, 65, 85))
            cornerRadius = dp(12).toFloat()
        }
    }

    private fun makeCard(): LinearLayout = LinearLayout(this).apply {
        orientation = LinearLayout.VERTICAL
        setPadding(dp(14), dp(14), dp(14), dp(14))
        background = GradientDrawable().apply {
            setColor(Color.rgb(20, 23, 32))
            cornerRadius = dp(18).toFloat()
            setStroke(dp(1), Color.argb(35, 255, 255, 255))
        }
        layoutParams = LinearLayout.LayoutParams(
            LinearLayout.LayoutParams.MATCH_PARENT,
            LinearLayout.LayoutParams.WRAP_CONTENT
        ).apply { bottomMargin = dp(14) }
    }

    private fun makeSectionTitle(title: String): TextView = TextView(this).apply {
        text = title
        textSize = 15f
        setTypeface(null, Typeface.BOLD)
        setTextColor(Color.WHITE)
        setPadding(0, 0, 0, dp(10))
    }

    private fun makeLabel(txt: String): TextView = TextView(this).apply {
        text = txt
        textSize = 12f
        setTextColor(Color.rgb(186, 196, 214))
        setPadding(0, dp(8), 0, dp(4))
    }

    private fun makeEditText(hintTxt: String, password: Boolean): EditText = EditText(this).apply {
        hint = hintTxt
        setHintTextColor(Color.rgb(100, 116, 139))
        setTextColor(Color.WHITE)
        textSize = 13f
        inputType = if (password) {
            InputType.TYPE_CLASS_TEXT or InputType.TYPE_TEXT_VARIATION_PASSWORD
        } else {
            InputType.TYPE_CLASS_TEXT
        }
        setPadding(dp(12), dp(10), dp(12), dp(10))
        background = GradientDrawable().apply {
            setColor(Color.rgb(13, 16, 24))
            cornerRadius = dp(12).toFloat()
            setStroke(dp(1), Color.argb(45, 255, 255, 255))
        }
    }

    private fun makeSpinner(items: List<String>): Spinner = Spinner(this).apply {
        adapter = ArrayAdapter(this@MainActivity, android.R.layout.simple_spinner_item, items).apply {
            setDropDownViewResource(android.R.layout.simple_spinner_dropdown_item)
        }
        setPadding(dp(8), dp(6), dp(8), dp(6))
        background = GradientDrawable().apply {
            setColor(Color.rgb(28, 33, 46))
            cornerRadius = dp(12).toFloat()
        }
    }

    private fun makePrimaryButton(label: String, bg: Int, onClick: () -> Unit): Button = Button(this).apply {
        text = label
        isAllCaps = false
        textSize = 14f
        setTypeface(null, Typeface.BOLD)
        setTextColor(Color.rgb(8, 10, 14))
        background = GradientDrawable().apply {
            setColor(bg)
            cornerRadius = dp(14).toFloat()
        }
        layoutParams = LinearLayout.LayoutParams(LinearLayout.LayoutParams.MATCH_PARENT, dp(46))
        setOnClickListener { onClick() }
    }

    private fun makeSecondaryButton(label: String, onClick: () -> Unit): Button = Button(this).apply {
        text = label
        isAllCaps = false
        textSize = 12f
        setTextColor(Color.WHITE)
        background = GradientDrawable().apply {
            setColor(Color.rgb(36, 42, 58))
            cornerRadius = dp(12).toFloat()
        }
        setOnClickListener { onClick() }
    }

    private fun makePermRow(parent: LinearLayout, title: String, onClick: () -> Unit): Button {
        val row = LinearLayout(this).apply {
            orientation = LinearLayout.HORIZONTAL
            gravity = Gravity.CENTER_VERTICAL
            setPadding(0, dp(5), 0, dp(5))
        }
        val tv = TextView(this).apply {
            text = title
            textSize = 12.5f
            setTextColor(Color.rgb(226, 232, 240))
        }
        val btn = Button(this).apply {
            isAllCaps = false
            textSize = 11.5f
            setOnClickListener { onClick() }
        }
        row.addView(tv, LinearLayout.LayoutParams(0, LinearLayout.LayoutParams.WRAP_CONTENT, 1f))
        row.addView(btn, LinearLayout.LayoutParams(dp(88), dp(34)))
        parent.addView(row)
        return btn
    }
}
