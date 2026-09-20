package io.github.leonfox28.zterm

import android.content.Context
import android.content.pm.PackageInfo
import android.content.pm.PackageManager
import android.os.Build
import io.github.leonfox28.zterm.nativebridge.*
import kotlinx.coroutines.*
import org.json.JSONObject
import java.io.ByteArrayOutputStream
import java.io.File
import java.io.IOException
import java.io.InputStream
import java.net.HttpURLConnection
import java.net.URI
import java.net.URL
import java.security.MessageDigest
import java.util.concurrent.atomic.AtomicReference

internal class AndroidUpdateSource(context: Context) : UpdateSource {
    private val context = context.applicationContext
    private class Candidate(val native: NativeUpdate) : UpdateCandidate {
        val info = native.info()
        override val details = UpdateDetails(info.version, info.versionCode.toLong(), info.length.toLong())
        override fun close() = native.close()
    }

    override suspend fun check(): UpdateCandidate? {
        var owned: NativeUpdate? = null
        try {
            val candidate = withContext(Dispatchers.IO) {
                val installed = packageInfo(context.packageName)
                validateAndroidUpdateBuild(context.packageName, certificate(installed))
                val discovery = JSONObject(bytes("https://api.github.com/repos/leonfox28/zterm/releases/latest", 256 * 1024).toString(Charsets.UTF_8))
                if (discovery.getBoolean("draft") || discovery.getBoolean("prerelease")) throw UpdateFailure("update_invalid")
                val tag = discovery.getString("tag_name")
                validateAndroidUpdateTag(tag)
                val base = "https://github.com/leonfox28/zterm/releases/download/" + tag + "/"
                val manifest = bytes(base + "zterm-release.json", 64 * 1024)
                val signature = bytes(base + "zterm-release.json.sig", 64)
                val checksums = bytes(base + "SHA256SUMS", 64 * 1024)
                val checksumsSignature = bytes(base + "SHA256SUMS.sig", 64)
                val metadata = bytes(base + "zterm-android.json", 64 * 1024)
                val verified = verifyAndroidUpdate(tag, manifest, signature, checksums, checksumsSignature, metadata)
                owned = verified
                if (verified.info().minSdk.toInt() > Build.VERSION.SDK_INT) throw UpdateFailure("update_unsupported")
                if (!verified.isNewerThan(BuildConfig.VERSION_NAME, BuildConfig.VERSION_CODE.toUInt())) {
                    return@withContext null
                }
                Candidate(verified)
            }
            if (candidate != null) owned = null // transfer only after a successful dispatcher return
            return candidate
        } catch (error: NativeException.RequestFailed) { throw UpdateFailure(error.code) }
        catch (error: org.json.JSONException) { throw UpdateFailure("update_invalid") }
        finally { owned?.close() }
    }

    override suspend fun download(candidate: UpdateCandidate, file: File, progress: (Long) -> Unit) {
        val selected = candidate as Candidate
        val expected = selected.details.length
        if (expected !in 1..128 * 1024 * 1024L) throw UpdateFailure("update_invalid")
        withContext(Dispatchers.IO) {
            if (file.parentFile?.usableSpace?.let { it < expected + 1024 * 1024L } != false) throw UpdateFailure("update_storage")
        }
        read(selected.info.url, expected) { input ->
            val output = try { file.outputStream() } catch (_: IOException) { throw UpdateFailure("update_storage") }
            output.use {
                val buffer = ByteArray(64 * 1024)
                var total = 0L
                var lastProgress = -1L
                while (true) {
                    currentCoroutineContext().ensureActive()
                    val size = input.read(buffer)
                    if (size < 0) break
                    total += size
                    if (total > expected) throw UpdateFailure("update_invalid")
                    try { output.write(buffer, 0, size) } catch (_: IOException) { throw UpdateFailure("update_storage") }
                    // At most 101 progress callbacks, independent of chunk/network frequency.
                    val percent = total * 100 / expected
                    if (percent != lastProgress) { progress(total); lastProgress = percent }
                }
                if (total != expected) throw UpdateFailure("update_invalid")
                output.fd.sync()
            }
        }
        withContext(Dispatchers.IO) {
            try { selected.native.verifyApk(file.absolutePath) }
            catch (_: NativeException) { throw UpdateFailure("update_invalid") }
            val archive = archiveInfo(file) ?: throw UpdateFailure("update_invalid")
            if (archive.packageName != context.packageName || archive.packageName != selected.info.packageNameCompat()
                || archive.versionName != selected.info.version || versionCode(archive) != selected.details.versionCode
                || archive.applicationInfo?.minSdkVersion != selected.info.minSdk.toInt()
                || certificate(archive) != selected.info.certificateSha256
                || certificate(packageInfo(context.packageName)) != selected.info.certificateSha256
            ) throw UpdateFailure("update_invalid")
        }
    }

