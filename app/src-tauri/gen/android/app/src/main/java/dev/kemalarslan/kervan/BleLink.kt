package dev.kemalarslan.kervan

import android.Manifest
import android.annotation.SuppressLint
import android.bluetooth.BluetoothAdapter
import android.bluetooth.BluetoothDevice
import android.bluetooth.BluetoothGatt
import android.bluetooth.BluetoothGattCallback
import android.bluetooth.BluetoothGattCharacteristic
import android.bluetooth.BluetoothGattDescriptor
import android.bluetooth.BluetoothGattServer
import android.bluetooth.BluetoothGattServerCallback
import android.bluetooth.BluetoothGattService
import android.bluetooth.BluetoothManager
import android.bluetooth.BluetoothProfile
import android.bluetooth.le.AdvertiseCallback
import android.bluetooth.le.AdvertiseData
import android.bluetooth.le.AdvertiseSettings
import android.bluetooth.le.ScanCallback
import android.bluetooth.le.ScanFilter
import android.bluetooth.le.ScanResult
import android.bluetooth.le.ScanSettings
import android.content.Context
import android.content.pm.PackageManager
import android.os.Build
import android.os.Handler
import android.os.HandlerThread
import android.os.ParcelUuid
import android.util.Log
import java.io.ByteArrayOutputStream
import java.util.UUID

/** The Rust side of the Bluetooth link (`src-tauri/src/android.rs`). */
object Ble {
  init {
    System.loadLibrary("kervan_app_lib")
  }

  /** The session id to advertise, or null while the node does not use Bluetooth. */
  @JvmStatic external fun session(): ByteArray?
  @JvmStatic external fun shouldConnect(peer: ByteArray): Boolean
  @JvmStatic external fun dialEnded(peer: ByteArray)
  /** A connection is ready; `peer` is the node this side connected to, if it did. */
  @JvmStatic external fun opened(id: Int, peer: ByteArray?)
  @JvmStatic external fun received(id: Int, data: ByteArray)
  @JvmStatic external fun closed(id: Int)

  @Volatile internal var link: BleLink? = null

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
 * The Bluetooth LE link on Android (§9.5). It advertises the Kervan service
 * with the node's session id, accepts connections to it, scans for other
 * nodes that advertise it and connects to them. Each connection carries the
 * node's sync stream, which the Rust side runs: this class only moves bytes,
 * cut to the connection's MTU. They go as indications and writes with
 * response, so one value at a time is in flight each way: notifications
 * filled the stack's queue until it could not answer the other side's
 * writes, and the link dropped when those answers were 30 seconds late.
 *
 * Everything runs on one thread, so the maps need no locks.
 */
@SuppressLint("MissingPermission")
class BleLink(private val context: Context) {
  private val thread = HandlerThread("kervan-ble").apply { start() }
  private val handler = Handler(thread.looper)
  private val manager = context.getSystemService(BluetoothManager::class.java)

  private var session: ByteArray? = null
  private var server: BluetoothGattServer? = null
  private var serverCharacteristic: BluetoothGattCharacteristic? = null
  private var advertising = false
  private var scanning = false
  private var nextId = 1

  /** Connections that carry a stream, by the id the Rust side knows them by. */
  private val connections = HashMap<Int, Connection>()
  /** Connections this side is still setting up, by device address. */
  private val dials = HashMap<String, Dial>()
  /** MTUs of devices connected to the server, before they subscribe. */
  private val serverMtus = HashMap<String, Int>()

  private class Dial(val peer: ByteArray, val gatt: BluetoothGatt) {
    var mtu = DEFAULT_MTU
  }

  private class Connection(
    val id: Int,
    val device: BluetoothDevice,
    /** Set when this side connected; null when the device connected to the server. */
    val gatt: BluetoothGatt?,
    val characteristic: BluetoothGattCharacteristic,
    var mtu: Int,
  ) {
    val queue = ArrayDeque<ByteArray>()
    var busy = false
    var prepared: ByteArrayOutputStream? = null
  }

  fun start() {
    Ble.link = this
    handler.post(::tick)
  }

  fun stop() {
    Ble.link = null
    handler.removeCallbacksAndMessages(null)
    handler.post {
      stopAll()
      thread.quitSafely()
    }
  }

