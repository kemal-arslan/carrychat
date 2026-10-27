package dev.kemalarslan.carrychat

import android.Manifest
import android.annotation.SuppressLint
import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.content.IntentFilter
import android.content.pm.PackageManager
import android.net.MacAddress
import android.net.wifi.p2p.WifiP2pConfig
import android.net.wifi.p2p.WifiP2pDevice
import android.net.wifi.p2p.WifiP2pInfo
import android.net.wifi.p2p.WifiP2pManager
import android.net.wifi.p2p.nsd.WifiP2pDnsSdServiceInfo
import android.net.wifi.p2p.nsd.WifiP2pDnsSdServiceRequest
import android.os.Build
import android.os.Handler
import android.os.HandlerThread
import android.os.SystemClock
import android.util.Log
import androidx.annotation.RequiresApi
import java.io.OutputStream
import java.net.InetSocketAddress
import java.net.ServerSocket
import java.net.Socket
import java.util.concurrent.ConcurrentHashMap
import java.util.concurrent.ExecutorService
import java.util.concurrent.Executors
import java.util.concurrent.atomic.AtomicInteger

/** The Rust side of the Wi-Fi Direct link (`src-tauri/src/android.rs`). */
object Direct {
  init {
    System.loadLibrary("carrychat_app_lib")
  }

  /** The session id to announce, or null while there is no node. */
  @JvmStatic external fun session(): ByteArray?
  @JvmStatic external fun shouldConnect(peer: ByteArray): Boolean
  @JvmStatic external fun dialEnded(peer: ByteArray)
  /** A connection is ready; `peer` is the node this side connected to, if it did. */
  @JvmStatic external fun opened(id: Int, peer: ByteArray?)
  @JvmStatic external fun received(id: Int, data: ByteArray)
  @JvmStatic external fun closed(id: Int)
  /** Tells the app whether Wi-Fi Direct is available: 1 off, 2 on. */
  @JvmStatic external fun state(state: Int)

  @Volatile internal var link: WifiDirectLink? = null

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
 * The Wi-Fi Direct link (§9.8): phones running CarryChat that do not support
 * Wi-Fi Aware still connect over Wi-Fi directly, with no access point and no
 * internet. Each phone announces the CarryChat service over Wi-Fi Direct
 * service discovery with its session id, and whether it hosts a group.
 *
 * Joining a group normally asks the user of the phone that hosts it. Here the
 * host makes an autonomous group whose name and passphrase both sides derive
 * from its session id, and the other phone joins it with them, so nobody is
 * asked. The phone with the higher session id hosts, since the other one is
 * the one the node lets connect first; the joining phone then opens a TCP
 * connection to the host, which carries the node's sync stream.
 *
 * Discovery and group changes run on one thread; sockets block on threads of
 * their own.
 */
@SuppressLint("MissingPermission")
@RequiresApi(Build.VERSION_CODES.Q)
class WifiDirectLink(private val context: Context) {
  private val thread = HandlerThread("carrychat-direct").apply { start() }
  private val handler = Handler(thread.looper)
  private val sockets: ExecutorService = Executors.newCachedThreadPool()
  private val manager = context.getSystemService(WifiP2pManager::class.java)
  private var channel: WifiP2pManager.Channel? = null

  private var enabled = false
  private var stopped = false
  /** The session id announced, in hex, and whether it was announced as hosting. */
  private var announced: Pair<String, Boolean>? = null
  private var requested = false

  /** This phone's own group, while it hosts one, and its listening socket. */
  private var hosting = false
  private var server: ServerSocket? = null
  private var hostedAt = 0L
  /** Hosting found no one to join; it waits until then to host again. */
  private var restUntil = 0L

  /** The host this phone is joining or has joined, by session id. */
  private var joining: ByteArray? = null
  private var joinedAt = 0L

  private val connections = ConcurrentHashMap<Int, Connection>()
  private val nextId = AtomicInteger(1)

