package dev.kemalarslan.carrychat

import android.Manifest
import android.annotation.SuppressLint
import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.content.IntentFilter
import android.content.pm.PackageManager
import android.net.ConnectivityManager
import android.net.Network
import android.net.NetworkCapabilities
import android.net.NetworkRequest
import android.net.wifi.aware.AttachCallback
import android.net.wifi.aware.DiscoverySessionCallback
import android.net.wifi.aware.PeerHandle
import android.net.wifi.aware.PublishConfig
import android.net.wifi.aware.PublishDiscoverySession
import android.net.wifi.aware.SubscribeConfig
import android.net.wifi.aware.SubscribeDiscoverySession
import android.net.wifi.aware.WifiAwareManager
import android.net.wifi.aware.WifiAwareNetworkInfo
import android.net.wifi.aware.WifiAwareNetworkSpecifier
import android.net.wifi.aware.WifiAwareSession
import android.os.Build
import android.os.Handler
import android.os.HandlerThread
import android.util.Log
import androidx.annotation.RequiresApi
import java.io.OutputStream
import java.net.ServerSocket
import java.net.Socket
import java.util.concurrent.ConcurrentHashMap
import java.util.concurrent.ExecutorService
import java.util.concurrent.Executors
import java.util.concurrent.atomic.AtomicInteger

/** The Rust side of the Wi-Fi Aware link (`src-tauri/src/android.rs`). */
object Aware {
  init {
    System.loadLibrary("carrychat_app_lib")
  }

  /** The session id to publish, or null while there is no node. */
  @JvmStatic external fun session(): ByteArray?
  @JvmStatic external fun shouldConnect(peer: ByteArray): Boolean
  @JvmStatic external fun dialEnded(peer: ByteArray)
  /** A connection is ready; `peer` is the node this side connected to, if it did. */
  @JvmStatic external fun opened(id: Int, peer: ByteArray?)
  @JvmStatic external fun received(id: Int, data: ByteArray)
  @JvmStatic external fun closed(id: Int)
  /** Tells the app whether Wi-Fi Aware is available: 1 off, 2 on. */
  @JvmStatic external fun state(state: Int)

  @Volatile internal var link: WifiAwareLink? = null

  /** Called by the Rust side: bytes of a connection's stream. */
  @JvmStatic fun send(id: Int, data: ByteArray) {
    link?.send(id, data)
  }

  /** Called by the Rust side when a connection's sync ends. */
  @JvmStatic fun close(id: Int) {
    link?.close(id)
  }
}

/**
 * The Wi-Fi Aware link (§9.6): phones running CarryChat find each other and
 * open a Wi-Fi connection directly, with no access point and no internet,
 * farther and much faster than Bluetooth. Each phone publishes the CarryChat
 * service with its session id and subscribes to it. Of two phones that find
 * each other, the one the node picks connects: it tells the other with a
 * discovery message, both ask Android for an Aware data path, the other
 * listens on a port and this one connects to it over TCP. The connection
 * carries the node's sync stream, which the Rust side runs.
 *
 * Discovery and data-path requests run on one thread; sockets block on
 * threads of their own.
 */
@SuppressLint("MissingPermission")
@RequiresApi(Build.VERSION_CODES.Q)
class WifiAwareLink(private val context: Context) {
  private val thread = HandlerThread("carrychat-aware").apply { start() }
  private val handler = Handler(thread.looper)
  private val sockets: ExecutorService = Executors.newCachedThreadPool()
  private val manager = context.getSystemService(WifiAwareManager::class.java)
  private val connectivity = context.getSystemService(ConnectivityManager::class.java)!!

  private var session: WifiAwareSession? = null
  private var publish: PublishDiscoverySession? = null
  private var subscribe: SubscribeDiscoverySession? = null
  private var attaching = false
  private var stopped = false

  /** Peers this side is connecting to, by session id in hex. */
  private val dialing = mutableSetOf<String>()
  private val connections = ConcurrentHashMap<Int, Connection>()
  private val nextId = AtomicInteger(1)

  private class Connection(
    val socket: Socket,
    val output: OutputStream,
    val writer: ExecutorService,
    val network: ConnectivityManager.NetworkCallback,
    val peer: ByteArray?,
  )

