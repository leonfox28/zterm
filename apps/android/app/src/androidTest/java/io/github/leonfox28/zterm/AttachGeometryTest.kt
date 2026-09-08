package io.github.leonfox28.zterm

import androidx.test.core.app.ApplicationProvider
import androidx.test.platform.app.InstrumentationRegistry
import io.github.leonfox28.zterm.nativebridge.NativeViewport
import kotlinx.coroutines.delay
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.withTimeout
import org.junit.Assert.*
import org.junit.Assume.assumeTrue
import org.junit.Test

/** Run alone so the first Repository load uses only the disposable resume target. */
class AttachGeometryTest {
    @Test fun integralEndpointsPreserveCursorAndInterpolateBelowToolbarRemainder() {
        for (cell in listOf(17, 23, 29, 41)) {
            val start = 1000
            val end = 600
            var previousGrid = start - start % cell
            for (bottom in 0..400) {
                val available = start - bottom
                val remainder = terminalBottomRemainder(available, cell, bottom, 0, 400, true)
                val grid = available - remainder
                assertTrue(remainder in 0 until cell)
                assertTrue("animation has no whole-row steps", previousGrid - grid in 0..2)
                previousGrid = grid
                if (bottom == 0 || bottom == 400) assertEquals(0, grid % cell)
            }
            assertEquals(end - end % cell, previousGrid)
            assertEquals(start % cell, terminalBottomRemainder(start, cell, 0, 400, 0, false))
        }
        val rows = 1000 / 17
        val target = 600 / 17
        assertEquals(0f, terminalPan(target * 17f, 17f, rows, 10, 0), 0f)
        val middlePan = terminalPan(target * 17f, 17f, rows, 39, 0)
        assertEquals(-85f, middlePan, 0f)
        assertEquals(34 * 17f, 39 * 17f + middlePan, 0f)
        assertEquals(-(rows - target) * 17f, terminalPan(target * 17f, 17f, rows, rows - 1, 0), 0f)
        assertEquals(0, terminalBottomRemainder(0, 17, 0, 0, 400, false))
        assertEquals(0, terminalBottomRemainder(3, 17, 0, 0, 400, false))
    }

    @Test fun imeCompletionSurvivesCoalescedCompositionAndOverlappingAnimations() {
        val owner = ImeAnimationState()
        val events = mutableListOf<Boolean>()
        val stop = owner.observe { events.add(it) }
        val ime = androidx.core.view.WindowInsetsCompat.Type.ime()
        val first = androidx.core.view.WindowInsetsAnimationCompat(ime, null, 160)
        val second = androidx.core.view.WindowInsetsAnimationCompat(ime, null, 160)
        val bar = androidx.core.view.WindowInsetsAnimationCompat(androidx.core.view.WindowInsetsCompat.Type.navigationBars(), null, 160)
        owner.onPrepare(first)
        owner.onPrepare(second)
        owner.onPrepare(bar)
        owner.onEnd(first)
        owner.onEnd(bar)
        assertTrue(owner.running)
        owner.onEnd(second)
        assertEquals(listOf(false, true, false), events)
        // No Compose frame runs between these edges, but both events survive.
        owner.onPrepare(first); owner.onEnd(first)
        assertEquals(listOf(false, true, false, true, false), events)
        stop()
        owner.onPrepare(first); owner.onEnd(first)
        assertEquals(5, events.size)
    }

    @Test fun measurementsDuringAttachReachTheNewTerminal() = runBlocking {
        val instrumentation = InstrumentationRegistry.getInstrumentation()
        assumeTrue("explicit attach fixture", InstrumentationRegistry.getArguments().getString("attachGeometry") == "1")
        val application = ApplicationProvider.getApplicationContext<ZtermApplication>()
        val store = AppStore(application)
        val saved = store.load()
        val host = saved.hosts.first { it.name == (InstrumentationRegistry.getArguments().getString("hostName") ?: "my-mac") }
        val runtime = application.runtime
        val seed = store.loadOrCreateSeed()
        try { runtime.initialize(seed,PlatformNetwork(application) {}.current(),saved.hosts.map { it.native() }) }
        finally { seed.fill(0) }
        runtime.listSessions(host.id)
        val owned = runtime.createSession(host.id,"android-attach-size-${System.nanoTime()}",null,NativeViewport(39u,140u),true)
        try {
            store.save(saved.copy(hosts = saved.hosts.map { if (it.id == host.id) it.copy(lastSession = owned.sessionId) else it }))
            val repository = application.repository
            try {
                withTimeout(20_000) { while (!repository.state.value.initialized) delay(10) }
                instrumentation.runOnMainSync {
                    repository.connectHost(host.id)
                    assertTrue("measurement races an unfinished attach",repository.state.value.busy)
                    assertNull(repository.frame.value)
                    repository.measure(35,72)
                    repository.measure(31,67)
                }
                withTimeout(20_000) {
                    while (repository.frame.value?.state != "active" || repository.state.value.busy) delay(10)
                }
                // Allow the final serialized resize, then assert the actual host
                // grid instead of just the locally retained desired measurement.
                withTimeout(5_000) {
                    while (repository.frame.value?.let { it.state == "active" && it.viewport == NativeViewport(31u,67u) } != true) delay(10)
                }
                assertEquals(owned.sessionId,repository.state.value.sessionId)
                assertNull(repository.state.value.error)
                val actual = runtime.listSessions(host.id).first { it.sessionId == owned.sessionId }
                assertEquals(31.toUShort(),actual.rows)
                assertEquals(67.toUShort(),actual.columns)
            } finally {
                instrumentation.runOnMainSync { repository.goHome() }
                withTimeout(15_000) { while (repository.frame.value != null || repository.state.value.busy) delay(10) }
            }
        } finally {
            try { runtime.closeSession(host.id,owned.sessionId) }
            finally { store.save(saved) }
        }
    }
}