  fun send(id: Int, data: ByteArray) {
    handler.post {
      val connection = connections[id] ?: return@post
      // MTU − 3 per write or indication, but never past the 512 bytes an
      // attribute value may hold: a large MTU would otherwise allow more.
      val len = (connection.mtu - 3).coerceIn(20, MAX_VALUE_LEN)
      var offset = 0
      while (offset < data.size) {
        val end = minOf(data.size, offset + len)
        connection.queue.addLast(data.copyOfRange(offset, end))
        offset = end
      }
      pump(connection)
    }
  }

  fun close(id: Int) {
    handler.post {
      val connection = connections.remove(id) ?: return@post
      disconnect(connection)
    }
  }

  /** Brings the link in line with Bluetooth, permissions and the node, every few seconds. */
  private fun tick() {
    try {
      refresh()
    } catch (error: Exception) {
      Log.w(TAG, "Bluetooth", error)
    }
    handler.postDelayed(::tick, TICK_MS)
  }

  private fun refresh() {
    val adapter = manager?.adapter
    val session = if (adapter != null && adapter.isEnabled && hasPermissions()) Ble.session() else null
    if (session == null || !session.contentEquals(this.session)) {
      stopAll()
      this.session = session
    }
    if (adapter == null || session == null) return
    if (server == null) openServer()
    if (!advertising) advertise(adapter, session)
    if (!scanning) scan(adapter)
  }

  private fun hasPermissions(): Boolean {
    val needed = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.S) {
      listOf(
        Manifest.permission.BLUETOOTH_SCAN,
        Manifest.permission.BLUETOOTH_ADVERTISE,
        Manifest.permission.BLUETOOTH_CONNECT,
      )
    } else {
      listOf(Manifest.permission.ACCESS_FINE_LOCATION)
    }
    return needed.all { context.checkSelfPermission(it) == PackageManager.PERMISSION_GRANTED }
  }

  private fun stopAll() {
    val adapter = manager?.adapter
    if (scanning) {
      runCatching { adapter?.bluetoothLeScanner?.stopScan(scanCallback) }
      scanning = false
    }
    if (advertising) {
      runCatching { adapter?.bluetoothLeAdvertiser?.stopAdvertising(advertiseCallback) }
      advertising = false
    }
    for (connection in connections.values) {
      Ble.closed(connection.id)
      disconnect(connection)
    }
    connections.clear()
    for (dial in dials.values) {
      Ble.dialEnded(dial.peer)
      runCatching { dial.gatt.close() }
    }
    dials.clear()
    serverMtus.clear()
    runCatching { server?.close() }
    server = null
    serverCharacteristic = null
  }

  private fun disconnect(connection: Connection) {
    val gatt = connection.gatt
    if (gatt != null) {
      runCatching {
        gatt.disconnect()
        gatt.close()
      }
    } else {
      runCatching { server?.cancelConnection(connection.device) }
    }
  }

  // Advertising and the server: other nodes connect to this one.

  private fun openServer() {
    val characteristic = BluetoothGattCharacteristic(
      DATA_UUID,
      BluetoothGattCharacteristic.PROPERTY_WRITE or
        BluetoothGattCharacteristic.PROPERTY_WRITE_NO_RESPONSE or
        BluetoothGattCharacteristic.PROPERTY_INDICATE,
      BluetoothGattCharacteristic.PERMISSION_WRITE,
    )
    characteristic.addDescriptor(
      BluetoothGattDescriptor(
        CCCD_UUID,
        BluetoothGattDescriptor.PERMISSION_READ or BluetoothGattDescriptor.PERMISSION_WRITE,
      ),
    )
    val service = BluetoothGattService(SERVICE_UUID, BluetoothGattService.SERVICE_TYPE_PRIMARY)
    service.addCharacteristic(characteristic)
    val server = manager?.openGattServer(context, serverCallback) ?: return
    server.addService(service)
    this.server = server
    serverCharacteristic = characteristic
  }

  private fun advertise(adapter: BluetoothAdapter, session: ByteArray) {
    val advertiser = adapter.bluetoothLeAdvertiser ?: return
    val settings = AdvertiseSettings.Builder()
      .setAdvertiseMode(AdvertiseSettings.ADVERTISE_MODE_BALANCED)
      .setTxPowerLevel(AdvertiseSettings.ADVERTISE_TX_POWER_MEDIUM)
      .setConnectable(true)
      .setTimeout(0)
      .build()
    // The service in the advertisement, so scans can filter on it; the
    // session id in the scan response, where it fits beside the 128-bit uuid.
    val data = AdvertiseData.Builder()
      .addServiceUuid(ParcelUuid(SERVICE_UUID))
      .setIncludeDeviceName(false)
      .build()
    val response = AdvertiseData.Builder()
      .addServiceData(ParcelUuid(SERVICE_UUID), session)
      .setIncludeDeviceName(false)
      .build()
    advertiser.startAdvertising(settings, data, response, advertiseCallback)
    advertising = true
  }