  private val stateChanged = object : BroadcastReceiver() {
    override fun onReceive(context: Context, intent: Intent) {
      handler.post {
        val available = manager?.isAvailable == true
        Aware.state(if (available) 2 else 1)
        if (available) attach() else drop()
      }
    }
  }

  fun start() {
    if (!supported(context)) return
    Aware.link = this
    Aware.state(if (manager?.isAvailable == true) 2 else 1)
    context.registerReceiver(stateChanged, IntentFilter(WifiAwareManager.ACTION_WIFI_AWARE_STATE_CHANGED))
    handler.post(::tick)
  }

  fun stop() {
    stopped = true
    Aware.link = null
    handler.removeCallbacksAndMessages(null)
    try {
      context.unregisterReceiver(stateChanged)
    } catch (_: IllegalArgumentException) {
    }
    handler.post {
      drop()
      thread.quitSafely()
    }
  }

  fun send(id: Int, data: ByteArray) {
    val connection = connections[id] ?: return
    connection.writer.execute {
      try {
        connection.output.write(data)
        connection.output.flush()
      } catch (error: Exception) {
        close(id)
      }
    }
  }

  fun close(id: Int) {
    val connection = connections.remove(id) ?: return
    try {
      connection.socket.close()
    } catch (_: Exception) {
    }
    connection.writer.shutdown()
    release(connection.network)
    connection.peer?.let { peer -> handler.post { dialing.remove(hex(peer)) } }
  }

  /** Attaches when Aware, the permission and a node are all there, and checks again later. */
  private fun tick() {
    if (stopped) return
    attach()
    handler.postDelayed(::tick, TICK_MS)
  }

  private fun attach() {
    val aware = manager ?: return
    if (session != null || attaching || !aware.isAvailable || !hasPermission()) return
    val own = Aware.session() ?: return
    attaching = true
    aware.attach(object : AttachCallback() {
      override fun onAttached(attached: WifiAwareSession) {
        attaching = false
        session = attached
        attached.publish(
          PublishConfig.Builder().setServiceName(SERVICE).setServiceSpecificInfo(own).build(),
          publishCallback,
          handler,
        )
        attached.subscribe(SubscribeConfig.Builder().setServiceName(SERVICE).build(), subscribeCallback, handler)
      }

      override fun onAttachFailed() {
        attaching = false
        Log.w(TAG, "Wi-Fi Aware did not attach")
      }
    }, handler)
  }

  /** Ends discovery and every connection, as when Wi-Fi Aware goes away. */
  private fun drop() {
    connections.keys.toList().forEach { id ->
      close(id)
      Aware.closed(id)
    }
    publish?.close()
    subscribe?.close()
    session?.close()
    publish = null
    subscribe = null
    session = null
    dialing.clear()
  }

  private val publishCallback = object : DiscoverySessionCallback() {
    override fun onPublishStarted(started: PublishDiscoverySession) {
      publish = started
    }

    /** A subscriber chose to connect: listen for it on a data path of its own. */
    override fun onMessageReceived(peerHandle: PeerHandle, message: ByteArray) {
      val published = publish ?: return
      val server = try {
        ServerSocket(0).apply { soTimeout = ACCEPT_TIMEOUT_MS }
      } catch (error: Exception) {
        return
      }
      val specifier = WifiAwareNetworkSpecifier.Builder(published, peerHandle)
        .setPskPassphrase(PASSPHRASE)
        .setPort(server.localPort)
        .build()
      val callback = object : ConnectivityManager.NetworkCallback() {
        override fun onLost(network: Network) {
          try {
            server.close()
          } catch (_: Exception) {
          }
        }
      }
      request(specifier, callback)
      sockets.execute {
        try {
          val socket = server.accept()
          connected(socket, null, callback)
        } catch (error: Exception) {
          release(callback)
        } finally {
          try {
            server.close()
          } catch (_: Exception) {
          }
        }
      }
    }
  }

  private val subscribeCallback = object : DiscoverySessionCallback() {
    override fun onSubscribeStarted(started: SubscribeDiscoverySession) {
      subscribe = started
    }

    override fun onServiceDiscovered(peerHandle: PeerHandle, info: ByteArray, filter: List<ByteArray>) {
      if (info.size == 8) dial(peerHandle, info)
    }
  }

