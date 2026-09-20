package io.github.leonfox28.zterm

import kotlinx.coroutines.*
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.asStateFlow
import java.io.File
import java.util.UUID

internal data class UpdateDetails(val version: String, val versionCode: Long, val length: Long)
internal interface UpdateCandidate : AutoCloseable { val details: UpdateDetails }
internal interface UpdateSource {
    suspend fun check(): UpdateCandidate?
    suspend fun download(candidate: UpdateCandidate, file: File, progress: (Long) -> Unit)
}
internal class UpdateFailure(val code: String) : Exception(code)
internal enum class UpdatePhase { Idle, Checking, Available, Downloading, Ready, Permission, HandedOff }
internal data class UpdateNotice(val id: Long, val code: String)
internal data class UpdateState(
    val phase: UpdatePhase = UpdatePhase.Idle,
    val details: UpdateDetails? = null,
    val manual: Boolean = false,
    val downloaded: Long = 0,
    val notice: UpdateNotice? = null,
)

internal const val UPDATE_REMINDER_MILLIS = 24 * 60 * 60 * 1000L
internal data class UpdateReminder(val version: String, val dismissedAt: Long) {
    fun suppresses(candidate: String, now: Long): Boolean =
        version == candidate && dismissedAt > 0 && now >= dismissedAt && now - dismissedAt < UPDATE_REMINDER_MILLIS
}

