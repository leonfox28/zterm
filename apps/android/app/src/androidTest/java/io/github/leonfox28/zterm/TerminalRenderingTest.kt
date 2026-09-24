package io.github.leonfox28.zterm

import android.graphics.Bitmap
import android.graphics.Canvas
import android.graphics.Color
import android.os.SystemClock
import android.view.MotionEvent
import android.view.PixelCopy
import android.graphics.Rect
import android.os.Handler
import android.os.Looper
import androidx.test.core.app.ActivityScenario
import androidx.test.platform.app.InstrumentationRegistry
import androidx.activity.compose.setContent
import androidx.compose.foundation.layout.*
import androidx.compose.material3.HorizontalDivider
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import androidx.compose.ui.viewinterop.AndroidView
import io.github.leonfox28.zterm.nativebridge.*
import org.junit.Assert.*
import org.junit.Test
import java.util.concurrent.CountDownLatch
import java.util.concurrent.TimeUnit

/** Draw real cells and dispatch real gestures, with native frame delivery held at a row boundary. */
class TerminalRenderingTest {
    private val instrumentation = InstrumentationRegistry.getInstrumentation()

    @Test fun cursorBlinkStopsWhenHiddenAndResumesWithoutANativeFrame() {
        ActivityScenario.launch(MainActivity::class.java).use { scenario ->
            lateinit var terminal: TerminalView
            scenario.onActivity { activity ->
                terminal = TerminalView(activity)
                activity.setContentView(terminal)
                terminal.update(frame(0, 0, 1, 0).copy(cursorVisible = true, cursorRow = 0u,
                    cursorBlinking = true), 12)
            }
            instrumentation.waitForIdleSync()
            fun capture(): Bitmap {
                lateinit var bitmap: Bitmap
                instrumentation.runOnMainSync { bitmap = pixels(terminal) }
                return bitmap
            }
            fun changesFrom(before: Bitmap): Boolean {
                repeat(20) {
                    SystemClock.sleep(75)
                    val next = capture()
                    val changed = !before.sameAs(next)
                    next.recycle()
                    if (changed) return true
                }
                return false
            }
            val first = capture()
            assertTrue("local timer changes caret pixels", changesFrom(first)); first.recycle()
            instrumentation.runOnMainSync { terminal.visibility = android.view.View.INVISIBLE }
            val hidden = capture()
            SystemClock.sleep(650)
            val later = capture()
            assertTrue("hidden View has no continuing blink", hidden.sameAs(later))
            hidden.recycle(); later.recycle()
            instrumentation.runOnMainSync { terminal.visibility = android.view.View.VISIBLE }
            val resumed = capture()
            assertTrue("visibility alone restarts the timer", changesFrom(resumed)); resumed.recycle()
        }
    }

    @Test fun cursorShapesProduceDistinctOverlaysWithoutChangingRows() {
        instrumentation.runOnMainSync {
            val base = frame(0, 0, 1, 0).copy(cursorVisible = true, cursorRow = 0u)
            val images = listOf(NativeCursorShape.BLOCK, NativeCursorShape.BEAM, NativeCursorShape.UNDERLINE)
                .map { pixels(liveView(base.copy(cursorShape = it))) }
            try {
                assertFalse(images[0].sameAs(images[1]))
                assertFalse(images[1].sameAs(images[2]))
                assertFalse(images[0].sameAs(images[2]))
            } finally { images.forEach { it.recycle() } }
        }
    }

    @Test fun concealHidesGlyphsAndDecorationsWhileStrikeChangesPixels() {
        instrumentation.runOnMainSync {
            val plain = frame(0, 0, 1, 0).copy(cursorVisible = false)
            fun withStyle(attributes: UByte, underline: UByte = 0u, text: String = "M") = plain.copy(
                rows = plain.rows.map { row -> row.copy(cells = row.cells.map {
                    it.copy(text = text, attributes = attributes, underline = underline)
                }) })
            val visible = pixels(liveView(withStyle(0u)))
            val struck = pixels(liveView(withStyle(8u)))
            val hidden = pixels(liveView(withStyle(24u, 1u)))
            val blank = pixels(liveView(withStyle(0u, text = " ")))
            try {
                assertFalse("strike adds visible decoration", visible.sameAs(struck))
                assertTrue("conceal suppresses glyphs and every decoration", hidden.sameAs(blank))
            } finally {
                visible.recycle(); struck.recycle(); hidden.recycle(); blank.recycle()
            }
        }
    }

    @Test fun blankCellsKeepBackgroundsDecorationsAndAdjacentGlyphs() {
        instrumentation.runOnMainSync {
            val base = NativeCell(" ", 1u, Color.WHITE.toUInt(), Color.BLUE.toUInt(), 0u, 0u, Color.CYAN.toUInt())
            val rows = List(6) { underline -> NativeRow(List(16) { column ->
                base.copy(text = when (column) {
                    0 -> "M"; 2 -> " \u0301"; 4 -> "界"; 5 -> ""; 6 -> "e\u0301"; 8 -> "f"; else -> " "
                }, width = when (column) { 4 -> 2u; 5 -> 0u; else -> 1u },
                    background = (if (column % 2 == 0) Color.BLUE else Color.DKGRAY).toUInt(),
                    attributes = (column % 8).toUByte(), underline = underline.toUByte())
            }, false) }
            val frame = frame(0, 0, 6, 0).copy(rows = rows, viewport = NativeViewport(6u, 16u),
                activeScreen = NativeActiveScreen.ALTERNATE)
            val empty = frame.copy(rows = rows.map { row -> row.copy(cells = row.cells.map {
                if (it.text == " ") it.copy(text = "") else it
            }) })
            val noAccent = frame.copy(rows = rows.map { row -> row.copy(cells = row.cells.map {
                if (it.text == " \u0301") it.copy(text = " ") else it
            }) })
            for (font in listOf(8, 12, 16)) {
                fun render(value: NativeFrame): Bitmap {
                    val view = liveView(value)
                    view.update(value, font)
                    view.layout(0, 0, 1000, value.viewport.rows.toInt() * view.gridCellHeight)
                    return pixels(view)
                }
                val actual = render(frame)
                val reference = render(empty)
                val missingAccent = render(noAccent)
                try {
                    assertTrue("space ink is empty; background, underline and adjacent glyphs survive at $font sp", actual.sameAs(reference))
                    assertFalse("space with a combining mark still has visible ink at $font sp", actual.sameAs(missingAccent))
                } finally { actual.recycle(); reference.recycle(); missingAccent.recycle() }
            }
        }
    }

