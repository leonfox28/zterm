package io.github.leonfox28.zterm

import android.os.Looper
import androidx.test.core.app.ActivityScenario
import io.github.leonfox28.zterm.nativebridge.*
import kotlinx.coroutines.*
import kotlinx.coroutines.channels.Channel
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.first
import org.junit.Assert.*
import org.junit.Test
import java.util.concurrent.CountDownLatch
import java.util.concurrent.TimeUnit
import java.util.concurrent.atomic.AtomicInteger

class TerminalFramesTest {
    @Test fun relativeRowsKeepObjectIdentityAndNewAuthorityWithExplicitSourceLifetime() = runBlocking {
        val handles = mutableListOf<Source>()
        val a = row("A"); val b = row("B"); val c = row("C"); val d = row("D")
        val first = Source(handles) { previous ->
            assertNull(previous)
            listOf(a, b, c).map { NativeRowUpdate.Replace(it) }
        }
        val changed = Source(handles) { previous ->
            assertFalse(requireNotNull(previous).uniffiIsDestroyed)
            listOf(NativeRowUpdate.Reuse(2u), NativeRowUpdate.Replace(d), NativeRowUpdate.Reuse(1u))
        }
        val metadata = Source(handles) { error("metadata must not convert rows") }
        TerminalFrameProjection().use { projection ->
            val initial = projection.resolve(frame(1u, first))
            val moved = projection.resolve(frame(2u, changed).copy(geometryGeneration = 2u))
            assertSame(initial.rows[2], moved.rows[0]); assertSame(d, moved.rows[1])
            assertSame(initial.rows[1], moved.rows[2]); assertSame(changed, moved.source)
            assertEquals(2uL, moved.geometryGeneration)
            first.close()
            assertTrue(handles.filter { it.owner === first.owner }.all { it.uniffiIsDestroyed })
            val latest = projection.resolve(frame(3u, metadata).copy(contentGeneration = 2u, inputEpoch = 2u))
            assertSame(moved.rows, latest.rows); assertSame(metadata, latest.source)
            assertEquals(2uL, latest.inputEpoch)
            changed.close(); metadata.close()
            projection.resolve(frame(4u, null)) // eager fallback retires the retained baseline.
            assertTrue(handles.all { it.uniffiIsDestroyed })
        }
    }

    @Test fun displayGateCombinesBurstsAndHidingDeliversFinalState() = runBlocking {
        ActivityScenario.launch(MainActivity::class.java).use {
            val handles = mutableListOf<Source>()
            val conversions = AtomicInteger()
            fun source(value: String) = Source(handles) {
                conversions.incrementAndGet()
                listOf(NativeRowUpdate.Replace(row(value)))
            }
            val native = MutableStateFlow(frame(1u, source("A")))
            val delivered = Channel<NativeFrame>(Channel.UNLIMITED)
            val clock = TerminalFrameClock()
            var displayed: NativeFrame? = null
            fun capture() = native.value.let { it.copy(source = it.source?.retained()) }
            fun publish(frame: NativeFrame) {
                val old = native.value
                native.value = frame
                old.source?.close()
            }
            val observation = launch(Dispatchers.Main.immediate) {
                collectTerminalFrames(::capture, { generation ->
                    native.first { it.generation > generation }
                    capture()
                }, clock) { frame ->
                    displayed?.source?.close(); displayed = frame
                    delivered.trySend(frame)
                }
            }
            try {
                assertEquals(1uL, withTimeout(5_000) { delivered.receive() }.generation)
                withContext(Dispatchers.Main.immediate) {
                    clock.visible = true
                    publish(frame(2u, source("B")))
                    yield() // let the collector register its single display callback.
                    publish(frame(3u, source("C")))
                    publish(frame(4u, source("D")))
                    assertEquals("no content conversion inside the burst", 1, conversions.get())
                }
                val result = withTimeout(5_000) { delivered.receive() }
                assertEquals(4uL, result.generation)
                assertEquals("only the latest window was prepared", 2, conversions.get())
                assertEquals("D", result.rows.single().cells.single().text)
                withContext(Dispatchers.Main.immediate) {
                    publish(frame(5u, source("E")))
                    yield()
                    publish(frame(6u, source("F")).copy(state = "ended", inputReady = false, inputEpoch = 2u))
                    // Hide in the same Main turn: progress cannot require another display callback.
                    clock.visible = false
                }
                val final = withTimeout(5_000) { delivered.receive() }
                assertEquals(6uL, final.generation); assertEquals("ended", final.state)
                assertFalse(final.inputReady); assertEquals(2uL, final.inputEpoch)
                assertEquals(3, conversions.get())
            } finally {
                withContext(Dispatchers.Main.immediate) {
                    observation.cancelAndJoin()
                    displayed?.source?.close(); native.value.source?.close()
                }
            }
            assertTrue("skipped and retained source handles all close", handles.all { it.uniffiIsDestroyed })
        }
    }

