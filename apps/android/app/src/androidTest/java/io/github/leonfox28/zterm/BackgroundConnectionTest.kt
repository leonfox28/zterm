package io.github.leonfox28.zterm

import android.app.ActivityManager
import android.app.NotificationManager
import android.view.View
import android.view.ViewGroup
import android.view.inputmethod.EditorInfo
import androidx.lifecycle.Lifecycle
import androidx.test.core.app.ActivityScenario
import androidx.test.core.app.ApplicationProvider
import androidx.test.platform.app.InstrumentationRegistry
import io.github.leonfox28.zterm.nativebridge.*
import kotlinx.coroutines.*
import org.junit.Assert.*
import org.junit.Assume.assumeTrue
import org.junit.Test
import java.io.File

/** Explicit disposable presentation fixture only. Never discovers a user's host. */
class BackgroundConnectionTest {
    private val instrumentation = InstrumentationRegistry.getInstrumentation()
    private val app = ApplicationProvider.getApplicationContext<ZtermApplication>()
    private val repository get() = app.repository
    private val runtime get() = app.runtime
    private val store = AppStore(app)
    private val viewport = NativeViewport(24u, 80u)

    @Test fun connectionSurvivesActivityAndServiceLifecycle() = runBlocking {
        requireFixture()
        val host = pairFixture()
        await { repository.state.value.initialized }
        assertFalse(repository.state.value.saved.preferences.keepBackgroundConnection)
        ActivityScenario.launch(MainActivity::class.java).use { activity ->
            main { repository.connectHost(host.id) }
            active()
            val session = requireNotNull(repository.state.value.sessionId)
            assertFalse(serviceRunning())
            val notificationVisible = InstrumentationRegistry.getArguments().getString("notificationVisible") == "1"
            activity.moveToState(Lifecycle.State.CREATED)
            main { repository.updatePreferences { it.copy(keepBackgroundConnection = true) } }
            await { repository.state.value.saved.preferences.keepBackgroundConnection }
            assertFalse("never start a new service from the background", serviceRunning())
            activity.moveToState(Lifecycle.State.RESUMED)
            await { serviceRunning() }
            assertEquals(notificationVisible, repository.backgroundConnection.visibleNotification())
            if (notificationVisible) assertTrue(app.getSystemService(NotificationManager::class.java).activeNotifications.any { it.id == BackgroundConnection.NOTIFICATION_ID })
            assertEquals(RecentConnection(host.id, session), store.load().activeTerminal)
            activity.moveToState(Lifecycle.State.CREATED)
            instrumentation.uiAutomation.executeShellCommand("input keyevent KEYCODE_SLEEP").use { android.os.ParcelFileDescriptor.AutoCloseInputStream(it).readBytes() }
            assertTrue(serviceRunning())
            main { assertTrue(repository.text("printf 'BACKGROUND_%s\\n' OK\r")) }
            await { repository.frame.value?.rows?.any { row -> row.cells.joinToString("") { it.text }.contains("BACKGROUND_OK") } == true }
            instrumentation.uiAutomation.executeShellCommand("input keyevent KEYCODE_WAKEUP").use { android.os.ParcelFileDescriptor.AutoCloseInputStream(it).readBytes() }
            instrumentation.uiAutomation.executeShellCommand("wm dismiss-keyguard").use { android.os.ParcelFileDescriptor.AutoCloseInputStream(it).readBytes() }
            activity.moveToState(Lifecycle.State.RESUMED)
            activity.recreate()
            active()
            assertEquals(session, repository.state.value.sessionId)
            assertTrue(serviceRunning())
            main { assertTrue(repository.text("printf 'IME_%s\\n' '")) }
            activity.onActivity { screen ->
                val view = findTerminal(screen.window.decorView) ?: error("terminal view missing")
                val connection = view.onCreateInputConnection(EditorInfo())
                assertTrue(connection.setComposingText("nihao", 1))
                assertTrue(connection.commitText("你好", 1))
                assertTrue(connection.finishComposingText())
            }
            main { assertTrue(repository.text("'\r")) }
            await { repository.frame.value?.rows?.any { row -> row.cells.joinToString("") { it.text }.contains("IME_你好") } == true }
            assertEquals(1, repository.frame.value!!.rows.count { row -> row.cells.joinToString("") { it.text }.contains("IME_你好") })
            // Permission is configured by the coordinator before launch. Android
            // kills the app on revocation, which requires a separate process test.
            main { repository.updatePreferences { it.copy(keepBackgroundConnection = false) } }
            await { !serviceRunning() }
            assertEquals("active", repository.frame.value?.state)
            main { repository.updatePreferences { it.copy(keepBackgroundConnection = true) } }
            await { serviceRunning() }
            assertFalse(repository.backgroundConnection.failed.value)
            assertEquals(notificationVisible, repository.backgroundConnection.visibleNotification())
            val notification = repository.backgroundConnection.notification(repository.backgroundConnection.state.value)
            notification.actions.last().actionIntent.send()
            await { repository.state.value.route == Route.Home && repository.state.value.hostId == null && !serviceRunning() }
            await { store.load().activeTerminal == null }
            assertEquals(session, runtime.listSessions(host.id).single().sessionId)
            main { repository.connectHost(host.id) }
            active(); await { serviceRunning() }
            // Another controller taking over retires this service, not the remote Session.
            runtime.connectTerminal(host.id, session, viewport, true, true).use { competitor ->
                await { competitor.currentFrame().let { frame -> try { frame.state == "active" } finally { frame.source?.close() } } }
                await { repository.frame.value?.state == "lease_lost" }
                await { !serviceRunning() }
                assertEquals(session, runtime.listSessions(host.id).single().sessionId)
                competitor.detach()
            }
            main { repository.selectSession(session, true) }
            active(); await { serviceRunning() }
            runtime.closeSession(host.id, session)
            await { repository.frame.value?.state == "ended" && !serviceRunning() }
            main { repository.goHome(); repository.updatePreferences { it.copy(keepBackgroundConnection = false) } }
            await { store.load().activeTerminal == null }
        }
    }

