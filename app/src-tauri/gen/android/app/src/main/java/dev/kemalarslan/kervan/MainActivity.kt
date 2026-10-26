package dev.kemalarslan.kervan

import android.Manifest
import android.content.pm.PackageManager
import android.os.Build
import android.os.Bundle
import android.view.View
import androidx.activity.enableEdgeToEdge
import androidx.core.view.ViewCompat
import androidx.core.view.WindowInsetsCompat

class MainActivity : TauriActivity() {
  override fun onCreate(savedInstanceState: Bundle?) {
    enableEdgeToEdge()
    super.onCreate(savedInstanceState)
    keepClearOfSystemBars(findViewById(android.R.id.content))
    attachScreen()
    askForPermissions()
    KervanService.start(this)
  }

  /** Back on the list keeps the app running behind other apps, as Home does. */
  @Suppress("OVERRIDE_DEPRECATION")
  override fun onBackPressed() {
    moveTaskToBack(true)
  }

  /**
   * Tells the Rust side a screen is here: it gives the screen a window when
   * the process outlived the last one, and learns how to reach the service.
   */
  private external fun attachScreen()

  /**
   * Notifications, for the service and for messages, and Bluetooth, for
   * nearby nodes. The service picks Bluetooth up within seconds of a yes.
   */
  private fun askForPermissions() {
    val wanted = mutableListOf<String>()
    if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
      wanted += Manifest.permission.POST_NOTIFICATIONS
    }
    if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.S) {
      wanted += Manifest.permission.BLUETOOTH_SCAN
      wanted += Manifest.permission.BLUETOOTH_ADVERTISE
      wanted += Manifest.permission.BLUETOOTH_CONNECT
    } else {
      wanted += Manifest.permission.ACCESS_FINE_LOCATION
    }
    val missing = wanted.filter { checkSelfPermission(it) != PackageManager.PERMISSION_GRANTED }
    if (missing.isNotEmpty()) requestPermissions(missing.toTypedArray(), 1)
  }

  /** Pads the page by the status bar, the navigation bar and the keyboard. */
  private fun keepClearOfSystemBars(view: View) {
    ViewCompat.setOnApplyWindowInsetsListener(view) { padded, insets ->
      val bars = insets.getInsets(
        WindowInsetsCompat.Type.systemBars() or
          WindowInsetsCompat.Type.displayCutout() or
          WindowInsetsCompat.Type.ime()
      )
      padded.setPadding(bars.left, bars.top, bars.right, bars.bottom)
      WindowInsetsCompat.CONSUMED
    }
  }
}