    @Test fun cachedRowsMoveImmediatelyAcrossSeveralRowsAndLateFrames() {
        instrumentation.runOnMainSync {
            val view = view(frame(12, 24, 37))
            val cell = view.gridCellHeight
            val initial = markerTop(view)
            val gesture = Drag(view, 8f * cell)
            try {
                gesture.move(5f * cell - 1)
                val moved = markerTop(view)
                assertEquals("three rows scroll without a native delivery", initial - 3 * cell - 1, moved)
                view.update(frame(9, 24, 37), 12)
                assertEquals("metadata handoff does not move text", moved, markerTop(view))
                view.update(frame(9, 16, 29), 12)
                assertEquals("replacement row window preserves position", moved, markerTop(view))
                gesture.move(5f * cell + 1)
                assertEquals("reversal is immediate", moved + 2, markerTop(view))
                view.update(frame(40, 50, 25), 12)
                assertEquals("late distant response cannot replace the actual viewport", moved + 2, markerTop(view))
            } finally { gesture.cancel() }
        }
    }

    @Test fun unknownNewerRowsStopAtTheEdgeAndNeverReplayOvershoot() {
        instrumentation.runOnMainSync {
            val view = view(frame(8, 8, 13))
            val cell = view.gridCellHeight
            val initial = markerTop(view)
            val gesture = Drag(view, 9f * cell)
            try {
                gesture.move(8f * cell + 1)
                assertEquals(initial - cell + 1, markerTop(view))
                gesture.move(5f * cell)
                assertEquals("one known overscan row bounds motion", initial - cell, markerTop(view))
                view.update(frame(7, 14, 27), 12)
                assertEquals("delivery only expands the available range", initial - cell, markerTop(view))
                gesture.move(5f * cell - 5)
                assertEquals("new motion uses the newly available rows", initial - cell - 5, markerTop(view))
            } finally { gesture.cancel() }
        }
    }

    @Test fun unknownOlderRowsWaitWithoutAJumpOnDelivery() {
        instrumentation.runOnMainSync {
            val view = view(frame(8, 8, 13))
            val cell = view.gridCellHeight
            val initial = markerTop(view)
            val gesture = Drag(view, 7f * cell)
            try {
                gesture.move(9f * cell)
                assertEquals(initial, markerTop(view))
                view.update(frame(9, 20, 33), 12)
                assertEquals(initial, markerTop(view))
                gesture.move(9f * cell + 5)
                assertEquals(initial + 5, markerTop(view))
            } finally { gesture.cancel() }
        }
    }

    @Test fun appendedOutputPreservesTheLogicalReadingPosition() {
        instrumentation.runOnMainSync {
            val view = view(frame(8, 20, 33))
            val initial = markerTop(view)
            view.update(frame(13, 25, 33, maximum = 1005), 12)
            assertEquals("appending live rows does not move old text", initial, markerTop(view))
        }
    }

    @Test fun outputAppendDuringTheFirstEdgeWaitKeepsTheCapturedRows() {
        instrumentation.runOnMainSync {
            val view = view(frame(0, 0, 12, marker = 1003))
            val cell = view.gridCellHeight
            val initial = markerTop(view)
            val gesture = Drag(view, 7f * cell)
            try {
                gesture.move(9f * cell)
                assertEquals("waiting for the first older row", initial, markerTop(view))
                view.update(frame(1, 12, 25, maximum = 1001, marker = 1003), 12)
                assertEquals("new live output does not move the captured reading position", initial, markerTop(view))
                gesture.move(9f * cell + 5)
                assertEquals(initial + 5, markerTop(view))
            } finally { gesture.cancel() }
        }
    }

    @Test fun hiddenCursorGridMovesLocallyAndMatchesDelayedResizeInBothDirections() {
        instrumentation.runOnMainSync {
            val original = frame(0, 0, 12, maximum = 0, marker = 5)
            val view = liveView(original)
            val cell = view.gridCellHeight
            assertEquals(5 * cell, markerTop(view))
            view.layout(0, 0, view.width, 9 * cell)
            assertEquals("hidden-cursor rows move before any remote frame", 2 * cell, markerTop(view))
            val moved = pixels(view)
            val resized = original.copy(generation = 2u, geometryGeneration = 2u,
                viewport = original.viewport.copy(rows = 9u), rows = original.rows.drop(3))
            view.update(resized, 12)
            val received = pixels(view)
            try { assertTrue("new ordinals preserve the moved presentation", moved.sameAs(received)) }
            finally { moved.recycle(); received.recycle() }
            view.layout(0, 0, view.width, 12 * cell)
            assertEquals("closing also moves available rows without remote data", 5 * cell, markerTop(view))
            // Reverse before the larger-grid response arrives.
            view.layout(0, 0, view.width, 9 * cell)
            assertEquals(2 * cell, markerTop(view))
            view.update(original.copy(generation = 3u, geometryGeneration = 3u), 12)
            assertEquals("late larger grid uses the actual local height", 2 * cell, markerTop(view))
            view.update(resized.copy(generation = 4u, geometryGeneration = 4u), 12)
            assertEquals(2 * cell, markerTop(view))
        }
    }