  private class Connection(
    val socket: Socket,
    val output: OutputStream,
    val writer: ExecutorService,
    val peer: ByteArray?,
  )

  private val changed = object : BroadcastReceiver() {
    override fun onReceive(context: Context, intent: Intent) {
      when (intent.action) {
        WifiP2pManager.WIFI_P2P_STATE_CHANGED_ACTION -> {
          val state = intent.getIntExtra(WifiP2pManager.EXTRA_WIFI_STATE, WifiP2pManager.WIFI_P2P_STATE_DISABLED)
          handler.post {
            enabled = state == WifiP2pManager.WIFI_P2P_STATE_ENABLED
            Direct.state(if (enabled) 2 else 1)
            if (!enabled) drop()
          }
        }
        WifiP2pManager.WIFI_P2P_CONNECTION_CHANGED_ACTION -> handler.post(::groupChanged)
      }
    }
  }

  fun start() {
    if (!supported(context)) return
    val manager = manager ?: return
    Direct.link = this
    channel = manager.initialize(context, thread.looper) { handler.post { channel = null } }
    val filter = IntentFilter().apply {
      addAction(WifiP2pManager.WIFI_P2P_STATE_CHANGED_ACTION)
      addAction(WifiP2pManager.WIFI_P2P_CONNECTION_CHANGED_ACTION)
    }
    context.registerReceiver(changed, filter)
    handler.post(::tick)
  }

