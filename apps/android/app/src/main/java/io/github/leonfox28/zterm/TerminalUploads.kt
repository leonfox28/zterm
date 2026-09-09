package io.github.leonfox28.zterm

import android.content.Context
import android.net.Uri
import android.provider.OpenableColumns
import io.github.leonfox28.zterm.nativebridge.*
import java.io.File
import kotlinx.coroutines.*
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock

internal enum class UploadPicker { Image, File }
internal data class UploadUi(
    val id: Long,
    val picker: UploadPicker,
    val phase: String = "reserving",
    val pickerLaunched: Boolean = false,
    val filename: String = "",
    val totalBytes: Long? = null,
    val acceptedBytes: Long = 0,
    val bytesPerSecond: Double = 0.0,
    val error: String? = null,
) {
    val active: Boolean get() = phase !in setOf("failed", "cancelled", "completed")
    val showDialog: Boolean get() = phase !in setOf("reserving", "picking", "cancelled", "completed")
}

/** Application-owned selection, bounded URI staging and native operation lifetime. */
internal class TerminalUploads(
    context: Context,
    private val scope: CoroutineScope,
    private val currentTerminal: () -> Pair<NativeTerminal, ULong>?,
) {
    private val resolver = context.contentResolver
    private val cache = context.cacheDir
    private val staging = Mutex()
    private val mutable = MutableStateFlow<UploadUi?>(null)
    val state = mutable.asStateFlow()
    var inputVersion = 0L
        private set
    val paused: Boolean get() = mutable.value?.active == true
    private var sequence = 0L
    private class Attempt(val id: Long, val terminal: NativeTerminal, val epoch: ULong, val picker: UploadPicker) {
        var native: NativeUpload? = null
        var preparation: Job? = null
        var observation: Job? = null
        var file: File? = null
        var uri: Uri? = null
    }
    private var current: Attempt? = null

    fun choose(picker: UploadPicker) { begin(picker, null) }

    private fun begin(picker: UploadPicker, uri: Uri?) {
        if (paused) return
        val target = currentTerminal() ?: return
        retire()
        val attempt = Attempt(++sequence, target.first, target.second, picker)
        current = attempt
        ++inputVersion
        mutable.value = UploadUi(attempt.id, picker)
        attempt.preparation = scope.launch {
            try {
                val native = attempt.terminal.reserveUpload(attempt.epoch)
                if (!owns(attempt)) { native.cancel(); native.close(); return@launch }
                attempt.native = native
                observe(attempt, native)
                if (uri == null) update(attempt) { it.copy(phase = "picking") }
                else stage(attempt, uri)
            } catch (cancel: CancellationException) { throw cancel }
            catch (error: Exception) { fail(attempt, error) }
        }
    }

    fun pickerLaunched(id: Long): Boolean {
        val ui = mutable.value ?: return false
        if (ui.id != id || ui.phase != "picking" || ui.pickerLaunched) return false
        mutable.value = ui.copy(pickerLaunched = true)
        return true
    }

    fun selected(id: Long, uri: Uri?) {
        val attempt = current?.takeIf { it.id == id && owns(it) } ?: return
        if (uri == null) { cancel(); return }
        if (mutable.value?.phase != "picking") return
        update(attempt) { it.copy(phase = "preparing") }
        attempt.preparation = scope.launch {
            try { stage(attempt, uri) }
            catch (cancel: CancellationException) { throw cancel }
            catch (error: Exception) { fail(attempt, error) }
        }
    }

    fun pickerFailed(id: Long) { current?.takeIf { it.id == id }?.let { fail(it, UploadFailure("upload_source_invalid")) } }

    private suspend fun stage(attempt: Attempt, uri: Uri) = staging.withLock {
        attempt.uri = uri
        update(attempt) { it.copy(phase = "preparing") }
        val metadata = withContext(Dispatchers.IO) {
            resolver.query(uri, arrayOf(OpenableColumns.DISPLAY_NAME, OpenableColumns.SIZE), null, null, null)?.use { cursor ->
                if (!cursor.moveToFirst()) return@use "" to null
                val name = cursor.getColumnIndex(OpenableColumns.DISPLAY_NAME)
                val size = cursor.getColumnIndex(OpenableColumns.SIZE)
                (if (name >= 0 && !cursor.isNull(name)) cursor.getString(name) else "") to
                    (if (size >= 0 && !cursor.isNull(size)) cursor.getLong(size).takeIf { it >= 0 } else null)
            } ?: ("" to null)
        }
        val limit = uploadLimitBytes().toLong()
        if (metadata.second?.let { it > limit } == true) throw UploadFailure("upload_too_large")
        val name = metadata.first.filterNot { it.isISOControl() }.take(160)
        update(attempt) { it.copy(filename = name, totalBytes = metadata.second) }
        val size = withContext(Dispatchers.IO) {
            val file = File.createTempFile("zterm-upload-", ".part", cache)
            attempt.file = file
            var copied = 0L
            resolver.openInputStream(uri)?.use { input ->
                file.outputStream().use { output ->
                    val buffer = ByteArray(64 * 1024)
                    while (true) {
                        currentCoroutineContext().ensureActive()
                        val count = input.read(buffer)
                        if (count < 0) break
                        if (copied + count > limit) throw UploadFailure("upload_too_large")
                        output.write(buffer, 0, count)
                        copied += count
                    }
                }
            } ?: throw UploadFailure("upload_source_invalid")
            copied
        }
        if (!owns(attempt)) return@withLock
        update(attempt) { it.copy(totalBytes = size) }
        val extension = metadata.first.substringAfterLast('.', "")
        requireNotNull(attempt.native).start(requireNotNull(attempt.file).absolutePath, size.toULong(), extension)
    }

    private fun observe(attempt: Attempt, native: NativeUpload) {
        attempt.observation = scope.launch {
            // Include an already-published terminal state when this coroutine starts late.
            var generation = 0uL
            var sampleTime = android.os.SystemClock.elapsedRealtime()
            var sampleBytes = 0L
            var speed = 0.0
            try {
                while (owns(attempt)) {
                    val next = native.waitForProgress(generation)
                    generation = next.generation
                    if (!owns(attempt)) break
                    if (mutable.value?.phase == "failed") break
                    val now = android.os.SystemClock.elapsedRealtime()
                    val bytes = next.acceptedBytes.toLong()
                    if (now - sampleTime >= 250) {
                        val rate = (bytes - sampleBytes).coerceAtLeast(0) * 1000.0 / (now - sampleTime)
                        speed = if (sampleBytes == 0L) rate else speed * .65 + rate * .35
                        sampleBytes = bytes; sampleTime = now
                    }
                    if (next.phase in setOf("completed", "cancelled")) { retire(); break }
                    if (next.phase == "failed") {
                        ++inputVersion
                        update(attempt) { it.copy(phase = "failed", error = next.error ?: "upload_source_invalid") }
                        break
                    }
                    if (next.phase == "preparing" && mutable.value?.phase in setOf("reserving", "picking")) continue
                    update(attempt) { it.copy(phase = next.phase, totalBytes = next.totalBytes?.toLong() ?: it.totalBytes, acceptedBytes = bytes, bytesPerSecond = speed) }
                }
            } catch (cancel: CancellationException) { throw cancel }
            catch (error: Exception) { fail(attempt, error) }
        }
    }

    private fun owns(attempt: Attempt): Boolean {
        val target = currentTerminal()
        return current === attempt && target != null && target.first === attempt.terminal && target.second == attempt.epoch
    }
    private inline fun update(attempt: Attempt, transform: (UploadUi) -> UploadUi) {
        if (current === attempt) mutable.value?.let { mutable.value = transform(it) }
    }
    private fun fail(attempt: Attempt, error: Exception) {
        if (current !== attempt) return
        attempt.native?.cancel()
        ++inputVersion
        val code = when (error) { is UploadFailure -> error.code; is NativeException.RequestFailed -> error.code; else -> "upload_source_invalid" }
        update(attempt) { it.copy(phase = "failed", error = code) }
    }
    fun authorityChanged() { current?.let { if (!owns(it)) retire() } }
    fun cancel() { retire() }
    fun retry() {
        val attempt = current ?: return
        if (paused) return
        begin(attempt.picker, attempt.uri)
    }
    fun retire() {
        val attempt = current
        current = null
        mutable.value = null
        ++inputVersion
        if (attempt == null) return
        attempt.native?.cancel()
        attempt.preparation?.cancel()
        attempt.observation?.cancel()
        scope.launch {
            attempt.preparation?.join()
            attempt.observation?.join()
            attempt.native?.close()
            withContext(Dispatchers.IO) { attempt.file?.delete() }
        }
    }
}
private class UploadFailure(val code: String) : Exception()