    @Test fun visibleCaretNeverLeavesTheTopEdgeOnEitherScreen() {
        instrumentation.runOnMainSync {
            for (screen in NativeActiveScreen.entries) for (cursor in listOf(0, 1, 3)) {
                val original = frame(0, 0, 12, maximum = 0, marker = cursor).copy(
                    activeScreen = screen, cursorVisible = true, cursorRow = cursor.toUShort(), cursorColumn = 15u)
                val view = liveView(original)
                val cell = view.gridCellHeight
                for (height in listOf(12 * cell, 11 * cell, 10 * cell, 6 * cell, cell / 2, 10 * cell, 12 * cell)) {
                    view.layout(0, 0, view.width, height)
                    assertEquals("visible caret remains in bounds: $screen cursor=$cursor height=$height",
                        maxOf(0, cursor * cell + height - 12 * cell), markerTop(view))
                }
            }
        }
    }

    @Test fun liveScreensKeepContentBelowVisibleCursorAboveToolbar() {
        instrumentation.runOnMainSync {
            for (screen in NativeActiveScreen.entries) {
                val original = frame(0, 0, 12, maximum = 0, marker = 11).copy(
                    activeScreen = screen, cursorVisible = true, cursorRow = 9u)
                val view = liveView(original)
                val cell = view.gridCellHeight
                view.layout(0, 0, view.width, 10 * cell + cell / 2)
                assertEquals("footer follows the intermediate local height before remote delivery",
                    view.height - cell, markerTop(view))
                view.layout(0, 0, view.width, 9 * cell)
                assertEquals("both rows below the cursor remain above the toolbar", 8 * cell, markerTop(view))
                val moved = pixels(view)
                val resized = original.copy(generation = 2u, geometryGeneration = 2u,
                    viewport = original.viewport.copy(rows = 9u), rows = original.rows.drop(3), cursorRow = 6u)
                view.update(resized, 12)
                val received = pixels(view)
                try { assertTrue("delayed resize preserves text, footer and caret pixels", moved.sameAs(received)) }
                finally { moved.recycle(); received.recycle() }
            }
        }
    }

    @Test fun resizeSnapshotCannotInterruptEitherMovingScreen() {
        instrumentation.runOnMainSync {
            for (screen in NativeActiveScreen.entries) {
                val original = frame(0, 0, 12, maximum = 0, marker = 5).copy(
                    activeScreen = screen, cursorVisible = true, cursorRow = 10u)
                val view = liveView(original)
                val cell = view.gridCellHeight
                pixels(view).recycle() // Establish the actual drawn baseline.
                view.updateImeAnimation(true)
                try {
                    view.layout(0, 0, view.width, 9 * cell)
                    val moved = pixels(view)
                    try {
                        // Actual Alacritty height shrink keeps the caret visible:
                        // ROW 02..10, before the child repaints ROW 03..11.
                        val resized = original.copy(generation = 2u, geometryGeneration = 2u,
                            viewport = original.viewport.copy(rows = 9u),
                            rows = original.rows.drop(2).take(9), cursorRow = 8u)
                        view.update(resized, 12)
                        val intermediate = pixels(view)
                        try { assertTrue("resize-only snapshot must not jump the moving rows down", moved.sameAs(intermediate)) }
                        finally { intermediate.recycle() }
                        val repainted = resized.copy(generation = 3u, rows = original.rows.drop(3), cursorRow = 7u)
                        view.update(repainted, 12)
                        view.layout(0, 0, view.width, 10 * cell)
                        assertEquals("prepared smaller grid cannot expose an unknown row mid-animation", 3 * cell, markerTop(view))
                        view.layout(0, 0, view.width, 9 * cell)
                        view.updateImeAnimation(false)
                        view.viewTreeObserver.dispatchOnPreDraw()
                        val committed = pixels(view)
                        try { assertTrue("settled handoff uses the latest prepared TUI frame", moved.sameAs(committed)) }
                        finally { committed.recycle() }
                    } finally { moved.recycle() }
                } finally {
                    view.updateImeAnimation(false)
                    view.viewTreeObserver.dispatchOnPreDraw()
                }
            }
        }
    }

    @Test fun reversedImeKeepsTheDrawnGridUntilTheLatestTargetArrives() {
        instrumentation.runOnMainSync {
            for (screen in NativeActiveScreen.entries) {
                val original = frame(0, 0, 12, maximum = 0, marker = 5).copy(
                    activeScreen = screen, cursorVisible = true, cursorRow = 10u)
                val view = liveView(original)
                val cell = view.gridCellHeight
                val before = pixels(view)
                try {
                    view.updateImeAnimation(true)
                    view.prepareImeViewport(view.width, 9 * cell)
                    view.layout(0, 0, view.width, 9 * cell)
                    pixels(view).recycle()
                    view.prepareImeViewport(view.width, 12 * cell)
                    view.layout(0, 0, view.width, 12 * cell)
                    val obsolete = original.copy(generation = 2u, geometryGeneration = 2u,
                        viewport = original.viewport.copy(rows = 9u),
                        rows = original.rows.drop(2).take(9), cursorRow = 8u)
                    view.update(obsolete, 12)
                    view.updateImeAnimation(false)
                    view.viewTreeObserver.dispatchOnPreDraw()
                    val waiting = pixels(view)
                    try { assertTrue("settled reversal must not draw the obsolete size", before.sameAs(waiting)) }
                    finally { waiting.recycle() }
                    val changed = original.rows.toMutableList().apply {
                        this[4] = this[4].copy(cells = this[4].cells.map { it.copy(background = Color.BLUE.toUInt()) })
                    }
                    view.update(original.copy(generation = 3u, geometryGeneration = 3u, rows = changed), 12)
                    val latest = pixels(view)
                    try { assertEquals("matching target resumes current content", Color.BLUE, latest.getPixel(0, 4 * cell)) }
                    finally { latest.recycle() }
                } finally {
                    before.recycle()
                    view.updateImeAnimation(false)
                    view.viewTreeObserver.dispatchOnPreDraw()
                }
            }
        }
    }

