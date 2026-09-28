package io.github.leonfox28.zterm

import android.content.Context
import android.net.Uri
import android.system.ErrnoException
import android.system.Os
import android.system.OsConstants
import android.util.AtomicFile
import io.github.leonfox28.zterm.nativebridge.*
import java.io.ByteArrayOutputStream
import java.io.File
import java.io.FileInputStream
import java.io.FileOutputStream
import java.io.OutputStream
import kotlinx.coroutines.*
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.asStateFlow
import org.json.JSONObject

internal data class DiagnosticsState(val remainingSeconds: Long = 0, val busy: Boolean = false, val failed: Boolean = false)

/** Application-owned private persistence. JNI callbacks never call back into native code. */
internal class DiagnosticFiles private constructor(directory: () -> File, private val limit: Long = 2L * 1024 * 1024) : DiagnosticSink {
    constructor(context: Context) : this({ File(context.noBackupFilesDir, "diagnostics") })
    constructor(directory: File, limit: Long = 2L * 1024 * 1024) : this({ directory }, limit)
    // Context.getNoBackupFilesDir can create directories. Resolve it only when
    // the native worker or serial IO dispatcher actually accesses storage.
    private val directory by lazy(directory)
    private val lock = Any()
    private val names = listOf("events.jsonl", "events.jsonl.1", "detail.jsonl", "detail.jsonl.1")
    private fun prepare() {
        if (!directory.exists()) {
            if (!directory.mkdir() && !directory.isDirectory) error("diagnostics directory unavailable")
            Os.chmod(directory.path, 0x1c0) // 0700
        }
        validate(directory, true)
    }
    private fun exists(file: File): Boolean = try { Os.lstat(file.path); true } catch (error: ErrnoException) {
        if (error.errno == OsConstants.ENOENT) false else throw error
    }
    private fun validate(file: File, directory: Boolean = false) {
        val stat = Os.lstat(file.path)
        check(stat.st_uid == android.os.Process.myUid())
        check(stat.st_mode and 0x3f == 0) // no group/other permissions
        check(if (directory) OsConstants.S_ISDIR(stat.st_mode) else OsConstants.S_ISREG(stat.st_mode))
    }
    private fun open(file: File, write: Boolean): java.io.FileDescriptor {
        if (exists(file)) validate(file)
        val flags = OsConstants.O_NOFOLLOW or if (write) OsConstants.O_WRONLY or OsConstants.O_APPEND or OsConstants.O_CREAT else OsConstants.O_RDONLY
        return Os.open(file.path, flags, 0x180) // 0600
    }
    private fun atomicWrite(file: File, bytes: ByteArray) {
        listOf(file, File(file.path + ".new"), File(file.path + ".bak")).forEach { if (exists(it)) validate(it) }
        val atomic = AtomicFile(file)
        val stream = atomic.startWrite()
        try {
            Os.fchmod(stream.fd, 0x180)
            stream.write(bytes)
            atomic.finishWrite(stream)
        } catch (error: Exception) { atomic.failWrite(stream); throw error }
    }
    private fun normalize(file: File) {
        if (!exists(file)) return
        val tail = FileInputStream(open(file, false)).use { input ->
            val length = input.channel.size()
            if (length <= limit) return
            input.channel.position(length - limit)
            val bytes = ByteArray(limit.toInt())
            var used = 0
            while (used < bytes.size) { val read = input.read(bytes, used, bytes.size - used); if (read < 0) break; used += read }
            bytes.copyOf(used)
        }
        val start = tail.indexOf(10.toByte()).let { if (it < 0) tail.size else it + 1 }
        val end = maxOf(start, tail.lastIndexOf(10.toByte()) + 1)
        atomicWrite(file, tail.copyOfRange(start, end))
    }
    override fun append(detail: Boolean, record: String): Boolean = synchronized(lock) {
        runCatching {
            val bytes = record.toByteArray(Charsets.UTF_8)
            check(bytes.size <= 4096 && bytes.lastOrNull() == 10.toByte())
            prepare()
            names.forEach { normalize(File(directory, it)) }
            val name = if (detail) "detail.jsonl" else "events.jsonl"
            val current = File(directory, name)
            val archive = File(directory, "$name.1")
            if (exists(current)) {
                validate(current)
                if (current.length() + bytes.size > limit) {
                    if (exists(archive)) { validate(archive); check(archive.delete()) }
                    check(current.renameTo(archive))
                }
            }
            FileOutputStream(open(current, true)).use { it.write(bytes) }
        }.isSuccess
    }
    override fun control(): DiagnosticControl = synchronized(lock) {
        runCatching {
            if (!exists(directory)) return@synchronized DiagnosticControl(1u, 0, 0)
            validate(directory, true)
            val file = File(directory, "control.json")
            if (!exists(file)) return@synchronized DiagnosticControl(1u, 0, 0)
            validate(file)
            val bytes = ByteArray(1025)
            val count = FileInputStream(open(file, false)).use { input ->
                var count = 0
                while (count < bytes.size) { val read = input.read(bytes, count, bytes.size - count); if (read < 0) break; count += read }
                count
            }
            check(count <= 1024)
            val json = JSONObject(String(bytes, 0, count, Charsets.UTF_8))
            check(json.getInt("schema") == 1)
            DiagnosticControl(1u, json.getLong("started_ms"), json.getLong("deadline_ms"))
        }.getOrDefault(DiagnosticControl(1u, 0, 0))
    }
    fun setControl(control: DiagnosticControl) = synchronized(lock) {
        prepare()
        val json = JSONObject().put("schema", control.schema.toInt()).put("started_ms", control.startedMs).put("deadline_ms", control.deadlineMs)
        atomicWrite(File(directory, "control.json"), json.toString().toByteArray(Charsets.UTF_8))
    }