/** Main-confined application owner. Activities observe; they never own a download job. */
internal class AppUpdates(
    private val source: UpdateSource,
    private val directory: File,
    private val readReminder: () -> UpdateReminder?,
    private val saveReminder: suspend (UpdateReminder) -> Unit,
    private val scope: CoroutineScope = CoroutineScope(SupervisorJob() + Dispatchers.Main.immediate),
    private val now: () -> Long = System::currentTimeMillis,
) {
    private val mutable = MutableStateFlow(UpdateState())
    val state = mutable.asStateFlow()
    private var startupHandled = false
    private var generation = 0L
    private var job: Job? = null
    private var candidate: UpdateCandidate? = null
    private var file: File? = null
    private var dismissed: UpdateReminder? = null
    private var noticeId = 0L

    fun startup() {
        if (startupHandled) return
        check(manual = false)
    }

    fun check(manual: Boolean = true) {
        startupHandled = true
        when (mutable.value.phase) {
            UpdatePhase.Checking -> {
                if (manual) mutable.value = mutable.value.copy(manual = true)
                return
            }
            UpdatePhase.Idle -> Unit
            else -> return
        }
        val mine = ++generation
        mutable.value = UpdateState(phase = UpdatePhase.Checking, manual = manual)
        job = scope.launch {
            var found: UpdateCandidate? = null
            try {
                withTimeout(60_000) { found = source.check() }
                if (mine != generation) return@launch
                val requested = mutable.value.manual
                val reminder = dismissed ?: readReminder()
                val verified = found
                when {
                    verified == null -> {
                        mutable.value = UpdateState()
                        if (requested) notice("update_latest")
                    }
                    !requested && reminder?.suppresses(verified.details.version, now()) == true ->
                        mutable.value = UpdateState()
                    else -> {
                        candidate = verified
                        mutable.value = UpdateState(UpdatePhase.Available, verified.details, requested)
                        found = null // ownership transferred to this operation
                    }
                }
            } catch (timeout: TimeoutCancellationException) {
                if (mine == generation) checkFailed("update_network")
            } catch (cancel: CancellationException) { throw cancel }
            catch (error: Exception) {
                if (mine == generation) checkFailed((error as? UpdateFailure)?.code ?: "update_network")
            } finally { found?.close() }
        }
    }

    private fun checkFailed(code: String) {
        val requested = mutable.value.manual
        mutable.value = UpdateState()
        if (requested) notice(code)
    }

    fun dismiss() {
        if (mutable.value.phase != UpdatePhase.Available) return
        val reminder = UpdateReminder(requireNotNull(candidate).details.version, now())
        dismissed = reminder
        release()
        scope.launch {
            try { saveReminder(reminder) }
            catch (cancel: CancellationException) { throw cancel }
            catch (_: Exception) { /* Keep this process quiet even when storage is unavailable. */ }
        }
    }

    fun download() {
        if (mutable.value.phase != UpdatePhase.Available) return
        val selected = requireNotNull(candidate)
        val mine = ++generation
        mutable.value = mutable.value.copy(phase = UpdatePhase.Downloading, notice = null)
        job = scope.launch(start = CoroutineStart.UNDISPATCHED) {
            var partial: File? = null
            var ownsCandidate = true
            try {
                val destination = withContext(Dispatchers.IO) {
                    if (!directory.isDirectory && !directory.mkdirs()) throw UpdateFailure("update_storage")
                    // Only old private update files are eligible, never a just-handed-off URI.
                    directory.listFiles()?.filter { it.name.startsWith("update-") && now() - it.lastModified() > UPDATE_REMINDER_MILLIS }
                        ?.forEach { it.delete() }
                    File(directory, "update-" + UUID.randomUUID() + ".apk")
                }
                partial = destination
                withTimeout(10 * 60_000L) {
                    source.download(selected, destination) { bytes ->
                        scope.launch {
                            if (mine == generation && mutable.value.phase == UpdatePhase.Downloading)
                                mutable.value = mutable.value.copy(downloaded = bytes)
                        }
                    }
                }
                if (mine == generation) {
                    file = partial
                    partial = null
                    ownsCandidate = false
                    mutable.value = mutable.value.copy(phase = UpdatePhase.Ready)
                }
            } catch (timeout: TimeoutCancellationException) {
                if (mine == generation) { ownsCandidate = false; fail("update_network") }
            } catch (cancel: CancellationException) { throw cancel }
            catch (error: Exception) {
                if (mine == generation) { ownsCandidate = false; fail((error as? UpdateFailure)?.code ?: "update_download_failed") }
            } finally {
                withContext(NonCancellable + Dispatchers.IO) {
                    try { partial?.delete() }
                    finally { if (ownsCandidate) selected.close() }
                }
            }
        }
    }

    fun cancel() {
        if (mutable.value.phase !in setOf(UpdatePhase.Downloading, UpdatePhase.Ready, UpdatePhase.Permission)) return
        val downloading = mutable.value.phase == UpdatePhase.Downloading
        ++generation
        job?.cancel()
        if (!downloading) candidate?.close() // downloading's finally owns its handle
        candidate = null
        val abandoned = file
        file = null
        mutable.value = UpdateState()
        if (abandoned != null) scope.launch(Dispatchers.IO) { abandoned.delete() }
    }

    fun requireInstallPermission() {
        if (mutable.value.phase == UpdatePhase.Ready) mutable.value = mutable.value.copy(phase = UpdatePhase.Permission)
    }

    fun permissionReturned(granted: Boolean) {
        if (granted && mutable.value.phase == UpdatePhase.Permission)
            mutable.value = mutable.value.copy(phase = UpdatePhase.Ready)
    }

    /** Consume before launching Android so recreation cannot launch twice. */
    fun takeInstallFile(): File? {
        if (mutable.value.phase != UpdatePhase.Ready) return null
        mutable.value = mutable.value.copy(phase = UpdatePhase.HandedOff)
        return file
    }

    fun installationReturned() {
        if (mutable.value.phase == UpdatePhase.HandedOff) release()
    }

    fun fail(code: String) { release(); notice(code) }

    fun consumeNotice(id: Long): Boolean {
        if (mutable.value.notice?.id != id) return false
        mutable.value = mutable.value.copy(notice = null)
        return true
    }

    private fun notice(code: String) { mutable.value = mutable.value.copy(notice = UpdateNotice(++noticeId, code)) }
    private fun release() {
        candidate?.close()
        candidate = null
        // Retain handed-off files for the installer; stale cache cleanup owns deletion.
        file = null
        mutable.value = UpdateState()
    }
}