    @Test fun authoritativeResizeStillWinsWhenTheFinalLayoutDiffers() {
        instrumentation.runOnMainSync {
            for (screen in NativeActiveScreen.entries) for (cursor in listOf(3, 23)) {
                val original = frame(0, 0, 24, maximum = 0, marker = cursor).copy(
                    viewport = NativeViewport(24u, 16u), activeScreen = screen,
                    cursorVisible = true, cursorRow = cursor.toUShort(), cursorColumn = 15u)
                val view = liveView(original)
                val cell = view.gridCellHeight
                pixels(view).recycle()
                view.updateImeAnimation(true)
                assertTrue(view.prepareImeViewport(view.width, 12 * cell))
                view.layout(0, 0, view.width, 12 * cell)
                val moved = pixels(view)
                // Actual host-model result before any child output (unified-ime-probe.rs).
                val scrolled = maxOf(0, cursor + 1 - 12)
                val resized = original.copy(generation = 2u, geometryGeneration = 2u,
                    viewport = NativeViewport(12u, 16u), rows = original.rows.drop(scrolled).take(12),
                    cursorRow = (cursor - scrolled).toUShort(),
                    historyMaximum = (if (screen == NativeActiveScreen.MAIN) scrolled else 0).toULong())
                view.update(resized, 12)
                val held = pixels(view)
                try { assertTrue("early host resize retains the moved drawing", moved.sameAs(held)) }
                finally { moved.recycle(); held.recycle() }
                view.updateImeAnimation(false)
                view.viewTreeObserver.dispatchOnPreDraw()
                val settled = pixels(view)
                val expected = pixels(liveView(resized))
                try { assertTrue("settled drawing adopts actual host rows and cursor", expected.sameAs(settled)) }
                finally { settled.recycle(); expected.recycle() }
                assertEquals((cursor - scrolled) * cell, markerTop(view))

                view.updateImeAnimation(true)
                assertTrue(view.prepareImeViewport(view.width, 24 * cell))
                view.layout(0, 0, view.width, 24 * cell)
                val closing = pixels(view)
                val restored = if (screen == NativeActiveScreen.MAIN) scrolled else 0
                val blanks = List(12 - restored) { original.rows[0].copy(cells =
                    original.rows[0].cells.map { it.copy(text = "") }) }
                val grown = original.copy(generation = 3u, geometryGeneration = 3u,
                    rows = original.rows.drop(scrolled - restored).take(12 + restored) + blanks,
                    cursorRow = (cursor - scrolled + restored).toUShort())
                view.update(grown, 12)
                val heldClose = pixels(view)
                try { assertTrue("growth with or without history keeps the moving baseline", closing.sameAs(heldClose)) }
                finally { closing.recycle(); heldClose.recycle() }
                view.updateImeAnimation(false)
                view.viewTreeObserver.dispatchOnPreDraw()
                val completed = pixels(view)
                val expectedGrowth = pixels(liveView(grown))
                try { assertTrue("final growth uses actual history or blank rows", expectedGrowth.sameAs(completed)) }
                finally { completed.recycle(); expectedGrowth.recycle() }
            }
        }
    }

    @Test fun imeRetentionNeverCrossesScreenOrInputAuthority() {
        instrumentation.runOnMainSync {
            for (screen in NativeActiveScreen.entries) {
                val original = frame(0, 0, 12, maximum = 0).copy(activeScreen = screen)
                val resized = original.copy(generation = 2u, geometryGeneration = 2u,
                    viewport = NativeViewport(9u, 16u), cursorRow = 8u,
                    rows = original.rows.take(9).map { row -> row.copy(cells =
                        row.cells.map { it.copy(background = Color.BLUE.toUInt()) }) })
                for (replacement in listOf(
                    resized.copy(activeScreen = NativeActiveScreen.entries.first { it != screen }),
                    resized.copy(inputEpoch = 2u),
                    resized.copy(inputReady = false, state = "reconnecting"),
                )) {
                    val view = liveView(original)
                    pixels(view).recycle()
                    view.updateImeAnimation(true)
                    view.layout(0, 0, view.width, 9 * view.gridCellHeight)
                    pixels(view).recycle()
                    view.update(replacement, 12)
                    val changed = pixels(view)
                    try { assertEquals("incompatible authority cannot retain the old drawing", Color.BLUE, changed.getPixel(0, 0)) }
                    finally { changed.recycle() }
                    view.updateImeAnimation(false)
                    view.viewTreeObserver.dispatchOnPreDraw()
                }
            }
        }
    }

