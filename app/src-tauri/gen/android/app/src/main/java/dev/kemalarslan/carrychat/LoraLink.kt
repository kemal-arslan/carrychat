package dev.kemalarslan.carrychat

import android.Manifest
import android.annotation.SuppressLint
import android.bluetooth.BluetoothDevice
import android.bluetooth.BluetoothGatt
import android.bluetooth.BluetoothGattCallback
import android.bluetooth.BluetoothGattCharacteristic
import android.bluetooth.BluetoothGattDescriptor
import android.bluetooth.BluetoothManager
import android.bluetooth.BluetoothProfile
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
import java.util.UUID

/** The Rust side of LoRa over Bluetooth LE (`src-tauri/src/android.rs`). */
object Lora {
  init {
    System.loadLibrary("carrychat_app_lib")
  }

  /** The Bluetooth address of the LoRa device chosen, or null. */
  @JvmStatic external fun device(): String?
  /** Connected to the device, which runs MeshCore or else Meshtastic; its messages follow. */
  @JvmStatic external fun opened(meshcore: Boolean)
  /** One message from the device: a `FromRadio`, or a MeshCore frame. */
  @JvmStatic external fun received(data: ByteArray)
  @JvmStatic external fun closed()
  /** A LoRa device the scan found. */
  @JvmStatic external fun found(address: String, name: String?)

  @Volatile internal var link: LoraLink? = null

  /** Called by the Rust side: one `ToRadio` message, or MeshCore frame, for the device. */
  @JvmStatic fun send(data: ByteArray) {
    link?.send(data)
  }

  /** Called by the Rust side when the link over the connection ended. */
  @JvmStatic fun close() {
    link?.disconnect()
  }

  /** Called by the Rust side: looks for LoRa devices for a few seconds. */
  @JvmStatic fun scan() {
    link?.scan()
  }
}

/**
 * Connects to the LoRa device chosen over Bluetooth LE, as the device's own
 * apps do. A Meshtastic device takes messages written to ToRadio, and its
 * messages are read from FromRadio until it is empty, whenever FromNum says
 * there are new ones. A MeshCore device takes frames written to its UART
 * service's RX characteristic, and notifies its own on TX. The Rust side
 * runs the link; this class only moves the messages. The device asks to pair
 * first, with the PIN on its screen or its fixed one.
 *
 * Everything runs on one thread, and GATT operations go one at a time.
 */
@SuppressLint("MissingPermission")
class LoraLink(private val context: Context) {
  private val thread = HandlerThread("carrychat-lora").apply { start() }
  private val handler = Handler(thread.looper)
  private val manager = context.getSystemService(BluetoothManager::class.java)

  private var address: String? = null
  private var gatt: BluetoothGatt? = null
  private var toRadio: BluetoothGattCharacteristic? = null
  private var fromRadio: BluetoothGattCharacteristic? = null
  private var meshcore = false
  private var ready = false
  private var reading = false
  private var scanning = false

  /** GATT operations waiting; each returns whether it started one to wait for. */
  private val operations = ArrayDeque<() -> Boolean>()
  private var busy = false

  fun start() {
    Lora.link = this
    handler.post(::tick)
  }

  fun stop() {
    Lora.link = null
    handler.removeCallbacksAndMessages(null)
    handler.post {
      stopScan()
      disconnectNow()
      thread.quitSafely()
    }
  }

  fun send(data: ByteArray) {
    handler.post {
      val gatt = gatt ?: return@post
      val characteristic = toRadio ?: return@post
      enqueue { write(gatt, characteristic, data) }
    }
  }

  fun disconnect() {
    handler.post(::disconnectNow)
  }

  fun scan() {
    handler.post {
      val scanner = manager?.adapter?.takeIf { it.isEnabled }?.bluetoothLeScanner ?: return@post
      if (!hasPermissions()) return@post
      stopScan()
      val filters = listOf(SERVICE, MESHCORE_SERVICE).map {
        ScanFilter.Builder().setServiceUuid(ParcelUuid(it)).build()
      }
      val settings = ScanSettings.Builder().setScanMode(ScanSettings.SCAN_MODE_LOW_LATENCY).build()
      scanning = runCatching { scanner.startScan(filters, settings, scanCallback) }.isSuccess
      handler.postDelayed(::stopScan, SCAN_MS)
    }
  }

  /** Brings the connection in line with the device chosen, every few seconds. */
  private fun tick() {
    try {
      refresh()
    } catch (error: Exception) {
      Log.w(TAG, "LoRa", error)
    }
    handler.postDelayed(::tick, TICK_MS)
  }

