package dev.kemalarslan.carrychat

import android.content.Context
import android.hardware.camera2.CameraCharacteristics
import android.hardware.camera2.CameraManager
import android.os.SystemClock
import android.util.Log

/**
 * The flashlight that sends a message as light (§9.11): it blinks the
 * symbols the node made, `1` lit and `0` dark, over and over until stopped.
 * It runs on a thread of its own, timed from the start against the clock,
 * so a change the camera service makes late does not push back the ones
 * after it. Turning the flashlight on and off needs no permission.
 */
object Torch {
  private const val TAG = "CarryChatTorch"

  @Volatile private var player: Thread? = null

  /** The back camera that has a flash, if any. */
  private fun camera(manager: CameraManager): String? =
    try {
      manager.cameraIdList.firstOrNull { id ->
        val characteristics = manager.getCameraCharacteristics(id)
        characteristics.get(CameraCharacteristics.FLASH_INFO_AVAILABLE) == true &&
          characteristics.get(CameraCharacteristics.LENS_FACING) == CameraCharacteristics.LENS_FACING_BACK
      }
    } catch (error: Exception) {
      null
    }

  fun available(context: Context): Boolean =
    context.getSystemService(CameraManager::class.java)?.let(::camera) != null

  /** Whether it is blinking: it stops by itself if the camera is taken, as by a page watching for light. */
  fun playing(): Boolean = player?.isAlive == true

  /** Starts blinking `pattern`, each symbol for `symbolMs`. False with no flashlight. */
  @Synchronized
  fun play(context: Context, pattern: String, symbolMs: Int): Boolean {
    stop()
    val manager = context.getSystemService(CameraManager::class.java) ?: return false
    val id = camera(manager) ?: return false
    if (pattern.isEmpty() || pattern.any { it != '0' && it != '1' } || symbolMs <= 0) return false
    val thread = Thread({ blink(manager, id, pattern, symbolMs.toLong()) }, "carrychat-torch")
    thread.priority = Thread.MAX_PRIORITY
    player = thread
    thread.start()
    return true
  }

  @Synchronized
  fun stop() {
    val thread = player ?: return
    player = null
    thread.interrupt()
  }

  private fun blink(manager: CameraManager, id: String, pattern: String, symbolMs: Long) {
    val start = SystemClock.elapsedRealtime()
    var lit: Boolean? = null
    var symbol = 0L
    try {
      while (player === Thread.currentThread()) {
        val want = pattern[(symbol % pattern.length).toInt()] == '1'
        if (want != lit) {
          manager.setTorchMode(id, want)
          lit = want
        }
        symbol += 1
        val wait = start + symbol * symbolMs - SystemClock.elapsedRealtime()
        if (wait > 0) Thread.sleep(wait)
      }
    } catch (_: InterruptedException) {
      // Stopped.
    } catch (error: Exception) {
      Log.w(TAG, "The flashlight stopped", error)
    } finally {
      runCatching { manager.setTorchMode(id, false) }
    }
  }
}
