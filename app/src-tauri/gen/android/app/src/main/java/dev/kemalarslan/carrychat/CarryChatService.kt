package dev.kemalarslan.carrychat

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.app.Service
import android.content.Context
import android.content.Intent
import android.content.pm.ServiceInfo
import android.net.wifi.WifiManager
import android.os.Build
import android.os.IBinder
import android.os.Process
import java.util.concurrent.atomic.AtomicInteger
import androidx.core.app.NotificationCompat
import androidx.core.app.ServiceCompat
import androidx.core.content.ContextCompat

/**
 * Keeps the process running while no screen is open. The node lives in the
 * process, so this is what lets messages arrive, and be passed on for
 * others, while the app is closed.
 */
class CarryChatService : Service() {
  private var multicastLock: WifiManager.MulticastLock? = null
  private var bluetooth: BleLink? = null
  private var wifiAware: WifiAwareLink? = null
  private var lora: LoraLink? = null

  override fun onCreate() {
    super.onCreate()
    appContext = applicationContext
    // Android drops broadcast and multicast packets unless an app holds this
    // lock, and those packets are how devices on the network find each other.
    val wifi = applicationContext.getSystemService(Context.WIFI_SERVICE) as WifiManager
    multicastLock = wifi.createMulticastLock("carrychat-lan").apply {
      setReferenceCounted(false)
      acquire()
    }
    bluetooth = BleLink(this).also { it.start() }
    if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q && WifiAwareLink.supported(this)) {
      wifiAware = WifiAwareLink(this).also { it.start() }
    }
    lora = LoraLink(this).also { it.start() }
  }

  override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
    if (intent?.action == ACTION_STOP) {
      // Turning CarryChat off ends the process, and the node in it, and keeps
      // it off after the phone restarts, until the app is opened again.
      setTurnedOff(this, true)
      ServiceCompat.stopForeground(this, ServiceCompat.STOP_FOREGROUND_REMOVE)
      stopSelf()
      Process.killProcess(Process.myPid())
      return START_NOT_STICKY
    }
    if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.UPSIDE_DOWN_CAKE) {
      try {
        ServiceCompat.startForeground(
          this,
          NOTIFICATION_ID,
          notification(),
          ServiceInfo.FOREGROUND_SERVICE_TYPE_REMOTE_MESSAGING or
            ServiceInfo.FOREGROUND_SERVICE_TYPE_CONNECTED_DEVICE,
        )
      } catch (error: SecurityException) {
        // Connected-device use needs a permission Android may not count as
        // granted yet; messaging alone still keeps the process running.
        ServiceCompat.startForeground(
          this,
          NOTIFICATION_ID,
          notification(),
          ServiceInfo.FOREGROUND_SERVICE_TYPE_REMOTE_MESSAGING,
        )
      }
    } else {
      ServiceCompat.startForeground(this, NOTIFICATION_ID, notification(), 0)
    }
    // The node starts here when no screen started it: after the phone
    // starts, or when Android restarts the service. Before there is an
    // identity there is nothing to run, unless a screen is open to make one.
    val running = Core.start(applicationContext.dataDir.absolutePath)
    if (!running && intent?.getBooleanExtra(EXTRA_FROM_SCREEN, false) != true) {
      ServiceCompat.stopForeground(this, ServiceCompat.STOP_FOREGROUND_REMOVE)
      stopSelf()
      return START_NOT_STICKY
    }
    return START_STICKY
  }

  override fun onDestroy() {
    bluetooth?.stop()
    bluetooth = null
    if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) wifiAware?.stop()
    wifiAware = null
    lora?.stop()
    lora = null
    multicastLock?.release()
    multicastLock = null
    super.onDestroy()
  }

  override fun onBind(intent: Intent?): IBinder? = null

  private fun notification(): Notification {
    if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
      val channel = NotificationChannel(
        CHANNEL_ID,
        getString(R.string.background_channel),
        NotificationManager.IMPORTANCE_LOW,
      )
      getSystemService(NotificationManager::class.java).createNotificationChannel(channel)
    }
    val open = PendingIntent.getActivity(
      this,
      0,
      Intent(this, MainActivity::class.java).addFlags(Intent.FLAG_ACTIVITY_SINGLE_TOP),
      PendingIntent.FLAG_IMMUTABLE,
    )
    val stop = PendingIntent.getService(
      this,
      1,
      Intent(this, CarryChatService::class.java).setAction(ACTION_STOP),
      PendingIntent.FLAG_IMMUTABLE,
    )
    return NotificationCompat.Builder(this, CHANNEL_ID)
      .setSmallIcon(R.drawable.ic_stat_carrychat)
      .setContentTitle(getString(R.string.background_title))
      .setContentText(getString(R.string.background_text))
      .setContentIntent(open)
      .addAction(0, getString(R.string.background_stop), stop)
      .setOngoing(true)
      .setShowWhen(false)
      .setCategory(NotificationCompat.CATEGORY_SERVICE)
      .setForegroundServiceBehavior(NotificationCompat.FOREGROUND_SERVICE_IMMEDIATE)
      .build()
  }

  companion object {
    private const val CHANNEL_ID = "carrychat-background"
    private const val MESSAGES_CHANNEL_ID = "carrychat-messages"
    private const val NOTIFICATION_ID = 1
    private const val ACTION_STOP = "dev.kemalarslan.carrychat.STOP"

    private var appContext: Context? = null
    private val nextMessageId = AtomicInteger(100)

    /**
     * Announces a message that arrived while no screen is open. The Rust
     * side calls this; with a screen open the page announces messages itself.
     */
    @JvmStatic
    fun notifyMessage(title: String, body: String) {
      val context = appContext ?: return
      val manager = context.getSystemService(NotificationManager::class.java)
      if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
        val channel = NotificationChannel(
          MESSAGES_CHANNEL_ID,
          context.getString(R.string.messages_channel),
          NotificationManager.IMPORTANCE_HIGH,
        )
        manager.createNotificationChannel(channel)
      }
      val open = PendingIntent.getActivity(
        context,
        0,
        Intent(context, MainActivity::class.java),
        PendingIntent.FLAG_IMMUTABLE,
      )
      val notification = NotificationCompat.Builder(context, MESSAGES_CHANNEL_ID)
        .setSmallIcon(R.drawable.ic_stat_carrychat)
        .setContentTitle(title)
        .setContentText(body)
        .setContentIntent(open)
        .setAutoCancel(true)
        .setCategory(NotificationCompat.CATEGORY_MESSAGE)
        .build()
      manager.notify(nextMessageId.getAndIncrement(), notification)
    }

    private const val EXTRA_FROM_SCREEN = "dev.kemalarslan.carrychat.FROM_SCREEN"

    fun start(context: Context, fromScreen: Boolean = false) {
      val intent = Intent(context, CarryChatService::class.java).putExtra(EXTRA_FROM_SCREEN, fromScreen)
      ContextCompat.startForegroundService(context, intent)
    }

    private const val PREFS = "carrychat"
    private const val TURNED_OFF = "turned_off"

    /** Whether the user turned CarryChat off from its notification. */
    fun turnedOff(context: Context): Boolean =
      context.getSharedPreferences(PREFS, Context.MODE_PRIVATE).getBoolean(TURNED_OFF, false)

    fun setTurnedOff(context: Context, off: Boolean) {
      context.getSharedPreferences(PREFS, Context.MODE_PRIVATE).edit().putBoolean(TURNED_OFF, off).apply()
    }
  }
}

/** The Rust side, which runs the node. */
object Core {
  init {
    System.loadLibrary("carrychat_app_lib")
  }

  /** Opens the node in [dir] and starts its links, unless they already run. */
  @JvmStatic external fun start(dir: String): Boolean
}