    @Test fun knownImeTargetSubmitsEarlyOnceAndFinalLayoutCorrectsIt() {
        instrumentation.runOnMainSync {
            for (screen in NativeActiveScreen.entries) {
                val original = frame(0, 0, 12, maximum = 0).copy(activeScreen = screen)
                val view = liveView(original)
                val cell = view.gridCellHeight
                assertFalse("endpoints outside animation are ignored", view.prepareImeViewport(view.width, 9 * cell))
                view.updateImeAnimation(true)
                assertTrue("target submits while the original height is still displayed", view.prepareImeViewport(view.width, 9 * cell))
                assertEquals(12 * cell, view.height)
                view.layout(0, 0, view.width, 11 * cell)
                assertFalse("intermediate layout did not replace the submitted target", view.prepareImeViewport(view.width, 9 * cell))
                // A cancellation or changed endpoint may settle somewhere else.
                view.layout(0, 0, view.width, 8 * cell)
                view.updateImeAnimation(false)
                view.viewTreeObserver.dispatchOnPreDraw()
                view.updateImeAnimation(true)
                assertFalse("settled layout already corrected the early target", view.prepareImeViewport(view.width, 8 * cell))
                assertTrue("reversal submits its new target", view.prepareImeViewport(view.width, 12 * cell))
                view.layout(0, 0, view.width, 12 * cell)
                view.updateImeAnimation(false)
                view.viewTreeObserver.dispatchOnPreDraw()
            }
        }
    }

    @OptIn(ExperimentalLayoutApi::class)
    @Test fun productionLayoutReportsTheTargetBeforeTheSystemImeFinishes() {
        org.junit.Assume.assumeTrue(android.os.Build.VERSION.SDK_INT >= 30)
        for (screen in NativeActiveScreen.entries) {
            ActivityScenario.launch(MainActivity::class.java).use { scenario ->
                lateinit var activity: MainActivity
                lateinit var view: TerminalView
                val laidOut = CountDownLatch(1)
                var targetReported = CountDownLatch(1)
                var finished = CountDownLatch(1)
                var showing = true
                var early = false
                var targetRows = 0
                var requests = 0
                var started = false
                var stop: (() -> Unit)? = null
                scenario.onActivity {
                    activity = it
                    val owner = ImeAnimationState()
                    androidx.core.view.ViewCompat.setWindowInsetsAnimationCallback(it.window.decorView, owner)
                    view = TerminalView(it).apply {
                        update(frame(0, 0, 12, maximum = 0).copy(activeScreen = screen), 12)
                        addOnLayoutChangeListener { _, left, top, right, bottom, _, _, _, _ ->
                            if (right > left && bottom > top) laidOut.countDown()
                        }
                    }
                    stop = owner.observe { running ->
                        view.updateImeAnimation(running)
                        if (running) started = true else if (started) finished.countDown()
                    }
                    val cell = view.gridCellHeight
                    it.setContent {
                        Box(Modifier.fillMaxSize().statusBarsPadding().navigationBarsPadding().imePadding()) {
                            TerminalGridLayout(cell, owner, Modifier.fillMaxSize(), onImeTarget = { width, height ->
                                if (view.prepareImeViewport(width, height)) {
                                    early = owner.running && if (showing) view.height / cell > height / cell
                                        else view.height / cell < height / cell
                                    targetRows = height / cell
                                    requests++
                                    targetReported.countDown()
                                }
                            }) {
                                Box(Modifier.fillMaxWidth().height(48.dp))
                                AndroidView(factory = { view }, modifier = Modifier.fillMaxSize())
                                HorizontalDivider()
                                Box(Modifier.fillMaxWidth().height(48.dp))
                            }
                        }
                    }
                }
                try {
                    assertTrue("terminal is laid out before showing the real IME", laidOut.await(5, TimeUnit.SECONDS))
                    // Layout can precede window focus after another ActivityScenario.
                    // A real button is tapped only after the editor can be served.
                    var served = false
                    val focusDeadline = SystemClock.uptimeMillis() + 5_000
                    while (!served && SystemClock.uptimeMillis() < focusDeadline) {
                        scenario.onActivity {
                            view.requestFocus()
                            served = view.hasWindowFocus() && it.getSystemService(android.view.inputmethod.InputMethodManager::class.java).isActive(view)
                        }
                        if (!served) SystemClock.sleep(25)
                    }
                    assertTrue("terminal owns a served editor before requesting IME", served)
                    for (show in listOf(true, false)) {
                        scenario.onActivity {
                            showing = show; started = false; early = false; requests = 0
                            targetReported = CountDownLatch(1); finished = CountDownLatch(1)
                            view.setKeyboardVisible(show)
                        }
                        assertTrue("production layout supplies an early target (show=$show)", targetReported.await(10, TimeUnit.SECONDS))
                        assertTrue("real keyboard animation finishes (show=$show)", finished.await(10, TimeUnit.SECONDS))
                        hardwarePixels(activity, view).recycle()
                        assertTrue("resize precedes the remaining keyboard motion (show=$show)", early)
                        assertEquals("one final target, no intermediate requests (show=$show)", 1, requests)
                        assertEquals("early target matches settled layout (show=$show)", targetRows, view.height / view.gridCellHeight)
                    }
                } finally {
                    scenario.onActivity { view.setKeyboardVisible(false); stop?.invoke() }
                }
            }
        }
    }