  /** Connects to a publisher found nearby, if the node chooses to. */
  private fun dial(peerHandle: PeerHandle, peer: ByteArray) {
    val subscribed = subscribe ?: return
    val own = Aware.session() ?: return
    if (hex(peer) in dialing || !Aware.shouldConnect(peer)) return
    dialing.add(hex(peer))
    subscribed.sendMessage(peerHandle, MESSAGE_ID, own)
    val specifier = WifiAwareNetworkSpecifier.Builder(subscribed, peerHandle)
      .setPskPassphrase(PASSPHRASE)
      .build()
    val callback = object : ConnectivityManager.NetworkCallback() {
      private var started = false

      override fun onCapabilitiesChanged(network: Network, capabilities: NetworkCapabilities) {
        val info = capabilities.transportInfo as? WifiAwareNetworkInfo ?: return
        val address = info.peerIpv6Addr ?: return
        if (started || info.port <= 0) return
        started = true
        val callback = this
        sockets.execute {
          try {
            val socket = network.socketFactory.createSocket(address, info.port)
            connected(socket, peer, callback)
          } catch (error: Exception) {
            Log.w(TAG, "Could not connect over Wi-Fi Aware: $error")
            failed(peer, callback)
          }
        }
      }

      override fun onUnavailable() {
        failed(peer, this)
      }

      override fun onLost(network: Network) {
        if (!started) failed(peer, this)
      }
    }
    request(specifier, callback)
  }

  private fun failed(peer: ByteArray, callback: ConnectivityManager.NetworkCallback) {
    release(callback)
    handler.post { dialing.remove(hex(peer)) }
    Aware.dialEnded(peer)
  }

  /** Hands a connected socket's stream to the node, and its bytes as they come. */
  private fun connected(socket: Socket, peer: ByteArray?, callback: ConnectivityManager.NetworkCallback) {
    socket.tcpNoDelay = true
    val id = nextId.getAndIncrement()
    connections[id] = Connection(socket, socket.getOutputStream(), Executors.newSingleThreadExecutor(), callback, peer)
    Aware.opened(id, peer)
    val buffer = ByteArray(READ_LEN)
    try {
      val input = socket.getInputStream()
      while (true) {
        val read = input.read(buffer)
        if (read < 0) break
        if (read > 0) Aware.received(id, buffer.copyOf(read))
      }
    } catch (_: Exception) {
    }
    if (connections.containsKey(id)) {
      close(id)
      Aware.closed(id)
    }
  }

  private fun request(specifier: WifiAwareNetworkSpecifier, callback: ConnectivityManager.NetworkCallback) {
    val request = NetworkRequest.Builder()
      .addTransportType(NetworkCapabilities.TRANSPORT_WIFI_AWARE)
      .setNetworkSpecifier(specifier)
      .build()
    connectivity.requestNetwork(request, callback, handler, DATA_PATH_TIMEOUT_MS)
  }

  private fun release(callback: ConnectivityManager.NetworkCallback) {
    try {
      connectivity.unregisterNetworkCallback(callback)
    } catch (_: IllegalArgumentException) {
    }
  }

  private fun hasPermission(): Boolean {
    val permission = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
      Manifest.permission.NEARBY_WIFI_DEVICES
    } else {
      Manifest.permission.ACCESS_FINE_LOCATION
    }
    return context.checkSelfPermission(permission) == PackageManager.PERMISSION_GRANTED
  }

  companion object {
    private const val TAG = "CarryChatAware"
    private const val SERVICE = "carrychat"
    /** Wi-Fi Aware encrypts the link with it; messages are sealed end to end anyway. */
    private const val PASSPHRASE = "carrychat-wifi-aware"
    private const val MESSAGE_ID = 1
    private const val TICK_MS = 15_000L
    private const val DATA_PATH_TIMEOUT_MS = 30_000
    private const val ACCEPT_TIMEOUT_MS = 30_000
    private const val READ_LEN = 4096

    /** Whether this phone can use Wi-Fi Aware at all. */
    fun supported(context: Context): Boolean =
      Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q &&
        context.packageManager.hasSystemFeature(PackageManager.FEATURE_WIFI_AWARE)

    private fun hex(bytes: ByteArray) = bytes.joinToString("") { "%02x".format(it) }
  }
}
