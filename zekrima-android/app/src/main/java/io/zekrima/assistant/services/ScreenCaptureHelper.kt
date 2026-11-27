package io.zekrima.assistant.services

import android.content.Context
import android.graphics.Bitmap
import android.graphics.PixelFormat
import android.hardware.display.DisplayManager
import android.media.ImageReader
import android.media.projection.MediaProjection
import android.os.Handler
import android.os.HandlerThread
import android.util.DisplayMetrics
import android.view.WindowManager
import java.io.ByteArrayOutputStream
import java.util.concurrent.CountDownLatch
import java.util.concurrent.TimeUnit

/**
 * Zero-Retention Screen Vision capture via MediaProjection -> in-memory JPEG bytes.
 * Images are NEVER written to disk and are recycled immediately after compression.
 */
object ScreenCaptureHelper {

    fun captureJpeg(
        context: Context,
        projection: MediaProjection?,
        highRes: Boolean
    ): ByteArray? {
        if (projection == null) return null
        val wm = context.getSystemService(Context.WINDOW_SERVICE) as? WindowManager ?: return null
        val dm = DisplayMetrics()
        @Suppress("DEPRECATION")
        wm.defaultDisplay?.getRealMetrics(dm)
        val rawW = dm.widthPixels.coerceAtLeast(360)
        val rawH = dm.heightPixels.coerceAtLeast(640)
        val density = dm.densityDpi.coerceAtLeast(160)

        val maxSide = if (highRes) 1600 else 1024
        val scale = (maxSide.toFloat() / maxOf(rawW, rawH).toFloat()).coerceAtMost(1f)
        val targetW = (rawW * scale).toInt().coerceAtLeast(240)
        val targetH = (rawH * scale).toInt().coerceAtLeast(240)

        val thread = HandlerThread("zekrima-screen-shot").apply { start() }
        val handler = Handler(thread.looper)
        val reader = ImageReader.newInstance(targetW, targetH, PixelFormat.RGBA_8888, 2)
        val latch = CountDownLatch(1)
        var jpegBytes: ByteArray? = null

        val vDisplay = try {
            projection.createVirtualDisplay(
                "ZekrimaVision",
                targetW,
                targetH,
                density,
                DisplayManager.VIRTUAL_DISPLAY_FLAG_AUTO_MIRROR,
                reader.surface,
                null,
                handler
            )
        } catch (_: Throwable) {
            reader.close()
            thread.quitSafely()
            return null
        }

        reader.setOnImageAvailableListener({ r ->
            val img = try {
                r.acquireLatestImage()
            } catch (_: Throwable) {
                null
            } ?: return@setOnImageAvailableListener
            try {
                val plane = img.planes[0]
                val buffer = plane.buffer
                val pixelStride = plane.pixelStride
                val rowStride = plane.rowStride
                val rowPadding = rowStride - pixelStride * targetW
                val paddedW = targetW + rowPadding / pixelStride
                val bmp = Bitmap.createBitmap(paddedW, targetH, Bitmap.Config.ARGB_8888)
                bmp.copyPixelsFromBuffer(buffer)
                val cropped = if (paddedW != targetW) {
                    Bitmap.createBitmap(bmp, 0, 0, targetW, targetH).also { bmp.recycle() }
                } else {
                    bmp
                }
                val baos = ByteArrayOutputStream()
                cropped.compress(Bitmap.CompressFormat.JPEG, if (highRes) 82 else 74, baos)
                cropped.recycle()
                jpegBytes = baos.toByteArray()
            } catch (_: Throwable) {
            } finally {
                img.close()
                latch.countDown()
            }
        }, handler)

        latch.await(1500, TimeUnit.MILLISECONDS)
        try {
            vDisplay?.release()
        } catch (_: Throwable) {
        }
        try {
            reader.close()
        } catch (_: Throwable) {
        }
        thread.quitSafely()
        return jpegBytes
    }
}
