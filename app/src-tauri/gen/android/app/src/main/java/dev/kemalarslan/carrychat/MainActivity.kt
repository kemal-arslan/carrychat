package dev.kemalarslan.carrychat

import android.Manifest
import android.content.pm.PackageManager
import android.graphics.drawable.ColorDrawable
import android.nfc.NfcAdapter
import android.nfc.Tag
import android.os.Build
import android.os.Bundle
import android.view.View
import android.webkit.JavascriptInterface
import android.webkit.WebView
import androidx.activity.enableEdgeToEdge
import androidx.core.view.ViewCompat
import androidx.core.view.WindowInsetsCompat
import androidx.core.view.WindowInsetsControllerCompat
import org.json.JSONObject

class MainActivity : TauriActivity() {
  private var webView: WebView? = null
  /** Whether the page wants cards read by NFC, as while adding a contact. */
  private var readingCards = false

  override fun onCreate(savedInstanceState: Bundle?) {
    enableEdgeToEdge()
    // The node opens as the screen is made: its data key goes first.
    KeyVault.provide(this)
    super.onCreate(savedInstanceState)
    // The theme the page last chose, until the page loads and says again.
    val prefs = getSharedPreferences(PREFS, MODE_PRIVATE)
    if (prefs.contains(DARK)) paintBars(prefs.getBoolean(DARK, false))
    keepClearOfSystemBars(findViewById(android.R.id.content))
    attachScreen()
    askForPermissions()
    CarryChatService.setTurnedOff(this, false)
    CarryChatService.start(this, fromScreen = true)
  }

  override fun onWebViewCreate(webView: WebView) {
    this.webView = webView
    webView.addJavascriptInterface(Page(), "CarryChatAndroid")
  }

  // Android lets only the activity in front read NFC.
  override fun onResume() {
    super.onResume()
    if (readingCards) startReadingCards()
  }

  override fun onPause() {
    super.onPause()
    NfcAdapter.getDefaultAdapter(this)?.disableReaderMode(this)
  }

  /** What the page asks of the activity. */
  inner class Page {
    /** Gives the status and navigation bars the page's light or dark theme. */
    @JavascriptInterface
    fun setDark(dark: Boolean) {
      getSharedPreferences(PREFS, MODE_PRIVATE).edit().putBoolean(DARK, dark).apply()
      runOnUiThread { paintBars(dark) }
    }

    /** The back button had nothing left to close on the page. */
    @JavascriptInterface
    fun leave() {
      runOnUiThread { moveTaskToBack(true) }
    }

    /** Whether this phone can pass cards over NFC (§3.3), with NFC turned on. */
    @JavascriptInterface
    fun nfcAvailable(): Boolean =
      NfcAdapter.getDefaultAdapter(this@MainActivity)?.isEnabled == true &&
        packageManager.hasSystemFeature(PackageManager.FEATURE_NFC_HOST_CARD_EMULATION)

    /** Offers this card to a phone that reads it over NFC, or, given null, nothing. */
    @JavascriptInterface
    fun nfcOffer(card: String?) {
      CardService.offer(card)
    }

    /** Reads cards over NFC while `on`; each is sent to the page as a `carrychat-nfc` event. */
    @JavascriptInterface
    fun nfcRead(on: Boolean) {
      runOnUiThread {
        readingCards = on
        if (on) startReadingCards() else NfcAdapter.getDefaultAdapter(this@MainActivity)?.disableReaderMode(this@MainActivity)
      }
    }
  }

  private fun startReadingCards() {
    val adapter = NfcAdapter.getDefaultAdapter(this) ?: return
    val flags = NfcAdapter.FLAG_READER_NFC_A or NfcAdapter.FLAG_READER_NFC_B or NfcAdapter.FLAG_READER_SKIP_NDEF_CHECK
    adapter.enableReaderMode(this, ::cardTag, flags, null)
  }

  /** A phone was held to this one: hands the card it offers to the page. */
  private fun cardTag(tag: Tag) {
    val card = CardService.read(tag) ?: return
    runOnUiThread {
      val detail = JSONObject.quote(card)
      webView?.evaluateJavascript("window.dispatchEvent(new CustomEvent('carrychat-nfc', { detail: $detail }))", null)
    }
  }

  private fun paintBars(dark: Boolean) {
    window.setBackgroundDrawable(
      ColorDrawable(getColor(if (dark) R.color.carrychat_bars_dark else R.color.carrychat_bars_light))
    )
    WindowInsetsControllerCompat(window, window.decorView).apply {
      isAppearanceLightStatusBars = !dark
      isAppearanceLightNavigationBars = !dark
    }
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
    }
    // Wi-Fi Aware finds phones nearby with its own permission from Android 13,
    // and with the location permission before that, as Bluetooth did before 12.
    if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
      wanted += Manifest.permission.NEARBY_WIFI_DEVICES
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
    const val PREFS = "carrychat"
    const val DARK = "dark_bars"
  }
}
