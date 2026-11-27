package io.zekrima.assistant

import io.zekrima.assistant.config.LlmProvider
import io.zekrima.assistant.render.PillGeometry
import io.zekrima.assistant.render.PillMode
import io.zekrima.assistant.render.PillSprings
import io.zekrima.assistant.render.ZekiCicadaAccessory
import io.zekrima.assistant.render.ZekiEngine
import io.zekrima.assistant.render.ZekiState
import io.zekrima.assistant.services.CicadaTextParser
import io.zekrima.assistant.services.EchoFilter
import io.zekrima.assistant.services.ModelDiscovery
import io.zekrima.assistant.services.PomodoroTimer
import io.zekrima.assistant.services.SpeechSegmenter
import io.zekrima.assistant.services.ToolAction
import io.zekrima.assistant.services.ToolProtocol
import io.zekrima.assistant.services.WavEncoder
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertTrue
import org.junit.Test
import kotlin.math.sin

class ZekrimaCoreTests {

    @Test
    fun pillSpringsConvergeToAutoHiddenAndExpanded() {
        val springs = PillSprings()
        springs.setMode(PillMode.AUTO_HIDDEN, 360f)
        repeat(120) { springs.update(0.016f) }
        assertTrue(springs.isSettled())
        assertEquals(PillGeometry.AUTO_HIDDEN_HEIGHT, springs.height.current, 0.2f)

        springs.setMode(PillMode.EXPANDED_CICADA, 360f)
        repeat(120) { springs.update(0.016f) }
        assertTrue(springs.isSettled())
        assertEquals(PillGeometry.EXPANDED_CICADA_HEIGHT, springs.height.current, 0.2f)
    }

    @Test
    fun zekiEngineHandlesPokeSleepAndCicadaModes() {
        val zeki = ZekiEngine()
        assertEquals(ZekiState.IDLE, zeki.state)
        zeki.poke()
        assertEquals(ZekiState.HAPPY, zeki.state)
        zeki.setCicadaMode(ZekiCicadaAccessory.HEADSET, 0.7f)
        assertEquals(ZekiCicadaAccessory.HEADSET, zeki.cicadaAccessory)
        assertTrue(zeki.isHighEnergy())
    }

    @Test
    fun toolProtocolParsesMultipleDeviceTools() {
        val raw = "Oke, volume diatur ke 35% dan Spotify dibuka.\n[[TOOL:set_volume:35]] [[TOOL:open_app:Spotify]]"
        val res = ToolProtocol.parseAndClean(raw)
        assertEquals("Oke, volume diatur ke 35% dan Spotify dibuka.", res.cleanReply)
        assertEquals(2, res.actions.size)
        assertEquals(ToolAction.SetVolume(35, null), res.actions[0])
        assertEquals(ToolAction.OpenApp("Spotify"), res.actions[1])
    }

    @Test
    fun speechSegmenterEmitsPhraseOnPauseAfterSpeech() {
        val seg = SpeechSegmenter(16_000)
        // Generate voiced 440Hz tone for ~450ms (15 frames of 30ms)
        val voiced = ShortArray(480) { i -> (sin(i * 2.0 * Math.PI * 440.0 / 16_000.0) * 14_000).toInt().toShort() }
        val silence = ShortArray(480) { 0 }

        var emitted: ShortArray? = null
        repeat(15) { seg.feedFrame(voiced) }
        repeat(24) {
            val (_, out) = seg.feedFrame(silence)
            if (out != null) emitted = out
        }
        assertNotNull("Expected SpeechSegmenter to emit utterance after pause", emitted)
        assertTrue(emitted!!.size >= 480 * 6)
    }

    @Test
    fun echoFilterDropsSpeakerBleedAndKeepsDistinctSpeech() {
        val filter = EchoFilter()
        val t0 = 10_000L
        filter.recordRemote("Please deploy the release build to production today", t0)
        assertTrue(filter.isLikelyEcho("deploy the release build to production", t0 + 1_200L))
        assertFalse(filter.isLikelyEcho("Baik, saya akan cek servernya sekarang", t0 + 1_500L))
    }

    @Test
    fun cicadaParsersHandleThreeLineAndGoogleGtx() {
        val parsed = CicadaTextParser.parseThreeLineHeard(
            "LANG: EN\nORIGINAL: Let's ship version four\nTRANSLATION: Ayo rilis versi empat"
        )
        assertNotNull(parsed)
        assertEquals("EN", parsed!!.first)
        assertEquals("Ayo rilis versi empat", parsed.third)

        val gtx = CicadaTextParser.parseGoogleGtx("""[[["Halo dunia","Hello world",null,null,1]],null,"en"]""")
        assertNotNull(gtx)
        assertEquals("EN", gtx!!.first)
        assertEquals("Halo dunia", gtx.second)

        assertTrue(CicadaTextParser.isHallucinationOrNoise("[Music]"))
        assertFalse(CicadaTextParser.isHallucinationOrNoise("Meeting dimulai jam sepuluh"))
    }

    @Test
    fun modelDiscoveryParsesGeminiAndOpenRouterJson() {
        val geminiJson = """{"models":[
            {"name":"models/gemini-2.5-flash","supportedGenerationMethods":["generateContent"]},
            {"name":"models/text-embedding-004","supportedGenerationMethods":["embedContent"]}
        ]}"""
        val gModels = ModelDiscovery.parseModelListJson(LlmProvider.GEMINI, geminiJson)
        assertEquals(listOf("gemini-2.5-flash"), gModels)

        val orJson = """{"data":[
            {"id":"openai/gpt-4o-mini"},
            {"id":"meta-llama/llama-3.3-70b-instruct:free"}
        ]}"""
        val orModels = ModelDiscovery.parseModelListJson(LlmProvider.OPENROUTER, orJson)
        assertTrue(orModels.first() == "openrouter/free")
        assertTrue(orModels.contains("meta-llama/llama-3.3-70b-instruct:free"))
    }

    @Test
    fun wavEncoderProducesValidRiffHeader() {
        val pcm = ShortArray(320) { 100 }
        val wav = WavEncoder.encodePcm16Mono(pcm)
        assertEquals(44 + 640, wav.size)
        assertEquals("RIFF", String(wav, 0, 4, Charsets.US_ASCII))
        assertEquals("WAVE", String(wav, 8, 4, Charsets.US_ASCII))
    }

    @Test
    fun pomodoroTimerCountsDownAndCompletes() {
        val timer = PomodoroTimer()
        timer.selectPreset(2)
        timer.toggleStartPause()
        assertFalse(timer.tick())
        assertTrue(timer.tick())
        assertFalse(timer.running)
        assertEquals("00:00", timer.formatted())
    }
}