    /** Run prepare, kill the process externally, then resume/occupied/ended in separate invocations. */
    @Test fun processRecoveryNeverCreatesOrSubstitutesSession() = runBlocking {
        requireFixture()
        val mode = InstrumentationRegistry.getArguments().getString("restoreMode") ?: ""
        assumeTrue("explicit process recovery mode", mode in setOf("prepare", "resume", "occupied", "ended"))
        if (mode == "prepare") {
            val host = pairFixture()
            await { repository.state.value.initialized }
            ActivityScenario.launch(MainActivity::class.java).use {
                main { repository.connectHost(host.id); repository.updatePreferences { it.copy(keepBackgroundConnection = true) } }
                active(); await { serviceRunning() }
                await { store.load().activeTerminal?.session == repository.state.value.sessionId }
            }
            return@runBlocking
        }
        val saved = store.load()
        val original = requireNotNull(saved.activeTerminal)
        assertTrue(saved.hosts.single { it.id == original.host }.name.startsWith("presentation-"))
        val seed = store.loadOrCreateSeed()
        try { runtime.initialize(seed, PlatformNetwork(app) {}.current(), saved.hosts.map { it.native() }) }
        finally { seed.fill(0) }
        val competitor = if (mode == "occupied") runtime.connectTerminal(original.host, original.session, viewport, true, false) else null
        val before = runtime.listSessions(original.host).map { it.sessionId }.toSet()
        if (mode == "ended") assertFalse("the host coordinator ended the original Session before reopening", original.session in before)
        try {
            ActivityScenario.launch(MainActivity::class.java).use {
                await { repository.state.value.initialized && repository.state.value.route == Route.Terminal && !repository.state.value.busy }
                assertEquals(original.session, repository.state.value.sessionId)
                assertTrue(repository.state.value.restoringSession)
                when (mode) {
                    "resume" -> {
                        active()
                        if (saved.preferences.keepBackgroundConnection) await { serviceRunning() }
                        else assertFalse(serviceRunning())
                    }
                    "occupied" -> {
                        assertTrue(repository.state.value.error in setOf("session_occupied", "controller_busy"))
                        assertFalse(serviceRunning())
                    }
                    "ended" -> {
                        assertEquals("session_not_found", repository.state.value.error)
                        assertFalse(serviceRunning())
                        main { repository.retry() }
                        await { !repository.state.value.busy }
                        assertEquals("session_not_found", repository.state.value.error)
                    }
                }
                assertEquals(before, runtime.listSessions(original.host).map { it.sessionId }.toSet())
            }
        } finally { competitor?.detach(); competitor?.close() }
    }

    private fun findTerminal(view: View): TerminalView? {
        if (view is TerminalView) return view
        if (view is ViewGroup) for (index in 0 until view.childCount) findTerminal(view.getChildAt(index))?.let { return it }
        return null
    }
    private fun requireFixture() {
        assumeTrue("explicit disposable host", InstrumentationRegistry.getArguments().getString("backgroundFixture") == "1")
    }
    private suspend fun pairFixture(): SavedHost {
        val ticket = File(app.filesDir, "background-ticket.txt")
        val saved = store.load()
        val seed = store.loadOrCreateSeed()
        try { runtime.initialize(seed, PlatformNetwork(app) {}.current(), saved.hosts.map { it.native() }) }
        finally { seed.fill(0) }
        if (InstrumentationRegistry.getArguments().getString("reuseBackgroundHost") == "1") {
            val host = saved.hosts.single { it.name == "presentation-fixture" }
            store.save(saved.copy(activeTerminal = null, preferences = saved.preferences.copy(keepBackgroundConnection = false)))
            return host
        }
        val host = withTimeout(30_000) { runtime.pairTicket(ticket.readText().trim()) }.use { pairing ->
            val value = pairing.host()
            assertTrue(value.name.startsWith("presentation-"))
            val host = SavedHost(value.deviceId, value.name, value.relayUrls)
            store.save(saved.copy(hosts = saved.hosts.filterNot { it.id == host.id } + host,
                activeTerminal = null, preferences = saved.preferences.copy(keepBackgroundConnection = false)))
            runtime.commitPairing(pairing)
            host
        }
        ticket.delete()
        return host
    }
    @Suppress("DEPRECATION") private fun serviceRunning(): Boolean = app.getSystemService(ActivityManager::class.java)
        .getRunningServices(100).any { it.service.className == TerminalConnectionService::class.java.name && it.foreground }
    private fun main(action: () -> Unit) = instrumentation.runOnMainSync(action)
    private suspend fun active() {
        await { !repository.state.value.busy }
        assertNull(repository.state.value.error)
        await { repository.frame.value?.state == "active" }
    }
    private suspend fun await(predicate: () -> Boolean) = withTimeout(30_000) { while (!predicate()) delay(25) }
}
