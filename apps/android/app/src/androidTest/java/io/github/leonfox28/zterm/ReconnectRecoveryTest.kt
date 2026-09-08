package io.github.leonfox28.zterm

import android.graphics.Bitmap
import androidx.compose.ui.test.*
import androidx.compose.ui.test.junit4.createEmptyComposeRule
import androidx.test.core.app.ActivityScenario
import androidx.test.core.app.ApplicationProvider
import androidx.test.platform.app.InstrumentationRegistry
import io.github.leonfox28.zterm.nativebridge.*
import kotlinx.coroutines.delay
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.withTimeout
import org.junit.Assert.*
import org.junit.Assume.assumeTrue
import org.junit.Rule
import org.junit.Test
import java.io.File

/** Run alone with reconnectFixture=1 and a ticket from a fresh disposable host.
 * With daemonRestart=1, the coordinator restarts ONLY that host after reconnect-ready,
 * then writes reconnect-restarted. Never run this fixture against a user's host.
 */
class ReconnectRecoveryTest {
    @get:Rule val ui = createEmptyComposeRule()
    private val instrumentation = InstrumentationRegistry.getInstrumentation()
    private val app = ApplicationProvider.getApplicationContext<ZtermApplication>()
    private val repository get() = app.repository
    private val runtime get() = app.runtime
    private val viewport = NativeViewport(24u, 80u)