    @Test fun hardwareRowContentReusesMovedOrdinalsAndIgnoresWrapMetadata() {
        org.junit.Assume.assumeTrue(android.os.Build.VERSION.SDK_INT >= 29)
        instrumentation.runOnMainSync {
            val renderer = TerminalRowRenderer()
            val scene = android.graphics.RenderNode("row-reuse-test")
            val rows = frame(0, 0, 3, maximum = 0).rows
            var recordings = 0
            fun draw(ordinal: Long, row: NativeRow, width: Int = 100) {
                val canvas = scene.beginRecording(width, 20)
                try {
                    renderer.draw(canvas, ordinal, row, width, 20, 4) {
                        recordings++
                        it.drawColor(row.cells[0].background.toInt())
                    }
                } finally { scene.endRecording() }
            }
            try {
                rows.forEachIndexed { i, row -> draw(i.toLong(), row) }
                assertEquals(3, recordings)
                draw(0, rows[1].copy(cells = rows[1].cells.map { it.copy() }, wrapped = !rows[1].wrapped))
                draw(1, rows[2])
                assertEquals("movement and equal copied content reuse text nodes", 3, recordings)
                val changed = rows[2].copy(cells = rows[2].cells.mapIndexed { i, cell ->
                    if (i == 0) cell.copy(background = Color.BLUE.toUInt()) else cell
                })
                draw(1, changed)
                assertEquals("styled cell change records only its row", 4, recordings)
                draw(1, rows[2])
                assertEquals("A-B-A restores cached drawing", 4, recordings)
                draw(1, rows[2], width = 101)
                assertEquals("changed drawing width cannot reuse an old clip", 5, recordings)
                renderer.clear()
                draw(1, rows[2], width = 101)
                assertEquals("discarded lists are recorded again", 6, recordings)
            } finally { renderer.clear(); scene.discardDisplayList() }
        }
    }

    @Test fun hardwareRowsReplaceChangedContentWithoutChangingOtherPixels() {
        org.junit.Assume.assumeTrue(android.os.Build.VERSION.SDK_INT >= 29)
        ActivityScenario.launch(MainActivity::class.java).use { scenario ->
            lateinit var activity: MainActivity
            lateinit var view: TerminalView
            val original = frame(8, 20, 33)
            scenario.onActivity {
                activity = it
                view = TerminalView(it).apply { update(original, 12) }
                it.setContentView(view, android.view.ViewGroup.LayoutParams(320, view.gridCellHeight * 12))
            }
            val before = hardwarePixels(activity, view)
            try {
                val row = 996L - original.firstRow
                val changed = original.rows.toMutableList().apply {
                    this[row.toInt()] = this[row.toInt()].copy(cells = this[row.toInt()].cells.map {
                        it.copy(text = "W", background = Color.BLUE.toUInt())
                    })
                }
                scenario.onActivity { view.update(original.copy(generation = 100u, rows = changed), 12) }
                val after = hardwarePixels(activity, view)
                try {
                    val top = 4 * view.gridCellHeight
                    val bottom = top + view.gridCellHeight
                    assertEquals(Color.MAGENTA, after.getPixel(0, 3 * view.gridCellHeight))
                    assertEquals(Color.BLUE, after.getPixel(0, top))
                    for (y in 0 until before.height) if (y !in top until bottom) {
                        for (x in 0 until before.width) assertEquals("unchanged pixel ($x,$y)", before.getPixel(x,y), after.getPixel(x,y))
                    }
                } finally { after.recycle() }
                scenario.onActivity { view.update(original.copy(generation = 101u), 12) }
                val restored = hardwarePixels(activity, view)
                try { assertTrue("A-B-A content restores the exact image", before.sameAs(restored)) }
                finally { restored.recycle() }
            } finally { before.recycle() }
        }
    }

    @Test fun hardwareHeightChangesAndResizeFramesReuseTheMovedRows() {
        org.junit.Assume.assumeTrue(android.os.Build.VERSION.SDK_INT >= 29)
        for (screen in NativeActiveScreen.entries) {
            ActivityScenario.launch(MainActivity::class.java).use { scenario ->
                lateinit var activity: MainActivity
                lateinit var view: TerminalView
                val original = frame(0, 0, 12, maximum = 0, marker = 5).copy(
                    activeScreen = screen, cursorVisible = true, cursorRow = 10u)
                scenario.onActivity {
                    activity = it
                    view = liveView(original, it)
                    it.setContentView(view, android.view.ViewGroup.LayoutParams(view.width, view.height))
                }
                val before = hardwarePixels(activity, view)
                val warm = view.rowRecordingCount
                assertTrue("warm actual text display lists", warm >= 12)
                try {
                    scenario.onActivity {
                        view.updateImeAnimation(true)
                        view.layoutParams = view.layoutParams.apply { height = 9 * view.gridCellHeight }
                    }
                    val moved = hardwarePixels(activity, view)
                    assertEquals("local height change reuses every row", warm, view.rowRecordingCount)
                    val resized = original.copy(generation = 2u, geometryGeneration = 2u,
                        viewport = original.viewport.copy(rows = 9u), rows = original.rows.drop(3), cursorRow = 7u)
                    scenario.onActivity { view.update(resized.copy(rows = original.rows.drop(2).take(9), cursorRow = 8u), 12) }
                    val intermediate = hardwarePixels(activity, view)
                    try {
                        assertTrue("resize-only candidate never displaces the displayed baseline", moved.sameAs(intermediate))
                        assertEquals(warm, view.rowRecordingCount)
                    } finally { intermediate.recycle() }
                    scenario.onActivity { view.update(resized.copy(generation = 3u), 12); view.updateImeAnimation(false) }
                    val received = hardwarePixels(activity, view)
                    try {
                        assertTrue("delayed new-size frame equals the locally moved pixels", moved.sameAs(received))
                        assertEquals("new geometry and ordinals do not record equal rows", warm, view.rowRecordingCount)
                    } finally { moved.recycle(); received.recycle() }
                    val changed = resized.rows.toMutableList().apply {
                        this[4] = this[4].copy(cells = this[4].cells.map { it.copy(background = Color.BLUE.toUInt()) })
                    }
                    scenario.onActivity { view.update(resized.copy(generation = 4u, rows = changed), 12) }
                    val patched = hardwarePixels(activity, view)
                    try {
                        assertEquals(Color.BLUE, patched.getPixel(0, 4 * view.gridCellHeight))
                        assertEquals("one changed row records once", warm + 1, view.rowRecordingCount)
                    } finally { patched.recycle() }
                    scenario.onActivity {
                        view.layoutParams = view.layoutParams.apply { height = 12 * view.gridCellHeight }
                    }
                    hardwarePixels(activity, view).recycle()
                    assertEquals("keyboard close also keeps recorded content", warm + 1, view.rowRecordingCount)
                    scenario.onActivity { view.update(original.copy(generation = 5u, geometryGeneration = 3u), 12) }
                    val restored = hardwarePixels(activity, view)
                    try {
                        assertTrue("larger authoritative grid restores exact original pixels", before.sameAs(restored))
                        // Android may discard lists that have left the displayed
                        // scene. Three newly exposed rows and the blue row changed;
                        // the eight rows still showing equal content remain reusable.
                        assertTrue("restoration records at most the four differing rows",
                            view.rowRecordingCount in (warm + 1)..(warm + 5))
                    } finally { restored.recycle() }
                } finally { before.recycle() }
            }
        }
    }

