package io.github.leonfox28.zterm

import android.content.Context
import android.security.keystore.KeyGenParameterSpec
import android.security.keystore.KeyProperties
import android.util.AtomicFile
import android.util.Base64
import io.github.leonfox28.zterm.nativebridge.NativeHost
import org.json.JSONArray
import org.json.JSONObject
import java.io.File
import java.security.KeyStore
import java.security.SecureRandom
import javax.crypto.Cipher
import javax.crypto.KeyGenerator
import javax.crypto.SecretKey
import javax.crypto.spec.GCMParameterSpec

internal data class SavedHost(
    val id: String,
    val name: String,
    val relays: List<String>,
    val lastSession: String? = null,
) {
    fun native() = NativeHost(id, name, relays)
}
internal data class Preferences(val language: String = "system", val theme: String = "system", val fontSize: Int = 12)
internal val terminalFontSizes = 8..16
internal data class RecentConnection(val host: String, val session: String)
internal data class SavedState(
    val hosts: List<SavedHost> = emptyList(),
    val preferences: Preferences = Preferences(),
    val recent: RecentConnection? = null,
)
internal class StoreFailure(val code: String) : Exception(code)

/** Atomic, no-backup address book and a seed wrapped by a non-exportable Keystore key. */
internal class AppStore(
    context: Context,
    private val directory: File = File(context.noBackupFilesDir, "zterm"),
    private val keyAlias: String = "zterm.identity.v1",
) {
    private val identity get() = AtomicFile(File(directory, "identity.json"))
    private val stateFile get() = AtomicFile(File(directory, "state.json"))

    @Synchronized fun loadOrCreateSeed(): ByteArray {
        try {
            ensureDirectory()
            val vault = KeyStore.getInstance("AndroidKeyStore").apply { load(null) }
            if (identity.baseFile.exists() || File(directory, "identity.json.bak").exists()) {
                val document = JSONObject(readBounded(identity, 2048).toString(Charsets.UTF_8))
                if (document.getInt("version") != 1) throw StoreFailure("identity_state_mismatch")
                val key = vault.getKey(keyAlias, null) as? SecretKey ?: throw StoreFailure("identity_state_mismatch")
                val iv = Base64.decode(document.getString("iv"), Base64.NO_WRAP)
                val encrypted = Base64.decode(document.getString("ciphertext"), Base64.NO_WRAP)
                if (iv.size != 12 || encrypted.size != 48) throw StoreFailure("identity_state_mismatch")
                val plaintext = Cipher.getInstance("AES/GCM/NoPadding").run {
                    init(Cipher.DECRYPT_MODE, key, GCMParameterSpec(128, iv))
                    updateAAD(IDENTITY_AAD)
                    doFinal(encrypted)
                }
                if (plaintext.size != 32) { plaintext.fill(0); throw StoreFailure("identity_state_mismatch") }
                return plaintext
            }
            // Existing host records without their identity must never silently acquire a new key.
            if (stateFile.baseFile.exists() || File(directory, "state.json.bak").exists()) {
                throw StoreFailure("identity_state_mismatch")
            }
            val key = (vault.getKey(keyAlias, null) as? SecretKey) ?: KeyGenerator.getInstance(
                KeyProperties.KEY_ALGORITHM_AES, "AndroidKeyStore",
            ).run {
                init(KeyGenParameterSpec.Builder(keyAlias, KeyProperties.PURPOSE_ENCRYPT or KeyProperties.PURPOSE_DECRYPT)
                    .setKeySize(256).setBlockModes(KeyProperties.BLOCK_MODE_GCM)
                    .setEncryptionPaddings(KeyProperties.ENCRYPTION_PADDING_NONE)
                    .setRandomizedEncryptionRequired(true).build())
                generateKey()
            }
            val seed = ByteArray(32).also { SecureRandom().nextBytes(it) }
            try {
                val cipher = Cipher.getInstance("AES/GCM/NoPadding").apply {
                    init(Cipher.ENCRYPT_MODE, key)
                    updateAAD(IDENTITY_AAD)
                }
                val encrypted = cipher.doFinal(seed)
                val document = JSONObject().put("version", 1)
                    .put("iv", Base64.encodeToString(cipher.iv, Base64.NO_WRAP))
                    .put("ciphertext", Base64.encodeToString(encrypted, Base64.NO_WRAP))
                writeAtomic(identity, document.toString().toByteArray(Charsets.UTF_8))
                return seed
            } catch (error: Exception) {
                seed.fill(0)
                throw error
            }
        } catch (error: StoreFailure) { throw error }
        catch (_: Exception) { throw StoreFailure("identity_state_mismatch") }
    }

    @Synchronized fun load(): SavedState {
        if (!stateFile.baseFile.exists() && !File(directory, "state.json.bak").exists()) return SavedState()
        try {
            val root = JSONObject(readBounded(stateFile, 256 * 1024).toString(Charsets.UTF_8))
            if (root.getInt("version") != 1) throw StoreFailure("storage_unavailable")
            val hostsArray = root.getJSONArray("hosts")
            if (hostsArray.length() > 32) throw StoreFailure("storage_unavailable")
            val hosts = List(hostsArray.length()) { index ->
                val host = hostsArray.getJSONObject(index)
                val id = host.getString("id")
                val name = host.getString("name")
                val routes = host.getJSONArray("relays")
                if (!DEVICE_ID.matches(id) || name.isBlank() || name.toByteArray().size > 1024 || routes.length() > 4) throw StoreFailure("storage_unavailable")
                val last = host.optionalString("lastSession")
                if (last != null && !SESSION_ID.matches(last)) throw StoreFailure("storage_unavailable")
                SavedHost(id, name, List(routes.length()) { routes.getString(it) }, last)
            }
            if (hosts.map { it.id }.toSet().size != hosts.size) throw StoreFailure("storage_unavailable")
            val settings = root.getJSONObject("preferences")
            val language = settings.getString("language").takeIf { it in setOf("system", "zh", "en") } ?: "system"
            val theme = settings.getString("theme").takeIf { it in setOf("system", "dark", "light") } ?: "system"
            val font = settings.getInt("fontSize").takeIf { it in terminalFontSizes } ?: 12
            val recent = root.optJSONObject("recent")?.let {
                val host = it.getString("host")
                val session = it.getString("session")
                if (hosts.none { saved -> saved.id == host } || !SESSION_ID.matches(session)) throw StoreFailure("storage_unavailable")
                RecentConnection(host, session)
            }
            return SavedState(hosts, Preferences(language, theme, font), recent)
        } catch (error: StoreFailure) { throw error }
        catch (_: Exception) { throw StoreFailure("storage_unavailable") }
    }

    @Synchronized fun save(state: SavedState) {
        try {
            ensureDirectory()
            if (state.hosts.size > 32 || state.hosts.map { it.id }.toSet().size != state.hosts.size) throw StoreFailure("storage_unavailable")
            val hosts = JSONArray()
            state.hosts.forEach { host ->
                hosts.put(JSONObject().put("id", host.id).put("name", host.name)
                    .put("relays", JSONArray(host.relays)).put("lastSession", host.lastSession ?: JSONObject.NULL))
            }
            val settings = JSONObject().put("language", state.preferences.language)
                .put("theme", state.preferences.theme).put("fontSize", state.preferences.fontSize)
            val recent = state.recent?.let { JSONObject().put("host", it.host).put("session", it.session) }
            val root = JSONObject().put("version", 1).put("hosts", hosts)
                .put("preferences", settings).put("recent", recent ?: JSONObject.NULL)
            val bytes = root.toString().toByteArray(Charsets.UTF_8)
            if (bytes.size > 256 * 1024) throw StoreFailure("storage_unavailable")
            writeAtomic(stateFile, bytes)
        } catch (_: Exception) { throw StoreFailure("storage_unavailable") }
    }

    private fun ensureDirectory() {
        if (!directory.isDirectory && !directory.mkdirs()) throw StoreFailure("storage_unavailable")
    }
    private fun readBounded(file: AtomicFile, maximum: Int): ByteArray = file.openRead().use { input ->
        if (input.channel.size() > maximum) throw StoreFailure("storage_unavailable")
        input.readBytes()
    }
    private fun writeAtomic(file: AtomicFile, bytes: ByteArray) {
        val output = file.startWrite()
        try { output.write(bytes); file.finishWrite(output) }
        catch (error: Exception) { file.failWrite(output); throw error }
    }
    private fun JSONObject.optionalString(key: String): String? = if (isNull(key) || !has(key)) null else getString(key)
    companion object {
        private val IDENTITY_AAD = "zterm-android-identity-v1".toByteArray(Charsets.US_ASCII)
        private val DEVICE_ID = Regex("[0-9a-f]{64}")
        private val SESSION_ID = Regex("[0-9a-f]{32}")
    }
}
