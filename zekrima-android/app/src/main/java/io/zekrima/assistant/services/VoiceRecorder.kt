package io.zekrima.assistant.services

import android.annotation.SuppressLint
import android.media.AudioFormat
import android.media.AudioRecord
import android.media.MediaRecorder
import java.io.ByteArrayOutputStream
import java.nio.ByteBuffer
import java.nio.ByteOrder
import java.util.concurrent.atomic.AtomicBoolean
import kotlin.math.sqrt

object WavEncoder {
    const val SAMPLE_RATE = 16000

    fun encodePcm16Mono(samples: ShortArray, sampleRate: Int = SAMPLE_RATE): ByteArray {
        val dataSize = samples.size * 2
        val byteRate = sampleRate * 2
        val out = ByteBuffer.allocate(44 + dataSize).order(ByteOrder.LITTLE_ENDIAN)
        out.put("RIFF".toByteArray(Charsets.US_ASCII))
        out.putInt(36 + dataSize)
        out.put("WAVE".toByteArray(Charsets.US_ASCII))
        out.put("fmt ".toByteArray(Charsets.US_ASCII))
        out.putInt(16) // PCM chunk size
        out.putShort(1) // PCM format
        out.putShort(1) // Mono
        out.putInt(sampleRate)
        out.putInt(byteRate)
        out.putShort(2) // Block align
        out.putShort(16) // Bits per sample
        out.put("data".toByteArray(Charsets.US_ASCII))
        out.putInt(dataSize)
        for (s in samples) {
            out.putShort(s)
        }
        return out.array()
    }
}

/**
 * 16 kHz mono PCM voice capture with energy-based hands-free VAD & live amplitude meter.
 */
class VoiceRecorder {
    private val recording = AtomicBoolean(false)
    @Volatile
    var liveLevel: Float = 0f
        private set

    private var worker: Thread? = null

    fun isRecording(): Boolean = recording.get()

    fun stop() {
        recording.set(false)
    }

    @SuppressLint("MissingPermission")
    fun startHandsFree(
        maxDurationMs: Long = 18_000L,
        silenceEndMs: Long = 1_400L,
        onFinished: (ByteArray?) -> Unit
    ) {
        if (!recording.compareAndSet(false, true)) return
        worker = Thread({
            var recorder: AudioRecord? = null
            try {
                val minBuf = AudioRecord.getMinBufferSize(
                    WavEncoder.SAMPLE_RATE,
                    AudioFormat.CHANNEL_IN_MONO,
                    AudioFormat.ENCODING_PCM_16BIT
                ).coerceAtLeast(3200)
                recorder = AudioRecord(
                    MediaRecorder.AudioSource.VOICE_RECOGNITION,
                    WavEncoder.SAMPLE_RATE,
                    AudioFormat.CHANNEL_IN_MONO,
                    AudioFormat.ENCODING_PCM_16BIT,
                    minBuf * 2
                )
                if (recorder.state != AudioRecord.STATE_INITIALIZED) {
                    recording.set(false)
                    onFinished(null)
                    return@Thread
                }
                recorder.startRecording()

                val chunk = ShortArray(480) // 30ms @ 16kHz
                val collected = ArrayList<Short>(WavEncoder.SAMPLE_RATE * 5)
                val startMs = System.currentTimeMillis()
                var speechDetected = false
                var lastSpeechMs = startMs

                while (recording.get()) {
                    val read = recorder.read(chunk, 0, chunk.size)
                    if (read <= 0) continue
                    var sumSq = 0.0
                    for (i in 0 until read) {
                        val s = chunk[i]
                        collected.add(s)
                        val f = s / 32768.0
                        sumSq += f * f
                    }
                    val rms = sqrt(sumSq / read).toFloat()
                    liveLevel = (rms * 7.5f).coerceIn(0f, 1f)

                    val now = System.currentTimeMillis()
                    if (rms > 0.018f) {
                        speechDetected = true
                        lastSpeechMs = now
                    }
                    if (now - startMs >= maxDurationMs) break
                    if (speechDetected && now - lastSpeechMs >= silenceEndMs) break
                    if (!speechDetected && now - startMs >= 6_000L) break
                }

                liveLevel = 0f
                recording.set(false)
                if (collected.size < WavEncoder.SAMPLE_RATE / 4) {
                    onFinished(null)
                } else {
                    val arr = ShortArray(collected.size) { collected[it] }
                    onFinished(WavEncoder.encodePcm16Mono(arr))
                }
            } catch (_: Throwable) {
                liveLevel = 0f
                recording.set(false)
                onFinished(null)
            } finally {
                try {
                    recorder?.stop()
                } catch (_: Throwable) {
                }
                try {
                    recorder?.release()
                } catch (_: Throwable) {
                }
            }
        }, "zekrima-voice-rec").apply { start() }
    }
}