  fun stop() {
    stopped = true
    Direct.link = null
    handler.removeCallbacksAndMessages(null)
    try {
      context.unregisterReceiver(changed)
    } catch (_: IllegalArgumentException) {
    }
    handler.post {
      drop()
      channel?.close()
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
    // A joining phone has nothing more to do in the group once its sync ends.
    if (connection.peer != null) handler.post(::leave)
  }

  /** Announces, discovers and gives up idle groups, every so often. */
  private fun tick() {
    if (stopped) return
    try {
      refresh()
    } catch (error: Exception) {
      Log.w(TAG, "Wi-Fi Direct", error)
    }
    handler.postDelayed(::tick, TICK_MS)
  }

  private fun refresh() {
    val manager = manager ?: return
    val channel = channel ?: return
    val own = Direct.session()
    if (own == null || !enabled || !hasPermission()) {
      drop()
      return
    }
    val now = SystemClock.elapsedRealtime()
    if (hosting && connections.isEmpty() && now - hostedAt > IDLE_MS) {
      Log.i(TAG, "No one joined; giving up the group")
      restUntil = now + REST_MS
      unhost()
    }
    if (joining != null && connections.isEmpty() && now - joinedAt > JOIN_MS) {
      leave()
    }
    announce(own)
    if (!requested) {
      manager.setDnsSdResponseListeners(channel, null) { _, record, device -> heard(record, device) }
      manager.addServiceRequest(channel, WifiP2pDnsSdServiceRequest.newInstance(SERVICE_TYPE), null)
      requested = true
    }
    // Discovery runs for a while and stops by itself; a joined phone does not look further.
    if (joining == null) manager.discoverServices(channel, null)
  }

  /** Announces the service with the session id and whether this phone hosts. */
  private fun announce(own: ByteArray) {
    val manager = manager ?: return
    val channel = channel ?: return
    val wanted = hex(own) to hosting
    if (wanted == announced) return
    announced = wanted
    val record = mapOf(KEY_SESSION to wanted.first, KEY_HOSTING to if (hosting) "1" else "0")
    manager.clearLocalServices(channel, object : WifiP2pManager.ActionListener {
      override fun onSuccess() = add()
      override fun onFailure(reason: Int) = add()

      fun add() {
        manager.addLocalService(channel, WifiP2pDnsSdServiceInfo.newInstance(INSTANCE, SERVICE_TYPE, record), null)
      }
    })
  }

  /** Another phone's announcement: host for a lower session id, join a higher one that hosts. */
  private fun heard(record: Map<String, String>, device: WifiP2pDevice) {
    handler.post {
      val own = Direct.session() ?: return@post
      val peer = record[KEY_SESSION]?.let(::bytes) ?: return@post
      if (peer.size != 8 || peer.contentEquals(own)) return@post
      if (compare(peer, own) < 0) {
        if (!hosting && joining == null && SystemClock.elapsedRealtime() > restUntil) host(own)
      } else if (record[KEY_HOSTING] == "1" && !hosting && joining == null && Direct.shouldConnect(peer)) {
        join(peer, device)
      }
    }
  }

  /** Makes this phone's own group, which others join with what its session id gives. */
  private fun host(own: ByteArray) {
    val manager = manager ?: return
    val channel = channel ?: return
    hosting = true
    hostedAt = SystemClock.elapsedRealtime()
    val config = WifiP2pConfig.Builder()
      .setNetworkName(networkName(own))
      .setPassphrase(passphrase(own))
      .setGroupOperatingBand(WifiP2pConfig.GROUP_OWNER_BAND_AUTO)
      .build()
    manager.createGroup(channel, config, object : WifiP2pManager.ActionListener {
      override fun onSuccess() {
        handler.post { listen() }
      }

      override fun onFailure(reason: Int) {
        Log.w(TAG, "Could not make a group: $reason")
        handler.post { hosting = false }
      }
    })
  }

  /** Accepts the TCP connections of phones that joined the group. */
  private fun listen() {
    if (server != null) return
    val socket = try {
      ServerSocket().apply {
        reuseAddress = true
        bind(InetSocketAddress(PORT))
      }
    } catch (error: Exception) {
      Log.w(TAG, "Could not listen: $error")
      return
    }
    server = socket
    sockets.execute {
      while (!socket.isClosed) {
        try {
          val accepted = socket.accept()
          sockets.execute { connected(accepted, null) }
        } catch (_: Exception) {
          break
        }
      }
    }
  }

  private fun unhost() {
    hosting = false
    try {
      server?.close()
    } catch (_: Exception) {
    }
    server = null
    val manager = manager ?: return
    val channel = channel ?: return
    manager.removeGroup(channel, null)
  }

  /** Joins a host's group; `groupChanged` connects once the group is formed. */
  private fun join(peer: ByteArray, device: WifiP2pDevice) {
    val manager = manager ?: return
    val channel = channel ?: return
    joining = peer
    joinedAt = SystemClock.elapsedRealtime()
    val config = WifiP2pConfig.Builder()
      .setNetworkName(networkName(peer))
      .setPassphrase(passphrase(peer))
      .setDeviceAddress(MacAddress.fromString(device.deviceAddress))
      .build()
    manager.stopPeerDiscovery(channel, null)
    manager.connect(channel, config, object : WifiP2pManager.ActionListener {
      override fun onSuccess() {}

      override fun onFailure(reason: Int) {
        Log.w(TAG, "Could not join: $reason")
        handler.post { leave() }
      }
    })
  }

  /** The group formed or went away. */
  private fun groupChanged() {
    val manager = manager ?: return
    val channel = channel ?: return
    manager.requestConnectionInfo(channel) { info: WifiP2pInfo? ->
      handler.post {
        if (info == null || !info.groupFormed) {
          // The group ended: its connections end with it.
          connections.keys.toList().forEach { id ->
            close(id)
            Direct.closed(id)
          }
          if (hosting) unhost()
          joining?.let(::ended)
        } else if (!info.isGroupOwner) {
          val peer = joining ?: return@post
          val host = info.groupOwnerAddress ?: return@post
          if (connections.values.any { it.peer?.contentEquals(peer) == true }) return@post
          sockets.execute {
            try {
              val socket = Socket()
              socket.connect(InetSocketAddress(host, PORT), CONNECT_TIMEOUT_MS)
              connected(socket, peer)
            } catch (error: Exception) {
              Log.w(TAG, "Could not connect over Wi-Fi Direct: $error")
              handler.post { leave() }
            }
          }
        }
      }
    }
  }

  /** Leaves the group joined, if any. */
  private fun leave() {
    val peer = joining ?: return
    if (connections.values.any { it.peer != null }) return
    ended(peer)
    val manager = manager ?: return
    val channel = channel ?: return
    manager.removeGroup(channel, null)
  }

  private fun ended(peer: ByteArray) {
    if (joining?.contentEquals(peer) != true) return
    joining = null
    Direct.dialEnded(peer)
  }

  /** Hands a connected socket's stream to the node, and its bytes as they come. */
  private fun connected(socket: Socket, peer: ByteArray?) {
    socket.tcpNoDelay = true
    val id = nextId.getAndIncrement()
    connections[id] = Connection(socket, socket.getOutputStream(), Executors.newSingleThreadExecutor(), peer)
    Direct.opened(id, peer)
    val buffer = ByteArray(READ_LEN)
    try {
      val input = socket.getInputStream()
      while (true) {
        val read = input.read(buffer)
        if (read < 0) break
        if (read > 0) Direct.received(id, buffer.copyOf(read))
      }
    } catch (_: Exception) {
    }
    if (connections.containsKey(id)) {
      close(id)
      Direct.closed(id)
    }
  }

  /** Ends discovery, the group and every connection, as when Wi-Fi goes off. */
  private fun drop() {
    connections.keys.toList().forEach { id ->
      close(id)
      Direct.closed(id)
    }
    if (hosting) unhost()
    joining?.let(::ended)
    val manager = manager ?: return
    val channel = channel ?: return
    if (announced != null) manager.clearLocalServices(channel, null)
    announced = null
    if (requested) manager.clearServiceRequests(channel, null)
    requested = false
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
    private const val TAG = "CarryChatDirect"
    private const val INSTANCE = "CarryChat"
    private const val SERVICE_TYPE = "_carrychat._tcp"
    private const val KEY_SESSION = "s"
    private const val KEY_HOSTING = "h"
    /** The host listens here, on its group's address. */
    private const val PORT = 43524
    private const val TICK_MS = 30_000L
    /** A group no one joins in this long is given up, and not made again for a while. */
    private const val IDLE_MS = 120_000L
    private const val REST_MS = 600_000L
    /** A join that has not connected in this long is given up. */
    private const val JOIN_MS = 60_000L
    private const val CONNECT_TIMEOUT_MS = 15_000
    private const val READ_LEN = 4096

    /** Whether this phone can use Wi-Fi Direct as this link does. */
    fun supported(context: Context): Boolean =
      Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q &&
        context.packageManager.hasSystemFeature(PackageManager.FEATURE_WIFI_DIRECT)

    /** Wi-Fi Direct group names start with `DIRECT-` and two characters. */
    private fun networkName(session: ByteArray) = "DIRECT-CC-" + hex(session)

    /** Encrypts the link; everything on it is sealed end to end already. */
    private fun passphrase(session: ByteArray) = "carrychat-" + hex(session)

    private fun hex(bytes: ByteArray) = bytes.joinToString("") { "%02x".format(it) }

    private fun bytes(hex: String): ByteArray? =
      if (hex.length % 2 != 0) null
      else runCatching { ByteArray(hex.length / 2) { hex.substring(it * 2, it * 2 + 2).toInt(16).toByte() } }.getOrNull()

    /** Compares session ids as the node does: unsigned, big-endian. */
    private fun compare(a: ByteArray, b: ByteArray): Int {
      for (i in a.indices) {
        val diff = (a[i].toInt() and 0xff) - (b[i].toInt() and 0xff)
        if (diff != 0) return diff
      }
      return 0
    }
  }
}