  private fun restartAdvertising() {
    if (!advertising) return
    runCatching { manager?.adapter?.bluetoothLeAdvertiser?.stopAdvertising(advertiseCallback) }
    advertising = false
    refresh()
  }

  private val advertiseCallback = object : AdvertiseCallback() {
    override fun onStartFailure(errorCode: Int) {
      Log.w(TAG, "Advertising failed: $errorCode")
      handler.post { advertising = errorCode == ADVERTISE_FAILED_ALREADY_STARTED }
    }
  }

  private val serverCallback = object : BluetoothGattServerCallback() {
    override fun onConnectionStateChange(device: BluetoothDevice, status: Int, newState: Int) {
      // A connection can stop the advertisement: start it again, so the
      // node stays findable while connected and right after.
      handler.post(::restartAdvertising)
      if (newState != BluetoothProfile.STATE_DISCONNECTED) return
      handler.post {
        serverMtus.remove(device.address)
        val connection = serverConnection(device) ?: return@post
        connections.remove(connection.id)
        Ble.closed(connection.id)
      }
    }

    override fun onMtuChanged(device: BluetoothDevice, mtu: Int) {
      handler.post {
        serverMtus[device.address] = mtu
        serverConnection(device)?.mtu = mtu
      }
    }

    override fun onDescriptorReadRequest(
      device: BluetoothDevice,
      requestId: Int,
      offset: Int,
      descriptor: BluetoothGattDescriptor,
    ) {
      handler.post {
        val value = if (serverConnection(device) != null) {
          BluetoothGattDescriptor.ENABLE_INDICATION_VALUE
        } else {
          BluetoothGattDescriptor.DISABLE_NOTIFICATION_VALUE
        }
        server?.sendResponse(device, requestId, BluetoothGatt.GATT_SUCCESS, 0, value)
      }
    }

    /** Subscribing to the indications is what opens the stream. */
    override fun onDescriptorWriteRequest(
      device: BluetoothDevice,
      requestId: Int,
      descriptor: BluetoothGattDescriptor,
      preparedWrite: Boolean,
      responseNeeded: Boolean,
      offset: Int,
      value: ByteArray,
    ) {
      if (responseNeeded) {
        server?.sendResponse(device, requestId, BluetoothGatt.GATT_SUCCESS, 0, null)
      }
      val subscribed = value.contentEquals(BluetoothGattDescriptor.ENABLE_INDICATION_VALUE) ||
        value.contentEquals(BluetoothGattDescriptor.ENABLE_NOTIFICATION_VALUE)
      handler.post {
        val existing = serverConnection(device)
        val characteristic = serverCharacteristic
        if (subscribed && existing == null && characteristic != null) {
          val id = nextId++
          val mtu = serverMtus[device.address] ?: DEFAULT_MTU
          connections[id] = Connection(id, device, null, characteristic, mtu)
          Ble.opened(id, null)
        } else if (!subscribed && existing != null) {
          connections.remove(existing.id)
          Ble.closed(existing.id)
        }
      }
    }

    override fun onCharacteristicWriteRequest(
      device: BluetoothDevice,
      requestId: Int,
      characteristic: BluetoothGattCharacteristic,
      preparedWrite: Boolean,
      responseNeeded: Boolean,
      offset: Int,
      value: ByteArray,
    ) {
      if (responseNeeded) {
        server?.sendResponse(device, requestId, BluetoothGatt.GATT_SUCCESS, offset, value)
      }
      val bytes = value.copyOf()
      handler.post {
        val connection = serverConnection(device) ?: return@post
        if (preparedWrite) {
          // A write longer than the MTU arrives in pieces, kept until executed.
          val prepared = connection.prepared ?: ByteArrayOutputStream().also { connection.prepared = it }
          prepared.write(bytes)
        } else {
          Ble.received(connection.id, bytes)
        }
      }
    }

    override fun onExecuteWrite(device: BluetoothDevice, requestId: Int, execute: Boolean) {
      server?.sendResponse(device, requestId, BluetoothGatt.GATT_SUCCESS, 0, null)
      handler.post {
        val connection = serverConnection(device) ?: return@post
        val prepared = connection.prepared ?: return@post
        connection.prepared = null
        if (execute) Ble.received(connection.id, prepared.toByteArray())
      }
    }

    override fun onNotificationSent(device: BluetoothDevice, status: Int) {
      handler.post {
        val connection = serverConnection(device) ?: return@post
        connection.busy = false
        pump(connection)
      }
    }
  }

