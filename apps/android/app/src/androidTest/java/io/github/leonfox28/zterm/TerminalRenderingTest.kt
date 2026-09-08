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
import io.github.leonfox28.zterm.nativebridge.*
import org.junit.Assert.*
import org.junit.Test
import java.util.concurrent.CountDownLatch
import java.util.concurrent.TimeUnit

/** Draw real cells and dispatch real gestures, with native frame delivery held at a row boundary. */
class TerminalRenderingTest {
    private val instrumentation = InstrumentationRegistry.getInstrumentation()

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

    private fun hardwarePixels(activity: MainActivity, view: TerminalView): Bitmap {
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

    private fun frame(offset: Int, windowOffset: Int, count: Int, maximum: Int = 1000, marker: Int = 995): NativeFrame {
        val first = maximum - windowOffset
        return NativeFrame(
            pointerMode = NativePointerMode.NONE, source = null,
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