  private fun refresh() {
    val adapter = manager?.adapter
    val wanted = if (adapter != null && adapter.isEnabled && hasPermissions()) Lora.device() else null
    if (wanted != address) {
      disconnectNow()
      address = wanted
    }
    if (adapter == null || wanted == null || gatt != null) return
    val device = adapter.getRemoteDevice(wanted)
    // Both kinds only talk to a paired phone; Android shows the PIN prompt.
    when (device.bondState) {
      BluetoothDevice.BOND_NONE -> {
        device.createBond()
        return
      }
      BluetoothDevice.BOND_BONDING -> return
    }
    gatt = device.connectGatt(context, false, callback, BluetoothDevice.TRANSPORT_LE)
  }

  private fun hasPermissions(): Boolean {
    val needed = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.S) {
      listOf(Manifest.permission.BLUETOOTH_SCAN, Manifest.permission.BLUETOOTH_CONNECT)
    } else {
      listOf(Manifest.permission.ACCESS_FINE_LOCATION)
    }
    return needed.all { context.checkSelfPermission(it) == PackageManager.PERMISSION_GRANTED }
  }

  private fun setUp(gatt: BluetoothGatt) {
    val meshtastic = gatt.getService(SERVICE)
    val uart = gatt.getService(MESHCORE_SERVICE)
    // Meshtastic: FromNum says when to read FromRadio. MeshCore: TX carries the frames.
    val notifying = if (meshtastic != null) {
      meshcore = false
      toRadio = meshtastic.getCharacteristic(TO_RADIO)
      fromRadio = meshtastic.getCharacteristic(FROM_RADIO) ?: meshtastic.getCharacteristic(FROM_RADIO_OLD)
      meshtastic.getCharacteristic(FROM_NUM)?.takeIf { fromRadio != null }
    } else {
      meshcore = true
      toRadio = uart?.getCharacteristic(MESHCORE_RX)
      uart?.getCharacteristic(MESHCORE_TX)
    }
    if (toRadio == null || notifying == null) {
      Log.w(TAG, "Not a Meshtastic or MeshCore device")
      disconnectNow()
      return
    }
    gatt.setCharacteristicNotification(notifying, true)
    notifying.getDescriptor(CCCD_UUID)?.let { descriptor ->
      enqueue { writeDescriptor(gatt, descriptor, BluetoothGattDescriptor.ENABLE_NOTIFICATION_VALUE) }
    }
    enqueue {
      ready = true
      Lora.opened(meshcore)
      false
    }
  }

  /** Something the device notified: a MeshCore frame, or Meshtastic's word to read. */
  private fun notified(value: ByteArray) {
    if (!meshcore) {
      drain()
    } else if (ready && value.isNotEmpty()) {
      Lora.received(value)
    }
  }

  private fun enqueue(operation: () -> Boolean) {
    operations.addLast(operation)
    next()
  }

  private fun next() {
    while (!busy) {
      val operation = operations.removeFirstOrNull() ?: return
      busy = operation()
    }
  }

  private fun done() {
    busy = false
    next()
  }

  /** Reads FromRadio until the device has nothing more to say. */
  private fun drain() {
    if (reading || !ready || meshcore) return
    val gatt = gatt ?: return
    val characteristic = fromRadio ?: return
    reading = true
    enqueue {
      gatt.readCharacteristic(characteristic).also { started -> if (!started) reading = false }
    }
  }

  private fun readDone(value: ByteArray, status: Int) {
    reading = false
    done()
    if (status != BluetoothGatt.GATT_SUCCESS) {
      // Most often the device is still pairing; try again shortly.
      handler.postDelayed(::drain, RETRY_MS)
    } else if (value.isNotEmpty()) {
      Lora.received(value)
      drain()
    }
  }

  private fun disconnectNow() {
    val gatt = gatt ?: return
    this.gatt = null
    toRadio = null
    fromRadio = null
    val wasReady = ready
    ready = false
    reading = false
    busy = false
    operations.clear()
    runCatching { gatt.disconnect() }
    runCatching { gatt.close() }
    if (wasReady) Lora.closed()
  }

  private fun stopScan() {
    if (!scanning) return
    scanning = false
    runCatching { manager?.adapter?.bluetoothLeScanner?.stopScan(scanCallback) }
  }

  private val scanCallback = object : ScanCallback() {
    override fun onScanResult(callbackType: Int, result: ScanResult) {
      val name = result.scanRecord?.deviceName ?: runCatching { result.device.name }.getOrNull()
      Lora.found(result.device.address, name)
    }
  }

  private val callback = object : BluetoothGattCallback() {
    override fun onConnectionStateChange(gatt: BluetoothGatt, status: Int, newState: Int) {
      handler.post {
        if (gatt != this@LoraLink.gatt) return@post
        if (newState == BluetoothProfile.STATE_CONNECTED && status == BluetoothGatt.GATT_SUCCESS) {
          gatt.requestMtu(MTU)
        } else if (newState == BluetoothProfile.STATE_DISCONNECTED || status != BluetoothGatt.GATT_SUCCESS) {
          disconnectNow()
        }
      }
    }

    override fun onMtuChanged(gatt: BluetoothGatt, mtu: Int, status: Int) {
      handler.post { if (gatt == this@LoraLink.gatt) gatt.discoverServices() }
    }

    override fun onServicesDiscovered(gatt: BluetoothGatt, status: Int) {
      handler.post { if (gatt == this@LoraLink.gatt) setUp(gatt) }
    }

    override fun onDescriptorWrite(gatt: BluetoothGatt, descriptor: BluetoothGattDescriptor, status: Int) {
      handler.post { if (gatt == this@LoraLink.gatt) done() }
    }

    override fun onCharacteristicWrite(
      gatt: BluetoothGatt,
      characteristic: BluetoothGattCharacteristic,
      status: Int,
    ) {
      handler.post {
        if (gatt != this@LoraLink.gatt) return@post
        done()
        // A Meshtastic device answers what it was sent through FromRadio.
        drain()
      }
    }

    override fun onCharacteristicRead(
      gatt: BluetoothGatt,
      characteristic: BluetoothGattCharacteristic,
      value: ByteArray,
      status: Int,
    ) {
      handler.post { if (gatt == this@LoraLink.gatt) readDone(value, status) }
    }

    @Deprecated("Called before Android 13")
    override fun onCharacteristicRead(
      gatt: BluetoothGatt,
      characteristic: BluetoothGattCharacteristic,
      status: Int,
    ) {
      if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) return
      @Suppress("DEPRECATION")
      val value = characteristic.value ?: ByteArray(0)
      handler.post { if (gatt == this@LoraLink.gatt) readDone(value, status) }
    }

    override fun onCharacteristicChanged(
      gatt: BluetoothGatt,
      characteristic: BluetoothGattCharacteristic,
      value: ByteArray,
    ) {
      handler.post { if (gatt == this@LoraLink.gatt) notified(value) }
    }

    @Deprecated("Called before Android 13")
    override fun onCharacteristicChanged(gatt: BluetoothGatt, characteristic: BluetoothGattCharacteristic) {
      if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) return
      @Suppress("DEPRECATION")
      val value = characteristic.value?.copyOf() ?: ByteArray(0)
      handler.post { if (gatt == this@LoraLink.gatt) notified(value) }
    }
  }

  private fun write(gatt: BluetoothGatt, characteristic: BluetoothGattCharacteristic, value: ByteArray): Boolean {
    val type = BluetoothGattCharacteristic.WRITE_TYPE_DEFAULT
    return if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
      gatt.writeCharacteristic(characteristic, value, type) == BluetoothGatt.GATT_SUCCESS
    } else {
      @Suppress("DEPRECATION")
      characteristic.value = value
      characteristic.writeType = type
      @Suppress("DEPRECATION")
      gatt.writeCharacteristic(characteristic)
    }
  }

  private fun writeDescriptor(gatt: BluetoothGatt, descriptor: BluetoothGattDescriptor, value: ByteArray): Boolean =
    if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
      gatt.writeDescriptor(descriptor, value) == BluetoothGatt.GATT_SUCCESS
    } else {
      @Suppress("DEPRECATION")
      descriptor.value = value
      @Suppress("DEPRECATION")
      gatt.writeDescriptor(descriptor)
    }

  companion object {
    private const val TAG = "CarryChatLora"
    /** Meshtastic's Bluetooth service and characteristics. */
    private val SERVICE: UUID = UUID.fromString("6ba1b218-15a8-461f-9fa8-5dcae273eafd")
    private val TO_RADIO: UUID = UUID.fromString("f75c76d2-129e-4dad-a1dd-7866124401e7")
    private val FROM_RADIO: UUID = UUID.fromString("2c55e69e-4993-11ed-b878-0242ac120002")
    /** FromRadio before firmware 2.1. */
    private val FROM_RADIO_OLD: UUID = UUID.fromString("8ba2bcc2-ee02-4a55-a531-c525c5e454d5")
    private val FROM_NUM: UUID = UUID.fromString("ed9da18c-a800-4f66-a670-aa7547e34453")
    /** MeshCore's Bluetooth service, the Nordic UART service: the app writes RX, the device notifies TX. */
    private val MESHCORE_SERVICE: UUID = UUID.fromString("6e400001-b5a3-f393-e0a9-e50e24dcca9e")
    private val MESHCORE_RX: UUID = UUID.fromString("6e400002-b5a3-f393-e0a9-e50e24dcca9e")
    private val MESHCORE_TX: UUID = UUID.fromString("6e400003-b5a3-f393-e0a9-e50e24dcca9e")
    private val CCCD_UUID: UUID = UUID.fromString("00002902-0000-1000-8000-00805f9b34fb")
    private const val MTU = 512
    private const val TICK_MS = 5_000L
    private const val SCAN_MS = 10_000L
    private const val RETRY_MS = 2_000L
  }
}
