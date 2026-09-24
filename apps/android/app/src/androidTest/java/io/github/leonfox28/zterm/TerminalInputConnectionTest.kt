package io.github.leonfox28.zterm

import android.text.Selection
import android.view.inputmethod.BaseInputConnection
import android.view.inputmethod.EditorInfo
import androidx.test.platform.app.InstrumentationRegistry
import io.github.leonfox28.zterm.nativebridge.*
import org.junit.Assert.*
import org.junit.Test

/** The production editor contract without an IME brand, transport or live host. */
class TerminalInputConnectionTest {
    private val instrumentation = InstrumentationRegistry.getInstrumentation()

    @Test fun composingReplacementKeepsCursorQueriesAndSpansConsistent() {
        instrumentation.runOnMainSync {
            val view = TerminalView(instrumentation.targetContext).apply { update(frame(), 12) }
            val connection = view.onCreateInputConnection(EditorInfo()) as BaseInputConnection
            assertTrue(connection.setComposingText("n", 1))
            assertEquals("IME must see the Pinyin before its cursor", "n", connection.getTextBeforeCursor(16, 0).toString())
            assertEquals("", connection.getTextAfterCursor(16, 0).toString())
            assertEditor(connection, "n", 1, 1, 0, 1)
            assertTrue(connection.setComposingText("ni", 1))
            assertEditor(connection, "ni", 2, 2, 0, 2)

            assertTrue(connection.setComposingText("nihao", 0))
            assertEditor(connection, "nihao", 0, 0, 0, 5)
            assertTrue(connection.setSelection(2, 2))
            assertEquals("ni", connection.getTextBeforeCursor(16, 0).toString())
            assertEquals("hao", connection.getTextAfterCursor(16, 0).toString())
            val info = EditorInfo()
            view.onCreateInputConnection(info)
            assertEquals(2, info.initialSelStart); assertEquals(2, info.initialSelEnd)

            assertTrue(connection.setComposingRegion(2, 5))
            assertTrue(connection.setComposingText("好", 1))
            assertEditor(connection, "ni好", 3, 3, 2, 3)
            assertTrue(connection.setSelection(0, 2))
            assertEquals("ni", connection.getSelectedText(0).toString())
        }
    }

    @Test fun surroundingDeletionRespectsSelectionCompositionAndCodePoints() {
        instrumentation.runOnMainSync {
            val view = TerminalView(instrumentation.targetContext).apply { update(frame(), 12) }
            val connection = view.onCreateInputConnection(EditorInfo()) as BaseInputConnection
            assertTrue(connection.setComposingText("A😀BC", 1))
            // Android deletion excludes the selection and composing range. Here
            // C remains composing; the emoji before the cursor must be atomic.
            assertTrue(connection.setComposingRegion(4, 5))
            assertTrue(connection.setSelection(3, 3))
            assertTrue(connection.deleteSurroundingTextInCodePoints(1, 0))
            assertEditor(connection, "ABC", 1, 1, 2, 3)
            assertTrue(connection.setComposingRegion(1, 2))
            assertTrue(connection.deleteSurroundingText(1, 1))
            assertEditor(connection, "B", 0, 0, 0, 1)
            assertFalse(connection.deleteSurroundingText(-1, 0))
            assertFalse(connection.deleteSurroundingTextInCodePoints(129, 0))
        }
    }

    @Test fun rejectedCommitsAndRetiredConnectionsPreserveTheOwningBuffer() {
        instrumentation.runOnMainSync {
            val view = TerminalView(instrumentation.targetContext).apply { update(frame(), 12) }
            val connection = view.onCreateInputConnection(EditorInfo()) as BaseInputConnection
            assertTrue(connection.beginBatchEdit())
            assertTrue(connection.beginBatchEdit())
            assertTrue(connection.setComposingText("n😀", 1))
            assertTrue(connection.endBatchEdit())
            assertFalse(connection.endBatchEdit())
            assertFalse(connection.endBatchEdit())
            assertTrue(connection.setSelection(1, 1))
            // There is no Repository to admit text: the exact editor state must
            // survive both candidate commit and explicit finalization failure.
            assertFalse(connection.commitText("你", 1))
            assertFalse(connection.finishComposingText())
            assertEditor(connection, "n😀", 1, 1, 0, 3)

            view.update(frame().copy(inputReady = false), 12)
            assertFalse(connection.setComposingText("lost", 1))
            assertFalse(connection.setSelection(0, 0))
            assertFalse(connection.setComposingRegion(0, 1))
            assertEditor(connection, "n😀", 1, 1, 0, 3)

            view.update(frame().copy(inputEpoch = 2u), 12)
            val fresh = view.onCreateInputConnection(EditorInfo()) as BaseInputConnection
            assertEditor(fresh, "", 0, 0, -1, -1)
            assertTrue(fresh.setComposingText("new", 1))
            assertFalse(connection.commitText("old", 1))
            assertFalse(connection.setSelection(0, 0))
            assertFalse(connection.setComposingRegion(0, 1))
            assertFalse(connection.deleteSurroundingText(1, 0))
            requireNotNull(connection.editable).append("retired")
            connection.closeConnection()
            assertEditor(fresh, "new", 3, 3, 0, 3)
        }
    }

    private fun assertEditor(connection: BaseInputConnection, text: String, start: Int, end: Int, composingStart: Int, composingEnd: Int) {
        val editable = requireNotNull(connection.editable)
        assertEquals(text, editable.toString())
        assertEquals(start, Selection.getSelectionStart(editable))
        assertEquals(end, Selection.getSelectionEnd(editable))
        assertEquals(composingStart, BaseInputConnection.getComposingSpanStart(editable))
        assertEquals(composingEnd, BaseInputConnection.getComposingSpanEnd(editable))
    }

    private fun frame() = NativeFrame(
        applicationTitle = "",
        cursorShape = NativeCursorShape.BLOCK, cursorBlinking = false,
        connectionPath = NativeConnectionPath.UNKNOWN, rttMs = null,
        pointerMode = NativePointerMode.NONE, activeScreen = NativeActiveScreen.MAIN, source = null,
        stats = NativeNavigationStats(0u, 0u, 0u, 0u, 0u, 0u, 0u), notice = null,
        inputEpoch = 1u, geometryGeneration = 1u, inputReady = true,
        historyOffset = 0u, windowOffset = 0u, contentGeneration = 0u,
        firstRow = 0, selection = null, generation = 1u, state = "active", error = null,
        viewport = NativeViewport(3u, 16u), rows = emptyList(), cursorRow = 0u, cursorColumn = 0u,
        cursorVisible = true, cursorColor = 0xffffffffu, background = 0xff000000u, historyMaximum = 0u,
    )
}