  private fun serverConnection(device: BluetoothDevice): Connection? =
    connections.values.firstOrNull { it.gatt == null && it.device.address == device.address }

  // Scanning and connecting: this node connects to others.

  private fun scan(adapter: BluetoothAdapter) {
    val scanner = adapter.bluetoothLeScanner ?: return
    val filter = ScanFilter.Builder().setServiceUuid(ParcelUuid(SERVICE_UUID)).build()
    val settings = ScanSettings.Builder().setScanMode(ScanSettings.SCAN_MODE_BALANCED).build()
    scanner.startScan(listOf(filter), settings, scanCallback)
    scanning = true
  }

  private val scanCallback = object : ScanCallback() {
    override fun onScanResult(callbackType: Int, result: ScanResult) {
      val peer = result.scanRecord?.getServiceData(ParcelUuid(SERVICE_UUID)) ?: return
      if (peer.size != 8) return
      handler.post { consider(result.device, peer) }
    }

    override fun onScanFailed(errorCode: Int) {
      Log.w(TAG, "Scanning failed: $errorCode")
      handler.post { scanning = false }
    }
  }

  private fun consider(device: BluetoothDevice, peer: ByteArray) {
    val address = device.address
    if (dials.containsKey(address) || connections.values.any { it.device.address == address }) return
    if (!Ble.shouldConnect(peer)) return
    val gatt = device.connectGatt(context, false, clientCallback, BluetoothDevice.TRANSPORT_LE)
    if (gatt == null) {
      Ble.dialEnded(peer)
      return
    }
    val dial = Dial(peer, gatt)
    dials[address] = dial
    handler.postDelayed({
      // Still setting up after this long: give up, and try again when seen.
      if (dials[address] === dial) failDial(address)
    }, DIAL_TIMEOUT_MS)
  }

  private fun failDial(address: String) {
    val dial = dials.remove(address) ?: return
    runCatching {
      dial.gatt.disconnect()
      dial.gatt.close()
    }
    Ble.dialEnded(dial.peer)
  }

  private val clientCallback = object : BluetoothGattCallback() {
    override fun onConnectionStateChange(gatt: BluetoothGatt, status: Int, newState: Int) {
      handler.post {
        val address = gatt.device.address
        when (newState) {
          BluetoothProfile.STATE_CONNECTED -> if (dials.containsKey(address)) gatt.requestMtu(MAX_MTU)
          BluetoothProfile.STATE_DISCONNECTED -> {
            if (dials.containsKey(address)) {
              failDial(address)
            } else {
              val connection = connections.values.firstOrNull { it.gatt === gatt }
              if (connection != null) {
                connections.remove(connection.id)
                Ble.closed(connection.id)
              }
              runCatching { gatt.close() }
            }
          }
        }
      }
    }

    override fun onMtuChanged(gatt: BluetoothGatt, mtu: Int, status: Int) {
      handler.post {
        val dial = dials[gatt.device.address] ?: return@post
        if (status == BluetoothGatt.GATT_SUCCESS) dial.mtu = mtu
        if (!gatt.discoverServices()) failDial(gatt.device.address)
      }
    }

    override fun onServicesDiscovered(gatt: BluetoothGatt, status: Int) {
      handler.post {
        val address = gatt.device.address
        if (!dials.containsKey(address)) return@post
        val characteristic = gatt.getService(SERVICE_UUID)?.getCharacteristic(DATA_UUID)
        val descriptor = characteristic?.getDescriptor(CCCD_UUID)
        if (characteristic == null || descriptor == null ||
          !gatt.setCharacteristicNotification(characteristic, true) ||
          !writeDescriptor(gatt, descriptor, BluetoothGattDescriptor.ENABLE_INDICATION_VALUE)
        ) {
          failDial(address)
        }
      }
    }

    override fun onDescriptorWrite(gatt: BluetoothGatt, descriptor: BluetoothGattDescriptor, status: Int) {
      handler.post {
        val address = gatt.device.address
        val dial = dials.remove(address) ?: return@post
        val characteristic = descriptor.characteristic
        if (status != BluetoothGatt.GATT_SUCCESS) {
          dials[address] = dial
          failDial(address)
          return@post
        }
        val id = nextId++
        connections[id] = Connection(id, gatt.device, gatt, characteristic, dial.mtu)
        Ble.opened(id, dial.peer)
      }
    }

    override fun onCharacteristicChanged(
      gatt: BluetoothGatt,
      characteristic: BluetoothGattCharacteristic,
      value: ByteArray,
    ) {
      received(gatt, value.copyOf())
    }

    @Deprecated("Called before Android 13")
    override fun onCharacteristicChanged(gatt: BluetoothGatt, characteristic: BluetoothGattCharacteristic) {
      if (Build.VERSION.SDK_INT < Build.VERSION_CODES.TIRAMISU) {
        @Suppress("DEPRECATION")
        received(gatt, characteristic.value.copyOf())
      }
    }

    override fun onCharacteristicWrite(gatt: BluetoothGatt, characteristic: BluetoothGattCharacteristic, status: Int) {
      handler.post {
        val connection = connections.values.firstOrNull { it.gatt === gatt } ?: return@post
        connection.busy = false
        pump(connection)
      }
    }
  }