    @Test fun hardwareRowLayersMatchDirectPaintingAndKeepSharedRowsSeamless() {
        org.junit.Assume.assumeTrue(android.os.Build.VERSION.SDK_INT >= 29)
        ActivityScenario.launch(MainActivity::class.java).use { scenario ->
            lateinit var activity: MainActivity
            lateinit var view: android.view.View
            val renderer = TerminalRowRenderer()
            val rowWidth = 288
            val rowHeight = 48
            val base = NativeCell("M", 1u, Color.WHITE.toUInt(), Color.DKGRAY.toUInt(), 0u, 0u, Color.WHITE.toUInt())
            val shared = NativeRow(listOf(
                base, base.copy(text = "W", attributes = 1u), base.copy(text = "f", attributes = 4u),
                base.copy(text = "e\u0301", attributes = 2u), base.copy(text = "界", width = 2u),
                base.copy(text = "", width = 0u), base.copy(text = "A", foreground = Color.CYAN.toUInt()),
                base.copy(text = " ", background = Color.BLUE.toUInt()), base.copy(text = "x"),
                base.copy(text = " "), base.copy(text = " "), base.copy(text = " "),
            ), false)
            var rows = listOf(shared, shared, shared)
            var shift = 0f
            scenario.onActivity {
                activity = it
                view = object : android.view.View(it) {
                    val paint = android.graphics.Paint(android.graphics.Paint.ANTI_ALIAS_FLAG).apply {
                        typeface = android.graphics.Typeface.MONOSPACE; textSize = 28f
                    }
                    fun paintRow(canvas: Canvas, row: NativeRow) {
                        row.cells.forEachIndexed { column, cell ->
                            if (cell.width == 0.toUByte()) return@forEachIndexed
                            val x = column * 24f
                            val right = x + cell.width.toInt() * 24f
                            paint.color = cell.background.toInt(); paint.alpha = 255
                            canvas.drawRect(x, 0f, right, rowHeight.toFloat(), paint)
                            paint.color = cell.foreground.toInt()
                            paint.alpha = if (cell.attributes.toInt() and 2 != 0) 150 else 255
                            paint.isFakeBoldText = cell.attributes.toInt() and 1 != 0
                            paint.textSkewX = if (cell.attributes.toInt() and 4 != 0) -.2f else 0f
                            canvas.save(); canvas.clipRect(x, 0f, right, rowHeight.toFloat())
                            canvas.drawText(cell.text, x, 35f, paint); canvas.restore()
                            paint.alpha = 255; paint.isFakeBoldText = false; paint.textSkewX = 0f
                        }
                    }
                    override fun onDraw(canvas: Canvas) {
                        canvas.drawColor(Color.BLACK)
                        // Direct hardware painting is the reference in the left half;
                        // the right half uses the production row cache in the same frame.
                        repeat(2) { half ->
                            canvas.save(); canvas.translate(half * rowWidth.toFloat(), 0f)
                            canvas.clipRect(0, 0, rowWidth, height); canvas.translate(0f, shift)
                            rows.forEachIndexed { index, row ->
                                canvas.save(); canvas.translate(0f, index * rowHeight.toFloat())
                                if (half == 0) paintRow(canvas, row)
                                else renderer.draw(canvas, index.toLong(), row, rowWidth, rowHeight, 10) { paintRow(it, row) }
                                canvas.restore()
                            }
                            canvas.restore()
                        }
                    }
                }
                it.setContentView(android.widget.FrameLayout(it).apply {
                    addView(view, android.widget.FrameLayout.LayoutParams(rowWidth * 2, rowHeight * 3).apply {
                        topMargin = (64 * resources.displayMetrics.density).toInt()
                    })
                })
            }
            fun compare(integerShift: Float) {
                scenario.onActivity { shift = integerShift }
                val bitmap = hardwarePixels(activity, view)
                try {
                    for (y in 0 until bitmap.height) for (x in 0 until rowWidth)
                        assertEquals("cached row equals direct painting at $x,$y shift=$shift",
                            bitmap.getPixel(x, y), bitmap.getPixel(x + rowWidth, y))
                } finally { bitmap.recycle() }
            }
            try {
                compare(0f); compare(-7f); compare(5f)
                assertEquals("identical row content shares one cached layer across positions", 1L, renderer.recordingCount)
                scenario.onActivity { shift = -.5f }
                val fractional = hardwarePixels(activity, view)
                try {
                    // Fractional text can be resampled; flat interiors at row joins
                    // must remain solid, without a dark gap between cached layers.
                    // Direct antialiased rectangles can themselves darken a join,
                    // so the declared background is the reference for this case.
                    for (join in listOf(rowHeight, rowHeight * 2)) for (y in join - 2..join + 1)
                        assertEquals("no fractional row seam at $y", Color.DKGRAY, fractional.getPixel(rowWidth + 250, y))
                } finally { fractional.recycle() }
                scenario.onActivity {
                    rows = listOf(shared, shared.copy(cells = shared.cells.mapIndexed { index, cell ->
                        if (index == 6) cell.copy(text = "B", background = Color.RED.toUInt()) else cell
                    }), shared)
                }
                compare(0f)
                assertEquals("only changed row content is recorded", 2L, renderer.recordingCount)
                scenario.onActivity { renderer.clear() }
                compare(0f)
                assertEquals("discarded cached layers recover through the painter", 4L, renderer.recordingCount)
            } finally { scenario.onActivity { renderer.clear() } }
        }
    }

