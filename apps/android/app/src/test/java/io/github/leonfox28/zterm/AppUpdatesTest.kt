package io.github.leonfox28.zterm

import kotlinx.coroutines.*
import kotlinx.coroutines.flow.first
import org.junit.Assert.*
import org.junit.Rule
import org.junit.Test
import org.junit.rules.TemporaryFolder
import java.io.File

class AppUpdatesTest {
    @get:Rule val temporary = TemporaryFolder()

    private class Candidate(version: String = "0.2.0") : UpdateCandidate {
        override val details = UpdateDetails(version, 2000, 3)
        var closes = 0
        override fun close() { closes++ }
    }
    private class Source : UpdateSource {
        var checks = 0
        var downloads = 0
        var result: suspend () -> UpdateCandidate? = { null }
        var transfer: suspend (File) -> Unit = { it.writeText("apk") }
        override suspend fun check(): UpdateCandidate? { checks++; return result() }
        override suspend fun download(candidate: UpdateCandidate, file: File, progress: (Long) -> Unit) {
            downloads++
            transfer(file)
            progress(candidate.details.length)
        }
    }
    private fun CoroutineScope.owner(source: Source, reminder: UpdateReminder? = null, time: Long = 100_000,
                                     save: suspend (UpdateReminder) -> Unit = {}) =
        AppUpdates(source, temporary.root, { reminder }, save, this, { time })
    private suspend fun AppUpdates.await(phase: UpdatePhase) = state.first { it.phase == phase }
    private fun test(block: suspend CoroutineScope.() -> Unit) = runBlocking {
        withTimeout(10_000) { try { block() } finally { coroutineContext.cancelChildren() } }
    }

    @Test fun coldLaunchChecksOnceAndCurrentOrFailedAutomaticResultsStaySilent() = test {
        for (failure in listOf(false, true)) {
            val source = Source().apply { result = { if (failure) throw UpdateFailure("update_network") else null } }
            val updates = owner(source)
            updates.startup()
            updates.await(UpdatePhase.Idle)
            repeat(3) { updates.startup() } // recreation and foreground return use the same owner
            yield()
            assertEquals(1, source.checks)
            assertNull(updates.state.value.notice)
            updates.check()
            val notice = updates.await(UpdatePhase.Idle).notice!!
            assertEquals(if (failure) "update_network" else "update_latest", notice.code)
            assertTrue(updates.consumeNotice(notice.id))
            assertFalse(updates.consumeNotice(notice.id))
        }
    }

    @Test fun manualRequestPromotesPendingStartupAndDoesNotFetchTwice() = test {
        val response = CompletableDeferred<UpdateCandidate?>()
        val source = Source().apply { result = { response.await() } }
        val updates = owner(source)
        updates.startup()
        repeat(5) { updates.check() }
        yield()
        assertTrue(updates.state.value.manual)
        assertEquals(1, source.checks)
        response.complete(null)
        assertEquals("update_latest", updates.await(UpdatePhase.Idle).notice?.code)
        updates.startup()
        assertEquals(1, source.checks)
    }

    @Test fun laterPersistsWithoutDownloadingAndManualCheckBypassesReminder() = test {
        val candidates = mutableListOf<Candidate>()
        val source = Source().apply { result = { Candidate().also { candidates += it } } }
        var stored: UpdateReminder? = null
        val updates = owner(source, save = { stored = it })
        updates.startup()
        updates.await(UpdatePhase.Available)
        updates.dismiss()
        yield()
        assertEquals(UpdateReminder("0.2.0", 100_000), stored)
        assertEquals(1, candidates.first().closes)
        assertEquals(0, source.downloads)
        updates.check()
        assertTrue(updates.await(UpdatePhase.Available).manual)
        updates.dismiss()
    }

