package dev.kemalarslan.kervan

import android.content.Context
import android.net.wifi.WifiManager
import android.os.Bundle
import android.view.View
import androidx.activity.enableEdgeToEdge
import androidx.core.view.ViewCompat
import androidx.core.view.WindowInsetsCompat

class MainActivity : TauriActivity() {
  private var multicastLock: WifiManager.MulticastLock? = null

  override fun onCreate(savedInstanceState: Bundle?) {
    enableEdgeToEdge()
    super.onCreate(savedInstanceState)
    keepClearOfSystemBars(findViewById(android.R.id.content))
    // Android drops broadcast and multicast packets unless an app holds this
    // lock, and those packets are how devices on the network find each other.
    val wifi = applicationContext.getSystemService(Context.WIFI_SERVICE) as WifiManager
    multicastLock = wifi.createMulticastLock("kervan-lan").apply {
      setReferenceCounted(false)
      acquire()
    }
  }

  override fun onDestroy() {
    multicastLock?.release()
    multicastLock = null
    super.onDestroy()
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
