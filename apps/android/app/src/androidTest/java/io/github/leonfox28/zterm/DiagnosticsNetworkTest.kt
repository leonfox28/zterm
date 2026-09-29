package io.github.leonfox28.zterm

import androidx.test.core.app.ApplicationProvider
import androidx.test.platform.app.InstrumentationRegistry
import io.github.leonfox28.zterm.nativebridge.*
import java.io.ByteArrayOutputStream
import java.io.File
import kotlinx.coroutines.delay
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.withTimeout
import org.json.JSONObject
import org.junit.Assert.*
import org.junit.Assume.assumeTrue
import org.junit.Test

/** Explicitly selected disposable Linux host/emulator only. The coordinator pauses
 * that host after diagnostics-ready, resumes it after diagnostics-reconnecting,
 * then launches the restart test in a new application process. */
class DiagnosticsNetworkTest {
    private val args get() = InstrumentationRegistry.getArguments()
    private val app get() = ApplicationProvider.getApplicationContext<ZtermApplication>()
    private val diagnostics get() = app.diagnostics

    @Test fun nativeFailureAndRealReconnectPersistWithoutTerminalContent() = runBlocking {
        assumeTrue(args.getString("diagnosticsNetwork") == "1")
        val repository = app.repository
        withTimeout(20_000) { while (!repository.state.value.initialized) delay(20) }
        val runtime = app.runtime
        val ticketFile = File(app.filesDir, "diagnostics-ticket.txt")
        val store = AppStore(app)
        val reuse = args.getString("reuseDiagnosticsHost") == "1"
        val ticket = if (reuse) "" else ticketFile.readText().trim()
        val host = if (reuse) {
            store.load().hosts.single { it.name.startsWith("logging-fixture") }
        } else runtime.pairTicket(ticket).use { pairing ->
            val value = pairing.host()
            assertTrue("disposable host only", value.name.startsWith("logging-fixture"))
            val host = SavedHost(value.deviceId, value.name, value.relayUrls, null)
            store.save(store.load().copy(hosts = listOf(host)))
            runtime.commitPairing(pairing)
            host
        }
        ticketFile.delete()
        // Cold relay/discovery readiness is a read-only fixture precondition;
        // never retry the Session mutation or attach assertion under test.
        withTimeout(30_000) {
            var ready = false
            while (!ready) {
                ready = runCatching { runtime.listSessions(host.id) }.isSuccess
                if (!ready) delay(250)
            }
        }
        val viewport = NativeViewport(24u, 80u)
        // A real shared-client connect failure before any Session exists.
        val failed = runCatching { runtime.connectTerminal("77".repeat(32), "88".repeat(16), viewport, true, false) }.exceptionOrNull()
        assertTrue(failed is NativeException.RequestFailed)
        val session = runtime.createSession(host.id, "logging-fixture-session", null, viewport, true)
        val ready = File(app.cacheDir, "diagnostics-ready")
        val reconnecting = File(app.cacheDir, "diagnostics-reconnecting")
        ready.delete(); reconnecting.delete()
        try {
            runtime.connectTerminal(host.id, session.sessionId, viewport, true, false).use { terminal ->
                val initial = waitFrame(terminal) { it.state == "active" && it.inputReady }
                terminal.commitText(initial.inputEpoch, "printf 'LOG_CONTENT_%s\\n' SENTINEL\r", 0u, false)
                waitFrame(terminal) { text(it).contains("LOG_CONTENT_SENTINEL") }
                ready.writeText("ready")
                val lost = waitFrame(terminal) { it.state == "reconnecting" }
                assertFalse(lost.inputReady)
                reconnecting.writeText("reconnecting")
                val resumed = waitFrame(terminal) { it.state == "active" }
                assertNotEquals(initial.inputEpoch, resumed.inputEpoch)
                assertEquals(session.sessionId, terminal.sessionId())
                terminal.detach()
            }
        } finally {
            runtime.closeSession(host.id, session.sessionId)
            ready.delete(); reconnecting.delete()
        }
        diagnostics.files.setControl(diagnosticControl(true))
        withTimeout(5_000) { while (!diagnostics.native.detailEnabled()) delay(20) }
        diagnostics.record(AppDiagnostic.VIEWPORT, columns = 80u, rows = 24u)
        assertTrue(diagnostics.native.flush())
        val output = ByteArrayOutputStream()
        diagnostics.files.export(output, true, diagnostics.native.pendingLost())
        val exported = output.toString("UTF-8")
        if (ticket.isNotEmpty()) assertFalse(exported.contains(ticket))
        assertFalse(exported.contains("LOG_CONTENT_SENTINEL"))
        assertFalse(exported.contains("printf"))
        assertReconnectRecords(exported)
    }

    @Test fun nextProcessRetainsHistoryAndOnlyRemainingDetailInterval() = runBlocking {
        assumeTrue(args.getString("diagnosticsRestart") == "1")
        assertTrue(diagnosticRemainingMs(diagnostics.files.control()) in 1uL..900_000uL)
        withTimeout(5_000) { while (!diagnostics.native.detailEnabled()) delay(20) }
        val output = ByteArrayOutputStream()
        diagnostics.files.export(output, true, diagnostics.native.pendingLost())
        assertReconnectRecords(output.toString("UTF-8"))
        assertFalse(output.toString("UTF-8").contains("LOG_CONTENT_SENTINEL"))
        diagnostics.files.setControl(diagnosticControl(false))
        withTimeout(5_000) { while (diagnostics.native.detailEnabled()) delay(20) }
    }

    private fun assertReconnectRecords(exported: String) {
        val records = exported.lineSequence().filter { it.isNotBlank() }.map(::JSONObject).filter { it.has("event") }.toList()
        assertTrue(records.any { it.getString("event") == "connection_failed" })
        val start = records.last { it.getString("event") == "reconnect_started" }.getJSONObject("fields")
        val completed = records.last { it.getString("event") == "reconnect_completed" }.getJSONObject("fields")
        assertEquals(start.getLong("operation_id"), completed.getLong("operation_id"))
        assertEquals(start.getString("session_id"), completed.getString("session_id"))
        assertNotEquals(start.getString("attachment_id"), completed.getString("attachment_id"))
        assertTrue(completed.has("elapsed_ms"))
    }
    private suspend fun waitFrame(terminal: NativeTerminal, predicate: (NativeFrame) -> Boolean): NativeFrame = withTimeout(90_000) {
        var frame = resolved(terminal.currentFrame())
        while (!predicate(frame)) {
            assertFalse("unexpected terminal outcome: ${frame.state}", frame.state in setOf("closed", "ended", "lease_lost"))
            val generation = frame.generation
            frame.source?.close()
            frame = resolved(terminal.waitForFrame(generation))
        }
        frame.source?.close()
        frame
    }
    private fun resolved(frame: NativeFrame) = frame.source?.let { frame.copy(rows = it.presentationRows()) } ?: frame
    private fun text(frame: NativeFrame) = frame.rows.joinToString("\n") { row -> row.cells.joinToString("") { it.text } }
}