    @Test fun reminderSurvivesRestartExpiresAt24HoursAndDoesNotHideHigherVersions() = test {
        val reminder = UpdateReminder("0.2.0", 100_000)
        for ((version, time, suppressed) in listOf(
            Triple("0.2.0", 100_000 + UPDATE_REMINDER_MILLIS - 1, true),
            Triple("0.2.0", 100_000 + UPDATE_REMINDER_MILLIS, false),
            Triple("0.2.1", 100_001L, false),
            Triple("0.2.0", 99_999L, false), // wall clock moved backwards
        )) {
            val selected = Candidate(version)
            val source = Source().apply { result = { selected } }
            val restarted = owner(source, reminder, time)
            restarted.startup()
            restarted.await(if (suppressed) UpdatePhase.Idle else UpdatePhase.Available)
            assertNull(restarted.state.value.notice)
            if (!suppressed) restarted.dismiss()
            assertEquals(1, selected.closes)
        }
    }

    @Test fun failedReminderSaveStaysSilentAndDoesNotBreakManualChecks() = test {
        val source = Source().apply { result = { Candidate() } }
        val updates = owner(source, save = { throw java.io.IOException("disk full") })
        updates.startup()
        updates.await(UpdatePhase.Available)
        updates.dismiss()
        yield()
        assertEquals(UpdateState(), updates.state.value)
        updates.check()
        updates.await(UpdatePhase.Available)
        updates.dismiss()
    }

    @Test fun cancellationDeletesPartialClosesOnceAndCannotPublishLateCompletion() = test {
        val selected = Candidate()
        val writing = CompletableDeferred<File>()
        val stopped = CompletableDeferred<Unit>()
        val source = Source().apply {
            result = { selected }
            transfer = { file ->
                file.writeText("part")
                writing.complete(file)
                try { awaitCancellation() } finally { stopped.complete(Unit) }
            }
        }
        val updates = owner(source)
        updates.check()
        updates.await(UpdatePhase.Available)
        updates.download()
        updates.download()
        val partial = writing.await()
        updates.cancel()
        assertEquals(UpdateState(), updates.state.value)
        stopped.await()
        while (partial.exists() || selected.closes == 0) delay(5)
        assertEquals(1, selected.closes)
        assertEquals(1, source.downloads)
        assertEquals(UpdateState(), updates.state.value)
    }

    @Test fun immediateCancellationReleasesCandidateEvenBeforeIoBegins() = test {
        val selected = Candidate()
        val source = Source().apply { result = { selected } }
        val updates = owner(source)
        updates.check()
        updates.await(UpdatePhase.Available)
        updates.download()
        updates.cancel()
        while (selected.closes == 0) delay(5)
        assertEquals(1, selected.closes)
        assertEquals(UpdateState(), updates.state.value)
    }

    @Test fun onlyVerifiedDownloadBecomesReadyAndInstallerHandoffIsConsumedOnce() = test {
        val selected = Candidate()
        val source = Source().apply { result = { selected } }
        val updates = owner(source)
        updates.check()
        updates.await(UpdatePhase.Available)
        assertEquals(0, source.downloads)
        updates.download()
        updates.await(UpdatePhase.Ready)
        updates.requireInstallPermission()
        updates.permissionReturned(false)
        assertEquals(UpdatePhase.Permission, updates.state.value.phase)
        assertNull(updates.takeInstallFile())
        updates.permissionReturned(true)
        val file = updates.takeInstallFile()!!
        assertEquals("apk", file.readText())
        assertNull(updates.takeInstallFile())
        updates.installationReturned()
        assertTrue("installer can finish reading retained URI", file.exists())
        assertEquals(1, selected.closes)
        assertEquals(UpdateState(), updates.state.value)
    }

    @Test fun invalidDownloadIsDeletedAndNeverReachesInstaller() = test {
        val selected = Candidate()
        val source = Source().apply {
            result = { selected }
            transfer = { file -> file.writeText("tampered"); throw UpdateFailure("update_invalid") }
        }
        val updates = owner(source)
        updates.check()
        updates.await(UpdatePhase.Available)
        updates.download()
        assertEquals("update_invalid", updates.await(UpdatePhase.Idle).notice?.code)
        assertNull(updates.takeInstallFile())
        while (temporary.root.listFiles()!!.isNotEmpty()) delay(5)
        assertEquals(1, selected.closes)
    }
}