    private fun hardwarePixels(activity: MainActivity, view: android.view.View): Bitmap {
        val drawn = CountDownLatch(1)
        instrumentation.runOnMainSync { view.invalidate(); view.postOnAnimation { view.postOnAnimation { drawn.countDown() } } }
        assertTrue(drawn.await(5, TimeUnit.SECONDS))
        assertTrue("exercise RenderNode hardware path", view.isHardwareAccelerated)
        val bitmap = Bitmap.createBitmap(view.width, view.height, Bitmap.Config.ARGB_8888)
        val copied = CountDownLatch(1)
        var result = -1
        instrumentation.runOnMainSync {
            val location = IntArray(2); view.getLocationInWindow(location)
            PixelCopy.request(activity.window, Rect(location[0], location[1], location[0]+view.width, location[1]+view.height),
                bitmap, { result = it; copied.countDown() }, Handler(Looper.getMainLooper()))
        }
        assertTrue(copied.await(5, TimeUnit.SECONDS))
        assertEquals(PixelCopy.SUCCESS, result)
        return bitmap
    }

    private fun view(frame: NativeFrame) = TerminalView(instrumentation.targetContext).also {
        it.update(frame, 12)
        it.layout(0, 0, 320, 12 * it.gridCellHeight)
    }

    private fun liveView(frame: NativeFrame, context: android.content.Context = instrumentation.targetContext) = TerminalView(context).also {
        it.update(frame, 12)
        val paint = android.graphics.Paint().apply {
            typeface = android.graphics.Typeface.MONOSPACE
            textSize = android.util.TypedValue.applyDimension(android.util.TypedValue.COMPLEX_UNIT_SP,
                12f, it.resources.displayMetrics)
        }
        val width = kotlin.math.ceil(paint.measureText("M") * frame.viewport.columns.toInt()).toInt()
        it.layout(0, 0, width, frame.viewport.rows.toInt() * it.gridCellHeight)
    }

    private fun pixels(view: TerminalView) = Bitmap.createBitmap(view.width, view.height,
        Bitmap.Config.ARGB_8888).also { view.draw(Canvas(it)) }

    private fun frame(offset: Int, windowOffset: Int, count: Int, maximum: Int = 1000, marker: Int = 995): NativeFrame {
        val first = maximum - windowOffset
        return NativeFrame(
        applicationTitle = "",
        cursorShape = NativeCursorShape.BLOCK, cursorBlinking = false,
            connectionPath = NativeConnectionPath.UNKNOWN, rttMs = null,
            pointerMode = NativePointerMode.NONE, activeScreen = NativeActiveScreen.MAIN, source = null,
            stats = NativeNavigationStats(0u, 0u, 0u, 0u, 0u, 0u, 0u), notice = null,
            inputEpoch = 1u, geometryGeneration = 1u, inputReady = true,
            historyOffset = offset.toULong(), windowOffset = windowOffset.toULong(), contentGeneration = 0u,
            firstRow = first.toLong(), selection = null,
            generation = offset.toULong(), state = "active", error = null,
            viewport = NativeViewport(12u, 16u),
            rows = List(count) { row ->
                val background = if (first + row == marker) Color.MAGENTA else Color.BLACK
                NativeRow("ROW ${first + row}".padEnd(16).map { character ->
                    NativeCell(character.toString(), 1u, Color.WHITE.toUInt(), background.toUInt(), 0u, 0u, Color.WHITE.toUInt())
                }, false)
            },
            cursorRow = 11u, cursorColumn = 0u, cursorVisible = false,
            cursorColor = Color.WHITE.toUInt(), background = Color.BLACK.toUInt(), historyMaximum = maximum.toULong(),
        )
    }

    private fun markerTop(view: TerminalView): Int {
        val bitmap = Bitmap.createBitmap(view.width, view.height, Bitmap.Config.ARGB_8888)
        try {
            view.draw(Canvas(bitmap))
            val row = (0 until bitmap.height).firstOrNull { bitmap.getPixel(0, it) == Color.MAGENTA }
            assertNotNull("logical marker row is visible in the actual Canvas", row)
            return row!!
        } finally { bitmap.recycle() }
    }

    private class Drag(private val view: TerminalView, private val startY: Float) {
        private val down = SystemClock.uptimeMillis()
        private var time = down
        private var y = startY
        init { event(MotionEvent.ACTION_DOWN) }
        fun move(nextY: Float) { y = nextY; time += 30; event(MotionEvent.ACTION_MOVE) }
        fun cancel() { time += 30; event(MotionEvent.ACTION_CANCEL) }
        private fun event(action: Int) {
            val event = MotionEvent.obtain(down, time, action, 100f, y, 0)
            try { view.onTouchEvent(event) } finally { event.recycle() }
        }
    }
}
