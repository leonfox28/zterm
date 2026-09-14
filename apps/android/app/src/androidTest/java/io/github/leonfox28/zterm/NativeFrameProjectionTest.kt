package io.github.leonfox28.zterm

import androidx.test.platform.app.InstrumentationRegistry
import io.github.leonfox28.zterm.nativebridge.*
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.withTimeout
import org.junit.Assert.*
import org.junit.Assume.assumeTrue
import org.junit.Test
import java.io.File

/** Opt-in: a fresh presentation_fixture host and its ticket, never a saved user host. */
class NativeFrameProjectionTest {
    @Test fun realBridgeReusesRowsAndKeepsResizeAuthority() = runBlocking {
        val instrumentation = InstrumentationRegistry.getInstrumentation()
        assumeTrue("explicit disposable host", InstrumentationRegistry.getArguments().getString("projectionFixture") == "1")
        val context = instrumentation.targetContext
        val ticket = File(context.filesDir, "projection-ticket.txt")
        val store = AppStore(context)
        val seed = store.loadOrCreateSeed()
        NativeRuntime().use { runtime ->
            try {
                runtime.initialize(seed, PlatformNetwork(context) {}.current(), emptyList())
                val host = runtime.pairTicket(ticket.readText().trim()).use { pairing ->
                    val value = pairing.host()
                    require(value.name.startsWith("presentation-"))
                    runtime.commitPairing(pairing)
                    value
                }
                ticket.delete()
                assertTrue(runtime.listSessions(host.deviceId).isEmpty())
                val session = runtime.createSession(host.deviceId, "android-projection-${System.nanoTime()}", null, NativeViewport(40u, 80u), true)
                try {
                    runtime.connectTerminal(host.deviceId, session.sessionId, NativeViewport(40u, 80u), true, false).use { terminal ->
                        TerminalFrameProjection().use { projection ->
                            var displayed: NativeFrame? = null
                            suspend fun awaitFrame(stage: String, predicate: (NativeFrame) -> Boolean): NativeFrame {
                                android.util.Log.i("ZtermAcceptance", "projection stage=$stage")
                                return try { withTimeout(15_000) {
                                suspend fun accept(next: NativeFrame): NativeFrame {
                                    val resolved = projection.resolve(next)
                                    assertEquals("real UniFFI update reconstruction matches complete projection",
                                        resolved.source?.presentationRows() ?: resolved.rows, resolved.rows)
                                    displayed?.source?.close(); displayed = resolved
                                    return resolved
                                }
                                var resolved = accept(terminal.currentFrame())
                                while (!predicate(resolved)) resolved = accept(terminal.waitForFrame(resolved.generation))
                                resolved
                                } } catch (error: kotlinx.coroutines.TimeoutCancellationException) {
                                    throw AssertionError("$stage: state=${displayed?.state} screen=${displayed?.activeScreen} rows=${displayed?.viewport?.rows}", error)
                                }
                            }
                            fun firstLine(frame: NativeFrame) = frame.rows.firstOrNull()?.cells?.joinToString("") { it.text }.orEmpty()
                            try {
                                val initial = awaitFrame("active") { it.state == "active" }
                                val encoded = android.util.Base64.encodeToString(fixture.toByteArray(), android.util.Base64.NO_WRAP)
                                terminal.commitText(initial.inputEpoch, "python3 -c 'import base64; exec(base64.b64decode(\"$encoded\"))'\r", 0u, false)
                                val full = awaitFrame("initial TUI") { it.activeScreen == NativeActiveScreen.ALTERNATE && firstLine(it).startsWith("P00") }
                                val rows = full.rows
                                requireNotNull(full.source).retained().use { baseline ->
                                    terminal.commitText(full.inputEpoch, "s", 0u, false)
                                    val sparse = awaitFrame("sparse output") { it.rows.getOrNull(5)?.cells?.firstOrNull()?.text == "S" }
                                    val updates = requireNotNull(sparse.source).presentationRowsFrom(baseline)
                                    assertEquals("one changed row crosses FFI with cells", 1, updates.count { it is NativeRowUpdate.Replace })
                                    assertSame("unchanged Kotlin row object survives actual output", rows[0], sparse.rows[0])
                                    assertNotSame(rows[5], sparse.rows[5])
                                    requireNotNull(sparse.source).retained().use { beforeResize ->
                                        terminal.resize(NativeViewport(30u, 80u))
                                        val small = awaitFrame("shrink repaint") { it.state == "active" && it.viewport.rows == 30.toUShort() && firstLine(it).startsWith("P00") }
                                        assertEquals("same text survives changed geometry", 0,
                                            requireNotNull(small.source).presentationRowsFrom(beforeResize).count { it is NativeRowUpdate.Replace })
                                        val rejected = runCatching { terminal.beginSelection(beforeResize, 0u, 0u) }.exceptionOrNull()
                                        assertTrue(rejected is NativeException.RequestFailed && rejected.code == "selection_changed")
                                        requireNotNull(small.source).retained().use { smaller ->
                                            terminal.resize(NativeViewport(40u, 80u))
                                            val large = awaitFrame("growth repaint") { it.state == "active" && it.viewport.rows == 40.toUShort() && it.rows.lastOrNull()?.cells?.take(3)?.joinToString("") { it.text } == "P39" }
                                            assertEquals("growth converts only new rows", 10,
                                                requireNotNull(large.source).presentationRowsFrom(smaller).count { it is NativeRowUpdate.Replace })
                                            assertEquals(full.inputEpoch, large.inputEpoch)
                                            terminal.commitText(large.inputEpoch, "q", 0u, false)
                                        }
                                    }
                                }
                                awaitFrame("main screen restored") { it.activeScreen == NativeActiveScreen.MAIN }
                                terminal.detach()
                            } finally { displayed?.source?.close() }
                        }
                    }
                } finally { runtime.closeSession(host.deviceId, session.sessionId) }
                assertTrue(runtime.listSessions(host.deviceId).isEmpty())
            } finally { seed.fill(0); ticket.delete(); withTimeout(5_000) { runtime.shutdown() } }
        }
    }

    private val fixture = """
        import os, sys, signal, termios, tty
        saved = termios.tcgetattr(0)
        sparse = False
        def draw(*unused):
            columns, rows = os.get_terminal_size()
            output = ['\x1b[?2026h']
            for row in range(rows):
                prefix = ('S' if sparse and row == 5 else 'P') + f'{row:02d} '
                text = (prefix + 'a' * columns)[:columns]
                output.append(f'\x1b[{row+1};1H' + text)
            output.append('\x1b[?2026l')
            sys.stdout.write(''.join(output)); sys.stdout.flush()
        try:
            tty.setraw(0)
            sys.stdout.write('\x1b[?1049h\x1b[?25l')
            signal.signal(signal.SIGWINCH, draw)
            draw()
            while True:
                key = os.read(0, 1)
                if key == b'q': break
                if key == b's': sparse = True; draw()
        finally:
            sys.stdout.write('\x1b[?25h\x1b[?1049l'); sys.stdout.flush()
            termios.tcsetattr(0, termios.TCSANOW, saved)
    """.trimIndent()
}
