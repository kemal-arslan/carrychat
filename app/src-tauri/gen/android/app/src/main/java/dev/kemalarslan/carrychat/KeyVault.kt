package dev.kemalarslan.carrychat

import android.content.Context
import android.security.keystore.KeyGenParameterSpec
import android.security.keystore.KeyProperties
import android.util.Log
import java.io.File
import java.security.KeyStore
import java.security.SecureRandom
import javax.crypto.Cipher
import javax.crypto.KeyGenerator
import javax.crypto.SecretKey
import javax.crypto.spec.GCMParameterSpec

/**
 * The key CarryChat's data is encrypted with on this phone (§10). It is random, wrapped with a key that Android
 * Keystore holds and never lets out, and kept where backups do not reach, so the data and its wrapped key together
 * still open nothing on another phone.
 */
object KeyVault {
  private const val TAG = "CarryChatKeys"
  private const val KEYSTORE = "AndroidKeyStore"
  private const val ALIAS = "carrychat-data"
  private const val FILE = "carrychat-data.key"
  private const val IV_BYTES = 12
  private const val TAG_BITS = 128

  /** Hands the data key to the node, before the screen or the service opens it. */
  fun provide(context: Context) {
    try {
      Core.setDataKey(dataKey(context))
    } catch (error: Exception) {
      // Without the key the node does not open, and the data stays as it is.
      Log.e(TAG, "The data key could not be read", error)
    }
  }

  /** The data key, made on first use. */
  private fun dataKey(context: Context): ByteArray {
    val file = File(context.noBackupFilesDir, FILE)
    if (file.exists()) return unwrap(file.readBytes())
    val key = ByteArray(32).also { SecureRandom().nextBytes(it) }
    // Written whole under another name, then put in place.
    val new = File(context.noBackupFilesDir, "$FILE.new")
    new.writeBytes(wrap(key))
    if (!new.renameTo(file)) throw IllegalStateException("could not keep the data key")
    return key
  }

  private fun wrap(key: ByteArray): ByteArray {
    val cipher = Cipher.getInstance("AES/GCM/NoPadding")
    cipher.init(Cipher.ENCRYPT_MODE, wrappingKey(create = true))
    return cipher.iv + cipher.doFinal(key)
  }

  private fun unwrap(wrapped: ByteArray): ByteArray {
    val cipher = Cipher.getInstance("AES/GCM/NoPadding")
    cipher.init(Cipher.DECRYPT_MODE, wrappingKey(create = false), GCMParameterSpec(TAG_BITS, wrapped, 0, IV_BYTES))
    return cipher.doFinal(wrapped, IV_BYTES, wrapped.size - IV_BYTES)
  }

  /** The Keystore key the data key is wrapped with. A wrapped key whose wrapping key is gone stays unopened. */
  private fun wrappingKey(create: Boolean): SecretKey {
    val keystore = KeyStore.getInstance(KEYSTORE).apply { load(null) }
    (keystore.getKey(ALIAS, null) as SecretKey?)?.let { return it }
    if (!create) throw IllegalStateException("the key wrapping the data key is gone")
    val generator = KeyGenerator.getInstance(KeyProperties.KEY_ALGORITHM_AES, KEYSTORE)
    generator.init(
      KeyGenParameterSpec.Builder(ALIAS, KeyProperties.PURPOSE_ENCRYPT or KeyProperties.PURPOSE_DECRYPT)
        .setBlockModes(KeyProperties.BLOCK_MODE_GCM)
        .setEncryptionPaddings(KeyProperties.ENCRYPTION_PADDING_NONE)
        .setKeySize(256)
        .build(),
    )
    return generator.generateKey()
  }
}
