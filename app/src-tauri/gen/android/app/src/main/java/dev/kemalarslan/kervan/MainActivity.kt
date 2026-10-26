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
    askToNotify()
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

  /** The service's notification, and message notifications, need this. */
  private fun askToNotify() {
    if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU &&
      checkSelfPermission(Manifest.permission.POST_NOTIFICATIONS) != PackageManager.PERMISSION_GRANTED
    ) {
      requestPermissions(arrayOf(Manifest.permission.POST_NOTIFICATIONS), 1)
    }
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