    @Test fun cancellationDuringProjectionClosesCandidateAndBaseline() = runBlocking {
        val handles = mutableListOf<Source>()
        val entered = CountDownLatch(1)
        val release = CountDownLatch(1)
        val first = Source(handles) { listOf(NativeRowUpdate.Replace(row("A"))) }
        val pending = Source(handles) {
            entered.countDown()
            check(release.await(5, TimeUnit.SECONDS))
            listOf(NativeRowUpdate.Reuse(0u))
        }
        val initialized = CompletableDeferred<Unit>()
        val job = launch(Dispatchers.Main.immediate) {
            TerminalFrameProjection().use { projection ->
                projection.resolve(frame(1u, first))
                initialized.complete(Unit)
                projection.resolve(frame(2u, pending))
                fail("cancelled projection must not be delivered")
            }
        }
        try {
            withTimeout(5_000) { initialized.await() }
            assertTrue(entered.await(5, TimeUnit.SECONDS))
            job.cancel()
        } finally {
            release.countDown(); job.join(); first.close()
        }
        assertTrue(handles.all { it.uniffiIsDestroyed })
    }

    /** UniFFI's supported no-handle constructor; only the bridge ownership protocol is faked. */
    private class Source(
        private val handles: MutableList<Source>,
        val owner: Any = Any(),
        private val updates: (NativeFrameSource?) -> List<NativeRowUpdate>,
    ) : NativeFrameSource(NoHandle) {
        init { handles.add(this) }
        override fun retained(): NativeFrameSource {
            check(!uniffiIsDestroyed)
            return Source(handles, owner, updates)
        }
        override fun presentationRowsFrom(previous: NativeFrameSource?): List<NativeRowUpdate> {
            assertNotEquals("projection must stay off Main", Looper.getMainLooper(), Looper.myLooper())
            check(!uniffiIsDestroyed)
            return updates(previous)
        }
    }

    private fun row(text: String) = NativeRow(listOf(NativeCell(text, 1u, 0xffffffffu, 0xff000000u, 0u, 0u, 0xffffffffu)), false)
    private fun frame(generation: ULong, source: NativeFrameSource?) = NativeFrame(
        applicationTitle = "",
        cursorShape = NativeCursorShape.BLOCK, cursorBlinking = false,
        connectionPath = NativeConnectionPath.UNKNOWN, rttMs = null,
        pointerMode = NativePointerMode.NONE, activeScreen = NativeActiveScreen.MAIN, source = source,
        stats = NativeNavigationStats(0u, 0u, 0u, 0u, 0u, 0u, 0u), notice = null,
        inputEpoch = 1u, geometryGeneration = 1u, inputReady = true,
        historyOffset = 0u, windowOffset = 0u, contentGeneration = generation,
        firstRow = 0, selection = null, generation = generation, state = "active", error = null,
        viewport = NativeViewport(3u, 1u), rows = emptyList(), cursorRow = 0u, cursorColumn = 0u,
        cursorVisible = false, cursorColor = 0xffffffffu, background = 0xff000000u, historyMaximum = 0u,
    )
}
