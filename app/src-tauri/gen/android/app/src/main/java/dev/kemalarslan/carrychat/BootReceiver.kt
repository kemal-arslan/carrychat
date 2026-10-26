package dev.kemalarslan.carrychat

import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent

/**
 * Starts CarryChat in the background when the phone has started, or the app
 * was updated, so messages keep arriving without opening it first. A user
 * who turned CarryChat off keeps it off until they open the app.
 */
class BootReceiver : BroadcastReceiver() {
  override fun onReceive(context: Context, intent: Intent) {
    if (intent.action != Intent.ACTION_BOOT_COMPLETED &&
      intent.action != Intent.ACTION_MY_PACKAGE_REPLACED
    ) {
      return
    }
    if (CarryChatService.turnedOff(context)) return
    CarryChatService.start(context)
  }
}
