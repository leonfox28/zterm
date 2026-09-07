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
    @Test fun measurementsDuringAttachReachTheNewTerminal() = runBlocking {
        val instrumentation = InstrumentationRegistry.getInstrumentation()
        assumeTrue("explicit attach fixture", InstrumentationRegistry.getArguments().getString("attachGeometry") == "1")
        val application = ApplicationProvider.getApplicationContext<ZtermApplication>()
        val store = AppStore(application)
        val saved = store.load()
        val host = saved.hosts.first { it.name == "my-mac" }
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