    @Test fun staleRecentAndRetryRecoverDefaultWithoutReplacingLiveSessions() = runBlocking {
        val args = InstrumentationRegistry.getArguments()
        assumeTrue("explicit disposable daemon", args.getString("reconnectFixture") == "1")
        val ticket = File(app.filesDir, "reconnect-ticket.txt")
        val store = AppStore(app)
        val saved = store.load()
        val seed = store.loadOrCreateSeed()
        try { runtime.initialize(seed, PlatformNetwork(app) {}.current(), saved.hosts.map { it.native() }) }
        finally { seed.fill(0) }
        val host = runtime.pairTicket(ticket.readText().trim()).use { pairing ->
            val value = pairing.host()
            assertTrue("test-owned host", value.name.startsWith("presentation-"))
            val host = SavedHost(value.deviceId, value.name, value.relayUrls, null)
            store.save(saved.copy(hosts = saved.hosts.filterNot { it.name == host.name } + host, recent = null))
            runtime.commitPairing(pairing)
            host
        }
        ticket.delete()
        assertTrue("fresh fixture has no Sessions", runtime.listSessions(host.id).isEmpty())
        val obsolete = runtime.createSession(host.id, "android-reconnect-obsolete", null, viewport, true)
        runtime.closeSession(host.id, obsolete.sessionId)
        store.save(store.load().copy(hosts = store.load().hosts.map { if (it.id == host.id) it.copy(lastSession = obsolete.sessionId) else it },
            recent = RecentConnection(host.id, obsolete.sessionId), preferences = Preferences("en", "dark", 12)))
        await { repository.state.value.initialized }
        val ready = File(app.cacheDir, "reconnect-ready")
        val restarted = File(app.cacheDir, "reconnect-restarted")
        ready.delete(); restarted.delete()
        val owned = mutableSetOf<String>()
        var testFailure: Throwable? = null
        try {
            ActivityScenario.launch(MainActivity::class.java).use {
                main { repository.connectHost(host.id, recent = true) }
                active()
                val first = requireNotNull(repository.state.value.sessionId)
                owned.add(first)
                assertNotEquals(obsolete.sessionId, first)
                assertEquals("main", runtime.listSessions(host.id).single().name)
                assertEquals(first, store.load().recent?.session)
                assertEquals(first, store.load().hosts.first { it.id == host.id }.lastSession)
                await { repository.frame.value?.let { it.connectionPath != NativeConnectionPath.UNKNOWN && it.rttMs != null } == true }
                for (language in listOf("en", "zh", "system")) {
                    main { repository.updatePreferences { it.copy(language = language) } }
                    await { repository.state.value.saved.preferences.language == language }
                    ui.onNode(hasText(" · Direct ·", substring = true) or hasText(" · Relay ·", substring = true)).assertIsDisplayed()
                }
                main { repository.updatePreferences { it.copy(language = "en") } }
                await { repository.state.value.saved.preferences.language == "en" }
                ui.waitForIdle()
                instrumentation.uiAutomation.takeScreenshot()?.let { bitmap ->
                    File(app.cacheDir, "reconnect-header.png").outputStream().use { bitmap.compress(Bitmap.CompressFormat.PNG, 100, it) }
                    bitmap.recycle()
                }
                if (args.getString("daemonRestart") == "1") {
                    ready.writeText("ready")
                    withTimeout(120_000) { while (!restarted.isFile) delay(100) }
                    await { repository.frame.value?.state in setOf("ended", "closed") }
                    assertEquals(NativeConnectionPath.UNKNOWN, repository.frame.value!!.connectionPath)
                    assertNull(repository.frame.value!!.rttMs)
                    ui.onNodeWithText("Retry").performClick()
                    active()
                    val replacement = requireNotNull(repository.state.value.sessionId)
                    owned.add(replacement)
                    assertNotEquals(first, replacement)
                    assertEquals(replacement, runtime.listSessions(host.id).single().sessionId)
                    assertEquals(replacement, store.load().recent?.session)
                }
                val current = requireNotNull(repository.state.value.sessionId)
                home()
                main { repository.connectHost(host.id) }
                active()
                assertEquals("live remembered Session reused", current, repository.state.value.sessionId)
                assertEquals(1, runtime.listSessions(host.id).size)
                main { assertTrue(repository.text("printf 'RECOVERY_%s\\n' OK\r")) }
                await { repository.frame.value?.rows?.any { row -> row.cells.joinToString("") { it.text }.contains("RECOVERY_OK") } == true }

                if (args.getString("daemonRestart") == "1") {
                    // The first explicit mutation still carries the old daemon's
                    // lease. Preserve its unknown outcome and do not replay it.
                    val error = runCatching { runtime.createSession(host.id, "android-reconnect-remaining", null, viewport, true) }.exceptionOrNull()
                    assertTrue(error is NativeException.RequestFailed)
                    assertEquals("operation_outcome_unknown", (error as NativeException.RequestFailed).code)
                    assertEquals(1, runtime.listSessions(host.id).size)
                }
                // A new explicit mutation obtains a fresh lease. An ended
                // selection with one remaining Session reuses it.
                val remaining = runtime.createSession(host.id, "android-reconnect-remaining", null, viewport, true)
                owned.add(remaining.sessionId)
                runtime.closeSession(host.id, current)
                await { repository.frame.value?.state == "ended" }
                main { repository.retry() }
                active()
                assertEquals(remaining.sessionId, repository.state.value.sessionId)
                assertEquals(1, runtime.listSessions(host.id).size)

                // Multiple candidates never create another main or choose silently.
                val second = runtime.createSession(host.id, "android-reconnect-second", null, viewport, true)
                val third = runtime.createSession(host.id, "android-reconnect-third", null, viewport, true)
                owned.add(second.sessionId); owned.add(third.sessionId)
                runtime.closeSession(host.id, remaining.sessionId)
                await { repository.frame.value?.state == "ended" }
                main { repository.retry() }
                await { !repository.state.value.busy && repository.state.value.panel }
                assertNull(repository.state.value.sessionId)
                assertNull(repository.frame.value)
                assertEquals(2, runtime.listSessions(host.id).size)

                // A sole occupied candidate stays in the picker; its owner remains Active.
                runtime.closeSession(host.id, third.sessionId)
                runtime.connectTerminal(host.id, second.sessionId, viewport, true, false).use { competitor ->
                    var frame = competitor.currentFrame()
                    while (frame.state != "active") {
                        val generation = frame.generation; frame.source?.close()
                        frame = competitor.waitForFrame(generation)
                    }
                    frame.source?.close()
                    main { repository.retry() }
                    await { !repository.state.value.busy && repository.state.value.panel }
                    assertNull(repository.frame.value)
                    assertEquals(1, runtime.listSessions(host.id).size)
                    competitor.currentFrame().let { assertEquals("active", it.state); it.source?.close() }
                    competitor.detach()
                }
                home()
                main { repository.connectHost(host.id); repository.goHome() }
                await { !repository.state.value.busy }
                assertEquals(Route.Home, repository.state.value.route)
                assertNull(repository.frame.value)
            }
        } catch (error: Throwable) {
            testFailure = error
            throw error
        } finally {
            ready.delete(); restarted.delete()
            val cleanupFailure = runCatching {
                home()
                for (session in runtime.listSessions(host.id)) {
                    if (session.sessionId in owned) runtime.closeSession(host.id, session.sessionId)
                }
                main { repository.setPreferences(saved.preferences) }
                await { repository.state.value.saved.preferences == saved.preferences }
            }.exceptionOrNull()
            if (cleanupFailure != null) {
                if (testFailure == null) throw cleanupFailure
                testFailure.addSuppressed(cleanupFailure)
            }
        }
    }

    private fun main(action: () -> Unit) = instrumentation.runOnMainSync(action)
    private suspend fun active() {
        await { !repository.state.value.busy }
        assertNull("connection error", repository.state.value.error)
        await { repository.frame.value?.state == "active" }
    }
    private suspend fun home() {
        main { repository.goHome() }
        await { repository.state.value.hostId == null && !repository.state.value.busy }
    }
    private suspend fun await(predicate: () -> Boolean) = withTimeout(30_000) { while (!predicate()) delay(20) }
}
