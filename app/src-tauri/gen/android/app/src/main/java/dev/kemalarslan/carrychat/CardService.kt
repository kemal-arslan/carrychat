package dev.kemalarslan.carrychat

import android.nfc.Tag
import android.nfc.cardemulation.HostApduService
import android.nfc.tech.IsoDep
import android.os.Bundle
import android.util.Base64

/**
 * Cards over NFC (§3.3). While the card sheet is open, this phone answers as
 * a contactless card does: a SELECT for CarryChat's AID gets the card back.
 * Another phone, its add-contact sheet open, reads it with `read`. Android
 * routes the AID here only while the screen is unlocked, and the phone offers
 * nothing while the sheet is closed.
 */
class CardService : HostApduService() {
  override fun processCommandApdu(command: ByteArray, extras: Bundle?): ByteArray {
    if (!isSelect(command)) return UNKNOWN
    val card = offered ?: return NOT_FOUND
    return card + OK
  }

  override fun onDeactivated(reason: Int) {}

  companion object {
    /** CarryChat's AID: `F0`, which marks a proprietary one, then "CARRYCH". */
    private val AID = byteArrayOf(0xF0.toByte(), 0x43, 0x41, 0x52, 0x52, 0x59, 0x43, 0x48)
    private val OK = byteArrayOf(0x90.toByte(), 0x00)
    private val NOT_FOUND = byteArrayOf(0x6A, 0x82.toByte())
    private val UNKNOWN = byteArrayOf(0x6D, 0x00)
    private const val PREFIX = "carrychat:"
    private const val BASE64URL = Base64.URL_SAFE or Base64.NO_PADDING or Base64.NO_WRAP
    private const val READ_TIMEOUT_MS = 3_000

    @Volatile private var offered: ByteArray? = null

    /** Offers `card`, in its `carrychat:` text form, or nothing. */
    fun offer(card: String?) {
      offered = card?.trim()?.removePrefix(PREFIX)?.let { runCatching { Base64.decode(it, BASE64URL) }.getOrNull() }
    }

    /** Reads the card a phone offers, in its text form, or null. */
    fun read(tag: Tag): String? {
      val isoDep = IsoDep.get(tag) ?: return null
      return try {
        isoDep.connect()
        isoDep.timeout = READ_TIMEOUT_MS
        val select = byteArrayOf(0x00, 0xA4.toByte(), 0x04, 0x00, AID.size.toByte()) + AID + byteArrayOf(0x00)
        val reply = isoDep.transceive(select)
        val status = reply.takeLast(2).toByteArray()
        if (reply.size <= 2 || !status.contentEquals(OK)) null
        else PREFIX + Base64.encodeToString(reply.copyOf(reply.size - 2), BASE64URL)
      } catch (_: Exception) {
        null
      } finally {
        runCatching { isoDep.close() }
      }
    }

    private fun isSelect(command: ByteArray): Boolean =
      command.size >= 5 + AID.size &&
        command[0] == 0x00.toByte() &&
        command[1] == 0xA4.toByte() &&
        command[2] == 0x04.toByte() &&
        command[4] == AID.size.toByte() &&
        command.copyOfRange(5, 5 + AID.size).contentEquals(AID)
  }
}
