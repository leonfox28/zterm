package io.github.leonfox28.zterm

import androidx.test.platform.app.InstrumentationRegistry
import io.github.leonfox28.zterm.nativebridge.*
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.withTimeout
import org.junit.Assert.*
import org.junit.Assume.assumeTrue
import org.junit.Test
import java.io.File

/** Opt-in coordinator cuts only this test app UID's transport, then restores it. */
class NetworkInterruptionTest {
    @Test fun interruptedTransportResumesWithoutReplayingInput() = runBlocking {
        assumeTrue(InstrumentationRegistry.getArguments().getString("networkInterruption") == "1")
        val context = InstrumentationRegistry.getInstrumentation().targetContext
        val store = AppStore(context)
        val hosts = store.load().hosts
        val host = hosts.first { it.name == "my-mac" }
        val seed = store.loadOrCreateSeed()
        val ready = File(context.cacheDir,"network-ready")
        val interrupted = File(context.cacheDir,"network-interrupted")
        ready.delete(); interrupted.delete()
        try {
            NativeRuntime().use { runtime ->
                runtime.initialize(seed,PlatformNetwork(context) {}.current(),hosts.map { it.native() })
                runtime.listSessions(host.id) // Match the saved-host navigation flow.
                val session = runtime.createSession(host.id,"android-network-${System.nanoTime()}",null,NativeViewport(24u,80u),true)
                try {
                    runtime.connectTerminal(host.id,session.sessionId,NativeViewport(24u,80u),true,false).use { terminal ->
                        val initial = wait(terminal) { it.state == "active" }
                        terminal.commitText(initial.inputEpoch,"printf 'BEFORE_NETWORK_%s\\n' ok\r",0u,false)
                        wait(terminal) { text(it).contains("BEFORE_NETWORK_ok") }
                        ready.writeText("ready")
                        val lost = wait(terminal) { it.state == "reconnecting" }
                        assertFalse(lost.inputReady)
                        assertTrue("last complete pixels retained",text(lost).contains("BEFORE_NETWORK_ok"))
                        val rejected = runCatching { terminal.commitText(lost.inputEpoch,"NEVER_REPLAY_NETWORK\r",0u,false) }.exceptionOrNull()
                        assertTrue(rejected is NativeException.RequestFailed && rejected.code == "input_not_ready")
                        interrupted.writeText("reconnecting")
                        val resumed = wait(terminal) { it.state == "active" }
                        assertNotEquals(initial.inputEpoch,resumed.inputEpoch)
                        terminal.commitText(resumed.inputEpoch,"printf 'AFTER_NETWORK_%s\\n' ok\r",0u,false)
                        val output = wait(terminal) { text(it).contains("AFTER_NETWORK_ok") }
                        assertFalse(text(output).contains("NEVER_REPLAY_NETWORK"))
                        terminal.detach()
                    }
                    assertTrue(runtime.listSessions(host.id).any { it.sessionId == session.sessionId })
                } finally { runtime.closeSession(host.id,session.sessionId) }
            }
        } finally { seed.fill(0); ready.delete(); interrupted.delete() }
    }
    private suspend fun wait(terminal: NativeTerminal,predicate: (NativeFrame)->Boolean): NativeFrame = withTimeout(55_000) {
        var frame = terminal.currentFrame()
        while (!predicate(frame)) {
            assertFalse("unexpected terminal end: ${frame.state}",frame.state in setOf("closed","ended","lease_lost"))
            val generation = frame.generation
            frame.source?.close()
            frame = terminal.waitForFrame(generation)
        }
        frame.source?.close()
        frame
    }
    private fun text(frame: NativeFrame) = frame.rows.joinToString("\n") { row -> row.cells.joinToString("") { it.text } }
}