    override fun flush(): Boolean = synchronized(lock) {
        runCatching {
            if (exists(directory)) {
                validate(directory, true)
                names.forEach { name -> val file = File(directory, name); if (exists(file)) FileOutputStream(open(file, true)).use { it.fd.sync() } }
            }
        }.isSuccess
    }
    private data class Snapshot(val input: FileInputStream, val length: Long, val start: Long)
    private fun snapshots(includeDebug: Boolean): List<Snapshot> = synchronized(lock) {
        if (!exists(directory)) return@synchronized emptyList()
        validate(directory, true)
        val result = mutableListOf<Snapshot>()
        try {
            names.take(if (includeDebug) 4 else 2).forEach { name ->
                val file = File(directory, name)
                if (exists(file)) {
                    val input = FileInputStream(open(file, false))
                    val length = input.channel.size()
                    result.add(Snapshot(input, minOf(length, limit), maxOf(0, length - limit)))
                }
            }
            result
        } catch (error: Exception) { result.forEach { it.input.close() }; throw error }
    }
    /** Captures handles under the lock, then releases writers for both bounded passes. */
    fun export(output: OutputStream, includeDebug: Boolean, lost: ULong) {
        val snapshots = snapshots(includeDebug)
        try {
            var omitted = 0uL
            snapshots.forEach { snapshot -> scan(snapshot) { line -> if (line == null || diagnosticExportRecord(line) == null) omitted++ } }
            output.write(diagnosticExportHeader(includeDebug, omitted, snapshots.sumOf { it.start }.toULong(), lost).toByteArray(Charsets.UTF_8))
            snapshots.asReversed().forEach { snapshot -> scan(snapshot) { line ->
                line?.let(::diagnosticExportRecord)?.let { output.write(it.toByteArray(Charsets.UTF_8)) }
            } }
            output.flush()
        } finally { snapshots.forEach { it.input.close() } }
    }
    private fun scan(snapshot: Snapshot, consume: (ByteArray?) -> Unit) {
        snapshot.input.channel.position(snapshot.start)
        var remaining = snapshot.length
        val buffer = ByteArray(8192)
        val line = ByteArrayOutputStream(4096)
        var discard = snapshot.start > 0
        while (remaining > 0) {
            val count = snapshot.input.read(buffer, 0, minOf(remaining, buffer.size.toLong()).toInt())
            if (count < 0) break
            remaining -= count
            for (index in 0 until count) {
                val byte = buffer[index].toInt() and 255
                if (line.size() >= 4096) discard = true
                if (!discard) line.write(byte)
                if (byte == 10) { consume(if (discard) null else line.toByteArray()); line.reset(); discard = false }
            }
        }
        if (line.size() > 0 || discard) consume(null)
    }
}

/** The shared Rust encoder/queue owns admission; this class owns OS controls and export. */
internal class AppDiagnostics(context: Context) {
    private val app = context.applicationContext
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.IO.limitedParallelism(1))
    internal val files = DiagnosticFiles(app)
    internal val native = installDiagnostics(files)
    private val mutable = MutableStateFlow(DiagnosticsState())
    val state = mutable.asStateFlow()
    init {
        record(AppDiagnostic.STARTED)
        scope.launch {
            while (isActive) {
                val remaining = diagnosticRemainingMs(files.control()).toLong().let { (it + 999) / 1000 }
                mutable.value = mutable.value.copy(remainingSeconds = remaining)
                delay(1000)
            }
        }
    }
    fun record(event: AppDiagnostic, category: String? = null, columns: UShort = 0u, rows: UShort = 0u) {
        native.recordApp(event, category, columns, rows)
    }
    fun setEnabled(enabled: Boolean) {
        scope.launch {
            mutable.value = mutable.value.copy(busy = true, failed = false)
            val result = runCatching { files.setControl(diagnosticControl(enabled)) }
            mutable.value = DiagnosticsState((diagnosticRemainingMs(files.control()).toLong() + 999) / 1000, failed = result.isFailure)
        }
    }
    fun export(uri: Uri, includeDebug: Boolean) {
        scope.launch {
            mutable.value = mutable.value.copy(busy = true, failed = false)
            val result = runCatching {
                native.flush()
                app.contentResolver.openOutputStream(uri, "w").let { it ?: error("export unavailable") }.use { files.export(it, includeDebug, native.pendingLost()) }
            }
            record(if (result.isSuccess) AppDiagnostic.EXPORT_COMPLETED else AppDiagnostic.EXPORT_FAILED)
            mutable.value = mutable.value.copy(busy = false, failed = result.isFailure)
        }
    }
}
