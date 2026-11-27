package io.zekrima.assistant.services

import android.content.Intent
import android.provider.Settings
import android.service.quicksettings.Tile
import android.service.quicksettings.TileService
import androidx.core.content.ContextCompat
import io.zekrima.assistant.ZekrimaApp
import io.zekrima.assistant.ui.MainActivity

class ZekrimaQuickTileService : TileService() {

    override fun onStartListening() {
        super.onStartListening()
        syncTile()
    }

    override fun onClick() {
        super.onClick()
        if (!Settings.canDrawOverlays(this)) {
            val intent = Intent(this, MainActivity::class.java).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
            startActivity(intent)
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
        syncTile()
    }

    private fun syncTile() {
        val tile = qsTile ?: return
        tile.state = if (ZekrimaApp.overlayRunning) Tile.STATE_ACTIVE else Tile.STATE_INACTIVE
        tile.label = "Zekrima"
        tile.subtitle = if (ZekrimaApp.overlayRunning) "Aktif" else "Mati"
        tile.updateTile()
    }
}