  private fun received(gatt: BluetoothGatt, bytes: ByteArray) {
    handler.post {
      val connection = connections.values.firstOrNull { it.gatt === gatt } ?: return@post
      Ble.received(connection.id, bytes)
    }
  }

  // Sending: one write or indication at a time, the next when it is confirmed.

  private fun pump(connection: Connection) {
    if (connection.busy || !connections.containsKey(connection.id)) return
    val chunk = connection.queue.removeFirstOrNull() ?: return
    val gatt = connection.gatt
    val server = server
    val started = try {
      if (gatt != null) {
        writeCharacteristic(gatt, connection.characteristic, chunk)
      } else {
        server != null && indicate(server, connection.device, connection.characteristic, chunk)
      }
    } catch (error: RuntimeException) {
      // A send the stack refuses outright ends this connection, not the app.
      Log.w(TAG, "Could not send over Bluetooth", error)
      connections.remove(connection.id)
      Ble.closed(connection.id)
      disconnect(connection)
      return
    }
    if (started) {
      connection.busy = true
    } else {
      // The stack is busy: try the same chunk again shortly.
      connection.queue.addFirst(chunk)
      handler.postDelayed({ pump(connection) }, RETRY_MS)
    }
  }

  @Suppress("DEPRECATION")
  private fun writeCharacteristic(
    gatt: BluetoothGatt,
    characteristic: BluetoothGattCharacteristic,
    value: ByteArray,
  ): Boolean {
    val type = BluetoothGattCharacteristic.WRITE_TYPE_DEFAULT
    return if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
      gatt.writeCharacteristic(characteristic, value, type) == BluetoothGatt.GATT_SUCCESS
    } else {
      characteristic.writeType = type
      characteristic.value = value
      gatt.writeCharacteristic(characteristic)
    }
  }

  @Suppress("DEPRECATION")
  private fun writeDescriptor(gatt: BluetoothGatt, descriptor: BluetoothGattDescriptor, value: ByteArray): Boolean =
    if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
      gatt.writeDescriptor(descriptor, value) == BluetoothGatt.GATT_SUCCESS
    } else {
      descriptor.value = value
      gatt.writeDescriptor(descriptor)
    }

  @Suppress("DEPRECATION")
  private fun indicate(
    server: BluetoothGattServer,
    device: BluetoothDevice,
    characteristic: BluetoothGattCharacteristic,
    value: ByteArray,
  ): Boolean =
    if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
      server.notifyCharacteristicChanged(device, characteristic, true, value) == BluetoothGatt.GATT_SUCCESS
    } else {
      characteristic.value = value
      server.notifyCharacteristicChanged(device, characteristic, true)
    }

  companion object {
    private const val TAG = "KervanBle"
    /** The Kervan service and the characteristic the stream crosses (§9.5). */
    val SERVICE_UUID: UUID = UUID.fromString("4b52564e-a0c5-4a3e-9b31-6f2d8e7c1a01")
    val DATA_UUID: UUID = UUID.fromString("4b52564e-a0c5-4a3e-9b31-6f2d8e7c1a02")
    private val CCCD_UUID: UUID = UUID.fromString("00002902-0000-1000-8000-00805f9b34fb")
    private const val DEFAULT_MTU = 23
    private const val MAX_VALUE_LEN = 512
    private const val MAX_MTU = 517
    private const val TICK_MS = 5_000L
    private const val DIAL_TIMEOUT_MS = 20_000L
    private const val RETRY_MS = 20L
  }
}
