package dev.kemalarslan.kervan

import android.Manifest
import android.content.pm.PackageManager
import android.graphics.drawable.ColorDrawable
import android.os.Build
import android.os.Bundle
import android.view.View
import android.webkit.JavascriptInterface
import android.webkit.WebView
import androidx.activity.enableEdgeToEdge
import androidx.core.view.ViewCompat
import androidx.core.view.WindowInsetsCompat
import androidx.core.view.WindowInsetsControllerCompat

class MainActivity : TauriActivity() {
  override fun onCreate(savedInstanceState: Bundle?) {
    enableEdgeToEdge()
    super.onCreate(savedInstanceState)
    // The theme the page last chose, until the page loads and says again.
    val prefs = getSharedPreferences(PREFS, MODE_PRIVATE)
    if (prefs.contains(DARK)) paintBars(prefs.getBoolean(DARK, false))
    keepClearOfSystemBars(findViewById(android.R.id.content))
    attachScreen()
    askForPermissions()
    KervanService.start(this)
  }

  override fun onWebViewCreate(webView: WebView) {
    webView.addJavascriptInterface(Bars(), "KervanAndroid")
  }

  /** Lets the page give the status and navigation bars its light or dark theme. */
  inner class Bars {
    @JavascriptInterface
    fun setDark(dark: Boolean) {
      getSharedPreferences(PREFS, MODE_PRIVATE).edit().putBoolean(DARK, dark).apply()
      runOnUiThread { paintBars(dark) }
    }
  }

  private fun paintBars(dark: Boolean) {
    window.setBackgroundDrawable(
      ColorDrawable(getColor(if (dark) R.color.kervan_bars_dark else R.color.kervan_bars_light))
    )
    WindowInsetsControllerCompat(window, window.decorView).apply {
      isAppearanceLightStatusBars = !dark
      isAppearanceLightNavigationBars = !dark
    }
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

  private companion object {
    const val PREFS = "kervan"
    const val DARK = "dark_bars"
  }
}