    // Kotlin escapes the generated field whose Rust name is a language keyword.
    private fun NativeUpdateInfo.packageNameCompat() = this.`package`

    @Suppress("DEPRECATION")
    private fun packageInfo(name: String): PackageInfo =
        context.packageManager.getPackageInfo(name, signatureFlags())

    @Suppress("DEPRECATION")
    private fun archiveInfo(file: File): PackageInfo? =
        context.packageManager.getPackageArchiveInfo(file.absolutePath, signatureFlags())

    @Suppress("DEPRECATION") // API 26–27 has no signingInfo alternative.
    private fun signatureFlags() = if (Build.VERSION.SDK_INT >= 28) PackageManager.GET_SIGNING_CERTIFICATES else PackageManager.GET_SIGNATURES

    @Suppress("DEPRECATION")
    private fun certificate(info: PackageInfo): String {
        val signatures = if (Build.VERSION.SDK_INT >= 28) info.signingInfo?.apkContentsSigners else info.signatures
        val signature = signatures?.singleOrNull() ?: throw UpdateFailure("update_unsupported")
        return MessageDigest.getInstance("SHA-256").digest(signature.toByteArray()).joinToString("") { "%02x".format(it) }
    }

    @Suppress("DEPRECATION")
    private fun versionCode(info: PackageInfo): Long = if (Build.VERSION.SDK_INT >= 28) info.longVersionCode else info.versionCode.toLong()

    private suspend fun bytes(url: String, maximum: Int): ByteArray = read(url, maximum.toLong()) { input ->
        val output = ByteArrayOutputStream()
        val buffer = ByteArray(8192)
        while (true) {
            currentCoroutineContext().ensureActive()
            val n = input.read(buffer)
            if (n < 0) break
            if (output.size() + n > maximum) throw UpdateFailure("update_invalid")
            output.write(buffer, 0, n)
        }
        output.toByteArray()
    }

    /** HTTPS redirects only; a cancellation watcher disconnects a blocked socket. */
    private suspend fun <T> read(url: String, maximum: Long, consume: suspend (InputStream) -> T): T = coroutineScope {
        val connection = AtomicReference<HttpURLConnection?>()
        val cancellation = launch(Dispatchers.IO, start = CoroutineStart.UNDISPATCHED) {
            try { awaitCancellation() } finally { connection.getAndSet(null)?.disconnect() }
        }
        try {
            withContext(Dispatchers.IO) {
                var next = URI(url)
                repeat(6) { redirects ->
                    ensureActive()
                    if (next.scheme != "https" || next.userInfo != null) throw UpdateFailure("update_invalid")
                    val http = URL(next.toASCIIString()).openConnection() as HttpURLConnection
                    connection.set(http)
                    try {
                        ensureActive()
                        http.instanceFollowRedirects = false
                        http.connectTimeout = 10_000
                        http.readTimeout = 20_000
                        http.setRequestProperty("User-Agent", "zterm-android/" + BuildConfig.VERSION_NAME)
                        http.setRequestProperty("Accept-Encoding", "identity")
                        http.setRequestProperty("Accept", if (next.host == "api.github.com") "application/vnd.github+json" else "application/octet-stream")
                        val status = http.responseCode
                        if (status in setOf(301, 302, 303, 307, 308)) {
                            if (redirects == 5) throw UpdateFailure("update_network")
                            next = next.resolve(http.getHeaderField("Location") ?: throw UpdateFailure("update_network"))
                        } else {
                            if (status != 200) throw UpdateFailure(if (status == 403 || status == 429) "update_rate_limit" else "update_network")
                            if (http.contentLengthLong > maximum) throw UpdateFailure("update_invalid")
                            return@withContext http.inputStream.use { consume(it) }
                        }
                    } finally { connection.compareAndSet(http, null); http.disconnect() }
                }
                throw UpdateFailure("update_network")
            }
        } finally { cancellation.cancel() }
    }
}
