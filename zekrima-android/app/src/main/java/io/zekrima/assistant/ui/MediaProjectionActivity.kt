package io.zekrima.assistant.ui

import android.app.Activity
import android.content.Context
import android.content.Intent
import android.media.projection.MediaProjectionManager
import android.os.Bundle
import androidx.core.content.ContextCompat
import io.zekrima.assistant.ZekrimaApp
import io.zekrima.assistant.services.IslandOverlayService

/**
 * Transparent permission activity to request Android MediaProjection consent
 * for Cicada System Audio Loopback (AudioPlaybackCapture) and Zero-Retention Screen Vision.
 */
class MediaProjectionActivity : Activity() {

    companion object {
        private const val REQ_PROJECTION = 4401
        const val EXTRA_PURPOSE = "purpose"
    }

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        ZekrimaApp.pendingProjectionPurpose = intent.getStringExtra(EXTRA_PURPOSE).orEmpty()
        val mpm = getSystemService(Context.MEDIA_PROJECTION_SERVICE) as MediaProjectionManager
        @Suppress("DEPRECATION")
        startActivityForResult(mpm.createScreenCaptureIntent(), REQ_PROJECTION)
    }

    @Deprecated("Standard Activity result callback")
    override fun onActivityResult(requestCode: Int, resultCode: Int, data: Intent?) {
        super.onActivityResult(requestCode, resultCode, data)
        if (requestCode == REQ_PROJECTION && resultCode == RESULT_OK && data != null) {
            ZekrimaApp.mediaProjectionResultCode = resultCode
            ZekrimaApp.mediaProjectionData = data
            val svc = Intent(this, IslandOverlayService::class.java)
                .setAction(IslandOverlayService.ACTION_PROJECTION_READY)
            ContextCompat.startForegroundService(this, svc)
        }
        finish()
    }
}
