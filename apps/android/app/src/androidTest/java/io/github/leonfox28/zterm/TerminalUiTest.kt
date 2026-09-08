package io.github.leonfox28.zterm

import android.content.ClipboardManager
import android.os.SystemClock
import android.view.KeyEvent
import android.view.MotionEvent
import android.view.View
import android.view.ViewGroup
import android.view.accessibility.AccessibilityNodeInfo
import android.view.inputmethod.EditorInfo
import androidx.compose.ui.test.*
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.lifecycle.Lifecycle
import androidx.test.platform.app.InstrumentationRegistry
import kotlinx.coroutines.runBlocking
import org.junit.Assert.*
import org.junit.Rule
import org.junit.Test

/** Exercises the production View/IME/ActionMode against a task-owned host Session. */
class TerminalUiTest {
    @get:Rule val ui = createAndroidComposeRule<MainActivity>()
    private val instrumentation = InstrumentationRegistry.getInstrumentation()
    private val repository get() = (ui.activity.application as ZtermApplication).repository

    @Test fun keyboardDismissalSurvivesTaskResume() {
        val hostName = InstrumentationRegistry.getArguments().getString("hostName")
        org.junit.Assume.assumeTrue("explicit disposable host", hostName?.startsWith("presentation-") == true)
        ui.waitUntil(20_000) { repository.state.value.initialized }
        val host = repository.state.value.saved.hosts.first { it.name == hostName }
        val preferences = repository.state.value.saved.preferences
        val name = "android-keyboard-${System.nanoTime()}"
        var owned: String? = null
        try {
            ui.runOnIdle { repository.goHome(); repository.setPreferences(Preferences("en", "dark", 12)) }
            ui.waitUntil { repository.frame.value == null && !repository.state.value.busy }
            val originalMode = ui.runOnIdle { ui.activity.window.attributes.softInputMode }
            ui.runOnIdle { repository.connectHost(host.id) }
            ui.waitUntil(20_000) { repository.state.value.route == Route.Terminal && !repository.state.value.busy }
            ui.runOnIdle { repository.createSession(name, "") }
            await("keyboard regression Session") { repository.frame.value?.state == "active" && repository.state.value.sessions.any { it.name == name } && !repository.state.value.busy }
            owned = repository.state.value.sessionId
            await("initial keyboard geometry") { geometrySettled() && terminal().hasWindowFocus() }
            val viewport = repository.frame.value!!.viewport
            val epoch = repository.frame.value!!.inputEpoch
            resumeTerminalTaskWithKeyboard(false)

            for (back in listOf(false, true)) {
                ui.onNodeWithContentDescription("Show keyboard").performClick()
                await("keyboard explicitly opened") { keyboardVisible() && geometrySettled() && repository.frame.value!!.viewport.rows < viewport.rows }
                if (back) instrumentation.sendKeyDownUpSync(KeyEvent.KEYCODE_BACK)
                else ui.onNodeWithContentDescription("Hide keyboard").performClick()
                await("keyboard explicitly dismissed") { !keyboardVisible() && geometrySettled() && repository.frame.value!!.viewport == viewport }
                repeat(2) { resumeTerminalTaskWithKeyboard(false) }
                assertTrue("hidden IME retains terminal input focus", ui.runOnIdle { terminal().hasFocus() })
                assertEquals("same Session after resume", owned, repository.state.value.sessionId)
                assertEquals("keyboard lifetime survives resume", epoch, repository.frame.value!!.inputEpoch)
            }

            // Deliver actual key events while the software keyboard is hidden.
            instrumentation.sendStringSync("printf 'KEYBOARD_%s\\n' 'RESUMED'")
            instrumentation.sendKeyDownUpSync(KeyEvent.KEYCODE_ENTER)
            await("hardware key input after resume") { content().contains("KEYBOARD_RESUMED") }
            // Gboard can explicitly show itself in response to injected virtual-keyboard events.
            if (keyboardVisible()) instrumentation.sendKeyDownUpSync(KeyEvent.KEYCODE_BACK)
            await("key injection settled") { !keyboardVisible() && geometrySettled() }
            ui.onNodeWithContentDescription("Show keyboard").performClick()
            await("keyboard can reopen after resume") { keyboardVisible() && geometrySettled() }
            resumeTerminalTaskWithKeyboard(true)
            ui.runOnIdle {
                val connection = terminal().onCreateInputConnection(EditorInfo())
                assertTrue(connection.commitText("printf 'RESUME_%s\\n' '中文'", 1))
                assertTrue(connection.performEditorAction(EditorInfo.IME_ACTION_NONE))
            }
            await("IME input after visible resume") { content().contains("RESUME_中文") }
            ui.onNodeWithContentDescription("Hide keyboard").performClick()
            await("keyboard closed after input") { !keyboardVisible() && geometrySettled() }
            tap(terminal(), .4f, .4f)
            resumeTerminalTaskWithKeyboard(false)

            ui.runOnIdle { repository.goHome() }
            ui.waitUntil { repository.frame.value == null }
            assertEquals("leaving Terminal restores the prior visibility policy",
                originalMode and android.view.WindowManager.LayoutParams.SOFT_INPUT_MASK_STATE,
                ui.runOnIdle { ui.activity.window.attributes.softInputMode } and android.view.WindowManager.LayoutParams.SOFT_INPUT_MASK_STATE)
        } finally {
            ui.runOnIdle { repository.goHome(); repository.setPreferences(preferences) }
            ui.waitUntil(15_000) { repository.frame.value == null }
            owned?.let { id -> runBlocking { repository.runtime.closeSession(host.id, id) } }
        }
    }

    private fun resumeTerminalTaskWithKeyboard(visible: Boolean) {
        val activity = ui.activity
        val task = activity.taskId
        ui.runOnIdle { assertTrue("task moved to background", activity.moveTaskToBack(true)) }
        ui.waitUntil(5_000) { !activity.hasWindowFocus() && !activity.lifecycle.currentState.isAtLeast(Lifecycle.State.STARTED) }
        val component = android.content.ComponentName(activity, MainActivity::class.java).flattenToString()
        android.os.ParcelFileDescriptor.AutoCloseInputStream(instrumentation.uiAutomation.executeShellCommand(
            "am start -a android.intent.action.MAIN -c android.intent.category.LAUNCHER -n $component"
        )).use { it.readBytes() }
        await("same task returned with settled IME") { activity.hasWindowFocus() && activity.lifecycle.currentState == Lifecycle.State.RESUMED && geometrySettled() }
        assertSame("resume does not recreate Activity", activity, ui.activity)
        assertEquals(task, activity.taskId)
        // Observe a settled interval: an immediate hidden sample can precede auto-show.
        val until = SystemClock.uptimeMillis() + 1_000
        while (SystemClock.uptimeMillis() < until) {
            assertEquals("task resume preserves keyboard visibility", visible, keyboardVisible())
            SystemClock.sleep(25)
        }
    }

    @Test fun keyboardPresentationAndCompositionStayContinuous() {
        val hostName = InstrumentationRegistry.getArguments().getString("hostName")
        org.junit.Assume.assumeTrue("explicit disposable host", hostName?.startsWith("presentation-") == true)
        ui.waitUntil(20_000) { repository.state.value.initialized }
        val host = repository.state.value.saved.hosts.first { it.name == hostName }
        val preferences = repository.state.value.saved.preferences
        val font = InstrumentationRegistry.getArguments().getString("fontSize")?.toInt() ?: 14
        require(font in terminalFontSizes)
        val name = "android-continuity-${System.nanoTime()}"
        var owned: String? = null
        var stop: (() -> Unit)? = null
        try {
            ui.runOnIdle { repository.goHome(); repository.setPreferences(Preferences("en", "dark", font)) }
            ui.waitUntil { repository.frame.value == null && !repository.state.value.busy }
            ui.runOnIdle { repository.connectHost(host.id) }
            ui.waitUntil(20_000) { repository.state.value.route == Route.Terminal && !repository.state.value.busy }
            ui.runOnIdle { repository.createSession(name, "") }
            await("continuity Session") { repository.frame.value?.state == "active" && repository.state.value.sessions.any { it.name == name } && !repository.state.value.busy }
            owned = repository.state.value.sessionId
            await("settled initial grid") { geometrySettled() && terminal().hasWindowFocus() }
            val epoch = repository.frame.value!!.inputEpoch
            val readiness = mutableListOf<Boolean>()
            stop = ui.runOnIdle { repository.observeTerminalFrames { if (it != null) readiness.add(it.inputReady) } }
            val connection = ui.runOnIdle { terminal().onCreateInputConnection(EditorInfo()) }
            ui.runOnIdle {
                assertTrue(connection.setComposingText("unused", 1))
                assertTrue(connection.setComposingText("printf 'COMPOSITION_%s\\n' '中文'", 1))
            }
            ui.onNodeWithContentDescription("Show keyboard").performClick()
            await("composition resize") { keyboardVisible() && geometrySettled() }
            assertEquals(epoch, repository.frame.value!!.inputEpoch)
            ui.runOnIdle {
                assertTrue(connection.finishComposingText())
                assertTrue(connection.finishComposingText()) // Finishing twice must not duplicate text.
                assertTrue(connection.performEditorAction(EditorInfo.IME_ACTION_NONE))
            }
            await("Chinese committed exactly once") { content().contains("COMPOSITION_中文") }
            assertEquals(1, content().split("COMPOSITION_中文").size - 1)
            ui.onNodeWithContentDescription("Hide keyboard").performClick()
            await("composition keyboard closed") { !keyboardVisible() && geometrySettled() }
            assertEquals(epoch, repository.frame.value!!.inputEpoch)
            assertTrue("healthy resize never suspends keyboard admission", readiness.all { it })
            ui.runOnIdle { stop?.invoke(); stop = null }

            for (mode in listOf("high", "low", "tui")) {
                val fixture = "mode='$mode'\n" + continuityFixture
                val encoded = android.util.Base64.encodeToString(fixture.toByteArray(), android.util.Base64.NO_WRAP)
                ui.runOnIdle { assertTrue(repository.text("python3 -c 'import base64; exec(base64.b64decode(\"$encoded\"))'\r")) }
                await("$mode scene") { geometrySettled() && content().startsWith("Tabs | One | Two") }
                val oldRows = repository.frame.value!!.viewport.rows.toInt()
                val before = captureContinuityGrid("$mode-before")
                assertEquals(0, terminal().height % terminal().gridCellHeight)
                val oldEpoch = repository.frame.value!!.inputEpoch
                ui.onNodeWithContentDescription("Show keyboard").performClick()
                await("$mode keyboard open") { keyboardVisible() && geometrySettled() && repository.frame.value!!.viewport.rows.toInt() < oldRows }
                val newRows = repository.frame.value!!.viewport.rows.toInt()
                if (mode == "tui") await("marked header restored") { content().startsWith("Tabs | One | Two") }
                val after = captureContinuityGrid("$mode-open")
                assertEquals(0, terminal().height % terminal().gridCellHeight)
                assertEquals(oldEpoch, repository.frame.value!!.inputEpoch)
                val pan = if (mode == "high") 0 else oldRows - newRows
                val start = if (mode == "tui") 1 else 0
                val cell = terminal().gridCellHeight
                val width = minOf(before.width, after.width) - 32 // Exclude the fading local scrollbar.
                val height = minOf(5, newRows - start - 1) * cell
                val oldPatch = android.graphics.Bitmap.createBitmap(before, 0, (pan + start) * cell, width, height)
                val newPatch = android.graphics.Bitmap.createBitmap(after, 0, start * cell, width, height)
                assertTrue("$mode preserves unchanged pixels after host resize", oldPatch.sameAs(newPatch))
                oldPatch.recycle(); newPatch.recycle(); before.recycle(); after.recycle()
                ui.onNodeWithContentDescription("Hide keyboard").performClick()
                await("$mode keyboard closed") { !keyboardVisible() && geometrySettled() && repository.frame.value!!.viewport.rows.toInt() == oldRows }
                ui.runOnIdle { assertTrue(repository.text("\r")) }
                await("$mode fixture ended") { content().contains("SCENE_DONE") }
            }
            ui.runOnIdle { assertTrue(repository.text("printf '\\033[2J\\033[H'; read -r unused\r")) }
            await("legitimate final clear remains visible") { repository.frame.value?.let { visibleRows(it) }?.all { row -> row.cells.all { it.text.isBlank() } } == true }
            captureContinuityGrid("final-clear").recycle()
            ui.runOnIdle { repository.text("\r") }
        } finally {
            ui.runOnIdle { stop?.invoke(); repository.goHome(); repository.setPreferences(preferences) }
            ui.waitUntil(15_000) { repository.frame.value == null }
            owned?.let { id -> runBlocking { repository.runtime.closeSession(host.id, id) } }
        }
    }

    private fun captureContinuityGrid(name: String): android.graphics.Bitmap {
        awaitDraw(terminal())
        val view = terminal()
        val origin = IntArray(2)
        ui.runOnIdle { view.getLocationOnScreen(origin) }
        val screenshot = requireNotNull(instrumentation.uiAutomation.takeScreenshot())
        val grid = android.graphics.Bitmap.createBitmap(screenshot, origin[0], origin[1], view.width, view.height)
        screenshot.recycle()
        java.io.File(instrumentation.targetContext.cacheDir, "continuity-$name.png").outputStream().use {
            grid.compress(android.graphics.Bitmap.CompressFormat.PNG, 100, it)
        }
        android.util.Log.i("ZtermAcceptance", "continuity=$name height=${view.height} cell=${view.gridCellHeight} rows=${repository.frame.value?.viewport?.rows}")
        return grid
    }

    private val continuityFixture = """
import os, signal, sys
initial = os.get_terminal_size().lines
def draw(*_):
    rows = os.get_terminal_size().lines
    out = '\x1b[?2026h' if mode == 'tui' else ''
    out += '\x1b[2J\x1b[H'
    for row in range(rows):
        label = 'Tabs | One | Two' if row == 0 else 'ROW %02d   stable terminal content' % (row + initial - rows)
        out += '\x1b[%d;1H%s' % (row + 1, label)
    cursor = min(10, rows - 1) if mode == 'high' else rows - 1
    out += '\x1b[%d;3H' % (cursor + 1)
    if mode == 'tui': out += '\x1b[?2026l'
    sys.stdout.write(out); sys.stdout.flush()
if mode == 'tui':
    sys.stdout.write('\x1b[?1049h')
    signal.signal(signal.SIGWINCH, draw)
draw()
input()
if mode == 'tui': sys.stdout.write('\x1b[?1049l')
print('SCENE_DONE')
""".trimIndent()

    @Test fun gesturesCopyOverlayAndActivityRetention() {
        instrumentation.uiAutomation.serviceInfo = instrumentation.uiAutomation.serviceInfo.apply {
            flags = flags or android.accessibilityservice.AccessibilityServiceInfo.FLAG_RETRIEVE_INTERACTIVE_WINDOWS
        }
        ui.waitUntil(20_000) { repository.state.value.initialized }
        val preferences = repository.state.value.saved.preferences
        val host = repository.state.value.saved.hosts.first { it.name == (InstrumentationRegistry.getArguments().getString("hostName") ?: "my-mac") }
        val name = "android-ui-${System.nanoTime()}"
        var owned: String? = null
        var stopKeyboardFrames: (() -> Unit)? = null
        try {
            ui.runOnIdle { repository.setPreferences(Preferences("en", "dark", 12)); repository.goHome() }
            ui.waitUntil { repository.state.value.saved.preferences.language == "en" && !repository.state.value.busy }
            ui.onAllNodesWithText(host.name).onLast().performClick()
            ui.waitUntil(20_000) { repository.state.value.route == Route.Terminal && !repository.state.value.busy }
            if (!repository.state.value.panel) {
                ui.onAllNodesWithText(host.name).onFirst().performClick()
            }
            if (ui.onAllNodesWithText("Name").fetchSemanticsNodes().isEmpty()) ui.onNodeWithText("New session").performClick()
            ui.onNodeWithText("Name").performTextInput(name)
            ui.onNodeWithText("Create").performClick()
            await("created attachment") { repository.frame.value?.state == "active" && repository.state.value.sessions.any { it.name == name } }
            owned = repository.state.value.sessionId
            assertNotNull(owned)

            await("terminal window focus after dialog dismissal") { terminal().hasWindowFocus() }
            ui.waitForIdle()
            // A preedit replaces itself locally, then commits once through InputConnection.
            val prefix = "i=1; while [ \"\$i\" -le 400 ]; do printf 'UI_%03d 中文é\\n' \"\$i\"; i=\$((i+1)); done"
            ui.runOnIdle {
                val connection = terminal().onCreateInputConnection(EditorInfo())
                connection.setComposingText("unused preedit", 1)
                connection.setComposingText(prefix, 1)
                connection.finishComposingText()
                connection.performEditorAction(EditorInfo.IME_ACTION_NONE)
            }
            await("terminal output") { repository.frame.value?.let { it.historyMaximum > 100u && text(it).contains("UI_400") } == true }

            val originalInput = ui.runOnIdle { terminal().onCreateInputConnection(EditorInfo()) }
            val originalRows = repository.frame.value!!.viewport.rows
            val keyboardEpochs = linkedSetOf<ULong>()
            stopKeyboardFrames = ui.runOnIdle { repository.observeTerminalFrames { it?.let { keyboardEpochs.add(it.inputEpoch) } } }
            android.util.Log.i("ZtermAcceptance","IME initial rows=$originalRows viewHeight=${terminal().height}")
            val tapView = terminal()
            awaitDraw(tapView)
            val tapTime = SystemClock.uptimeMillis()
            touch(tapView,tapTime,MotionEvent.ACTION_DOWN,.5f,.5f)
            touch(tapView,tapTime,MotionEvent.ACTION_UP,.5f,.5f)
            SystemClock.sleep(400)
            assertFalse("ordinary content tap does not open IME", keyboardVisible())
            assertEquals("ordinary tap preserves viewport", originalRows, repository.frame.value!!.viewport.rows)
            ui.onNodeWithContentDescription("Show keyboard").performClick()
            await("system keyboard shown") { ui.activity.window.decorView.rootWindowInsets?.isVisible(android.view.WindowInsets.Type.ime()) == true && repository.frame.value?.state == "active" && (repository.frame.value?.viewport?.rows ?: originalRows) < originalRows }
            ui.onNodeWithText("Esc").assertIsDisplayed()
            instrumentation.sendKeyDownUpSync(android.view.KeyEvent.KEYCODE_BACK)
            await("Back hides keyboard before leaving terminal") { ui.activity.window.decorView.rootWindowInsets?.isVisible(android.view.WindowInsets.Type.ime()) == false && repository.frame.value?.state == "active" && repository.frame.value?.viewport?.rows == originalRows }
            assertEquals(owned,repository.state.value.sessionId)
            ui.runOnIdle { stopKeyboardFrames?.invoke(); stopKeyboardFrames = null }
            android.util.Log.i("ZtermAcceptance", "IME show/hide input epochs=${keyboardEpochs.size}")
            assertEquals("healthy keyboard transitions preserve input lifetime", 1, keyboardEpochs.size)
            ui.runOnIdle {
                assertTrue("same InputConnection survives keyboard resize",originalInput.commitText("printf 'IME_RESIZE_%s\\n' '中文'",1))
                assertTrue(originalInput.performEditorAction(EditorInfo.IME_ACTION_NONE))
            }
            await("composition committed once after keyboard resize") { repository.frame.value?.let { text(it).contains("IME_RESIZE_中文") } == true }
            awaitDraw(tapView)
            // Compose performClick waits for idleness and can return after the
            // platform animation. Trigger this reversal probe before that wait.
            instrumentation.runOnMainSync { tapView.setKeyboardVisible(true) }
            await("keyboard opening animation") { terminal().imeAnimating() && ui.activity.window.decorView.rootWindowInsets?.isVisible(android.view.WindowInsets.Type.ime()) == true }
            instrumentation.runOnMainSync { ui.activity.window.insetsController?.hide(android.view.WindowInsets.Type.ime()) }
            await("canceled keyboard opening restores final geometry") {
                !terminal().imeAnimating() && ui.activity.window.decorView.rootWindowInsets?.isVisible(android.view.WindowInsets.Type.ime()) == false && repository.frame.value?.state == "active" && repository.frame.value?.viewport?.rows == originalRows
            }

            val viewport = repository.frame.value!!.viewport
            ui.onNodeWithText(name).performClick()
            ui.onNodeWithText("New session").assertIsDisplayed()
            assertEquals("overlay preserves terminal dimensions", viewport, repository.frame.value!!.viewport)
            ui.onAllNodesWithText(name).onFirst().performClick()
            ui.waitUntil { !repository.state.value.panel }

            val view = terminal()
            repeat(3) { gesture(view, .5f, .2f, .5f, .85f, 600); SystemClock.sleep(250) }
            await("touch scroll") { (repository.frame.value?.historyOffset ?: 0u) > 5u }
            val olderOffset = repository.frame.value!!.historyOffset
            val cachedHits = repository.frame.value!!.stats.cacheHits
            gesture(view, .5f, .85f, .5f, .2f, 1200)
            await("upward drag returns through cached history") {
                repository.frame.value?.let { it.historyOffset < olderOffset && it.stats.cacheHits > cachedHits } == true
            }
            val bottomEpoch = repository.frame.value!!.inputEpoch
            val bottomStates = linkedSetOf<String>()
            val stopBottomFrames = ui.runOnIdle { repository.observeTerminalFrames { it?.let { bottomStates.add(it.state) } } }
            ui.runOnIdle { repository.scroll(0, false) }
            await("cached bottom stays local") { repository.frame.value?.historyOffset == 0uL }
            ui.runOnIdle { stopBottomFrames() }
            assertEquals(setOf("active"), bottomStates)
            assertEquals(bottomEpoch, repository.frame.value!!.inputEpoch)
            repeat(3) { gesture(view, .5f, .2f, .5f, .85f, 600); SystemClock.sleep(250) }
            await("scroll again for selection") { (repository.frame.value?.historyOffset ?: 0u) > 5u }
            ui.onNodeWithText("Esc").assertIsDisplayed()
            ui.onNodeWithText("→").assertIsDisplayed()
            SystemClock.sleep(350) // Finish the platform fling before placing the long press.
            longPress(view, .15f, .25f)
            await("native selection") { repository.frame.value?.selection != null }
            val selectionFrame = repository.frame.value!!
            val selection = selectionFrame.selection!!
            val handleX = (selection.focusColumn.toInt()+1f) / selectionFrame.viewport.columns.toInt() + .012f
            val handleY = (selection.focusRow-(selectionFrame.firstRow+selectionFrame.windowOffset.toLong()-selectionFrame.historyOffset.toLong())+1f) / selectionFrame.viewport.rows.toInt() + .01f
            dragToEdge(view,handleX,handleY)
            await("continuous cross-screen handle selection") {
                repository.frame.value?.selection?.let { kotlin.math.abs(it.focusRow-it.anchorRow) >= viewport.rows.toInt()*2 } == true
            }
            val evidence = repository.frame.value!!.stats
            assertTrue("cached gestures render locally", evidence.cacheHits > 0u)
            assertTrue("accounted semantic rows stay bounded", evidence.retainedRows <= 4096u)
            assertTrue("cache and captured pins stay within 16 MiB", evidence.peakBytes <= 16uL*1024u*1024u)
            android.util.Log.i("ZtermAcceptance", "navigation evidence: $evidence")
            val captured = repository.frame.value!!.selection
            for (language in listOf("system","zh","en")) for (theme in listOf("system","dark","light")) {
                val expected = Preferences(language,theme,12)
                ui.runOnIdle { repository.setPreferences(expected) }
                ui.waitUntil { repository.state.value.saved.preferences == expected }
                ui.waitForIdle()
                assertEquals("appearance changes preserve captured content", captured, repository.frame.value!!.selection)
                assertEquals("preferences persist independently", expected, AppStore(instrumentation.targetContext).load().preferences)
                assertEquals(owned,repository.state.value.sessionId)
            }
            ui.runOnIdle { repository.setPreferences(Preferences("en","dark",12)) }
            ui.waitUntil { repository.state.value.saved.preferences == Preferences("en","dark",12) }
            SystemClock.sleep(300)
            instrumentation.uiAutomation.takeScreenshot()?.let { bitmap ->
                java.io.File(instrumentation.targetContext.cacheDir, "selection-ui.png").outputStream().use { bitmap.compress(android.graphics.Bitmap.CompressFormat.PNG, 100, it) }
                bitmap.recycle()
            }
            assertTrue("floating system Copy action", clickSystemText("Copy"))
            await("copy clears selection") { repository.frame.value?.selection == null }
            ui.runOnIdle {
                val clip = ui.activity.getSystemService(ClipboardManager::class.java).primaryClip
                val copied = clip?.getItemAt(0)?.text?.toString().orEmpty()
                assertTrue("system clipboard spans three screens with exact Unicode", copied.count { it == '\n' } >= viewport.rows.toInt()*2 && copied.contains("中文é"))
            }
            longPress(view,.15f,.25f)
            await("selection for cancellation") { repository.frame.value?.selection != null }
            val lateCopy = java.util.concurrent.atomic.AtomicBoolean(false)
            ui.runOnIdle { repository.copySelection { lateCopy.set(true) }; repository.clearSelection() }
            await("copy canceled") { repository.frame.value?.selection == null }
            ui.waitForIdle()
            assertFalse("canceled FFI copy cannot cause a clipboard effect",lateCopy.get())

            val session = owned
            ui.activityRule.scenario.moveToState(Lifecycle.State.CREATED)
            SystemClock.sleep(700)
            assertEquals("background keeps exact Session", session, repository.state.value.sessionId)
            ui.activityRule.scenario.moveToState(Lifecycle.State.RESUMED)
            ui.activityRule.scenario.recreate()
            await("activity recreation") { repository.frame.value?.state == "active" }
            assertEquals(session, repository.state.value.sessionId)
            ui.onNodeWithText("Esc").assertIsDisplayed()
            ui.runOnIdle {
                terminal().onCreateInputConnection(EditorInfo()).commitText("printf 'UI_RETURN_%s\\n' ok\r", 1)
            }
            await("first input after history and recreation") { repository.frame.value?.let { it.historyOffset == 0uL && text(it).contains("UI_RETURN_ok") } == true }

            val portrait = repository.frame.value!!.viewport
            ui.runOnIdle { ui.activity.requestedOrientation = android.content.pm.ActivityInfo.SCREEN_ORIENTATION_LANDSCAPE }
            await("landscape keeps Session and resizes") { repository.frame.value?.let { it.state == "active" && it.viewport.columns > portrait.columns } == true }
            assertEquals(session,repository.state.value.sessionId)
            ui.onNodeWithText("Esc").assertIsDisplayed()
            ui.runOnIdle { ui.activity.requestedOrientation = android.content.pm.ActivityInfo.SCREEN_ORIENTATION_PORTRAIT }
            await("portrait restored") { repository.frame.value?.let { it.state == "active" && it.viewport == portrait } == true }
            for (font in listOf(8,9,13,16,12)) {
                ui.runOnIdle { repository.setPreferences(Preferences("en","dark",font)) }
                await("font applies measured geometry") { repository.frame.value?.let { it.state == "active" && repository.state.value.saved.preferences.fontSize == font && when {
                    font == 12 -> it.viewport == portrait
                    font < 12 -> it.viewport.rows > portrait.rows
                    else -> it.viewport.rows < portrait.rows
                } } == true }
                assertEquals(session,repository.state.value.sessionId)
            }

            ui.onNodeWithText(name).performClick()
            ui.onNode(hasContentDescription("Session actions") and hasAnyAncestor(hasText(name))).performClick()
            ui.onNodeWithText("Rename").performClick()
            ui.onNodeWithText("Name").performTextReplacement("$name-renamed")
            ui.onNodeWithText("Save").performClick()
            await("row rename") { !repository.state.value.busy && repository.state.value.sessions.any { it.sessionId == owned && it.name == "$name-renamed" } }
            ui.onNodeWithText("New session").performClick()
            ui.onNodeWithText("Name").performTextInput("$name-second")
            ui.onNodeWithText("Create").performClick()
            await("second explicit Session") { repository.frame.value?.state == "active" && repository.state.value.sessions.any { it.name == "$name-second" } }
            val second = repository.state.value.sessionId
            assertNotEquals(owned,second)
            ui.onNodeWithText("$name-second").performClick()
            ui.onNode(hasContentDescription("Session actions") and hasAnyAncestor(hasText("$name-second"))).performClick()
            ui.onNodeWithText("Delete").performClick()
            ui.onNodeWithText("Cancel").performClick()
            assertEquals("cancel preserves active Session",second,repository.state.value.sessionId)
            ui.onNode(hasContentDescription("Session actions") and hasAnyAncestor(hasText("$name-second"))).performClick()
            ui.onNodeWithText("Delete").performClick()
            ui.onNodeWithText("Delete",useUnmergedTree = true).performClick()
            await("confirmed remote deletion") { !repository.state.value.busy && repository.state.value.sessions.none { it.sessionId == second } }
            ui.onNodeWithText("$name-renamed").performClick()
            await("switch back to retained Session") { repository.state.value.sessionId == owned && repository.frame.value?.state == "active" }
        } finally {
            ui.runOnIdle { stopKeyboardFrames?.invoke() }
            ui.runOnIdle { repository.goHome(); repository.setPreferences(preferences) }
            ui.waitUntil(15_000) { repository.frame.value == null }
            runBlocking {
                val names = setOf(name,"$name-renamed","$name-second")
                repository.runtime.listSessions(host.id).filter { it.name in names }.forEach { repository.runtime.closeSession(host.id,it.sessionId) }
            }
        }
    }

    @Test fun childMouseAndAlternateScrollUseTouchWithoutStealingLongPress() {
        instrumentation.uiAutomation.serviceInfo = instrumentation.uiAutomation.serviceInfo.apply {
            flags = flags or android.accessibilityservice.AccessibilityServiceInfo.FLAG_RETRIEVE_INTERACTIVE_WINDOWS
        }
        ui.waitUntil(20_000) { repository.state.value.initialized }
        val preferences = repository.state.value.saved.preferences
        val host = repository.state.value.saved.hosts.first { it.name == (InstrumentationRegistry.getArguments().getString("hostName") ?: "my-mac") }
        val name = "android-ui-pointer-${System.nanoTime()}"
        try {
            ui.runOnIdle { repository.setPreferences(Preferences("en","dark",12)); repository.goHome() }
            ui.waitUntil { repository.frame.value == null && !repository.state.value.busy }
            ui.runOnIdle { repository.connectHost(host.id) }
            await("pointer host") { repository.state.value.route == Route.Terminal && !repository.state.value.busy }
            ui.runOnIdle { repository.createSession(name, "") }
            await("pointer Session") { repository.frame.value?.state == "active" && repository.state.value.sessions.any { it.name == name } && !repository.state.value.busy }
            await("pointer window") { terminal().hasWindowFocus() }
            await("fixture shell ready") { content().isNotBlank() }
            val fixture = android.util.Base64.encodeToString(pointerFixture.toByteArray(), android.util.Base64.NO_WRAP)
            ui.runOnIdle { repository.text("python3 -c 'import base64; exec(base64.b64decode(\"$fixture\"))'\r") }
            await("child mouse mode") { repository.frame.value?.pointerMode == io.github.leonfox28.zterm.nativebridge.NativePointerMode.MOUSE && content().contains("P=0 R=0") }
            ui.waitForIdle()
            val oldEpochBeforeTap = repository.frame.value!!.inputEpoch
            val oldRowsBeforeTap = repository.frame.value!!.viewport.rows
            val old = repository.frame.value!!.source!!.retained()
            try {
                val view = terminal()
                val paint = android.graphics.Paint().apply {
                    typeface = android.graphics.Typeface.MONOSPACE
                    textSize = android.util.TypedValue.applyDimension(android.util.TypedValue.COMPLEX_UNIT_SP,12f,view.resources.displayMetrics)
                }
                val expectedX = (view.width * .4f / paint.measureText("M")).toInt()+1
                val expectedY = (view.height * .3f / kotlin.math.ceil((paint.fontMetrics.descent-paint.fontMetrics.ascent).toDouble())).toInt()+1
                awaitDraw(view)
                val down = SystemClock.uptimeMillis()
                touch(view, down, MotionEvent.ACTION_DOWN, .4f,.3f)
                touch(view, down, MotionEvent.ACTION_UP, .4f,.3f)
                await("one complete child tap") { content().contains("P=1 R=1") }
                assertEquals(expectedX,counter("X")); assertEquals(expectedY,counter("Y"))
                SystemClock.sleep(400)
                assertFalse("child tap does not open IME", keyboardVisible())
                assertEquals("child tap does not resize", oldEpochBeforeTap, repository.frame.value!!.inputEpoch)
                ui.onNodeWithContentDescription("Show keyboard").performClick()
                await("explicit keyboard final size") { ui.activity.window.decorView.rootWindowInsets?.isVisible(android.view.WindowInsets.Type.ime()) == true && repository.frame.value?.state == "active" && repository.frame.value!!.viewport.rows < oldRowsBeforeTap }
                // A source captured before IME geometry changes cannot click a
                // different cell in the newly resized child grid.
                ui.runOnIdle { repository.pointer(old,0,0) }
                await("old pointer source rejected") { repository.state.value.error == "input_not_ready" }
                assertTrue(content().contains("P=1 R=1"))
                ui.runOnIdle { repository.clearError() }
            } finally { old.close() }
            ui.onNodeWithContentDescription("Hide keyboard").performClick()
            await("keyboard button hides IME") { !keyboardVisible() && geometrySettled() }
            val hiddenViewport = repository.frame.value!!.viewport
            val view = terminal()
            repeat(2) { index ->
                if (index != 0) SystemClock.sleep(100)
                awaitDraw(view)
                val tap = SystemClock.uptimeMillis()
                touch(view,tap,MotionEvent.ACTION_DOWN,.4f,.3f)
                touch(view,tap,MotionEvent.ACTION_UP,.4f,.3f)
            }
            ui.runOnIdle { repository.text("s") }
            await("rapid taps processed") { counter("Z") == 1 }
            assertEquals("each rapid tap sends one click",3,counter("P"))
            assertEquals(3,counter("R"))
            assertFalse("rapid taps keep IME hidden", keyboardVisible())
            assertEquals(hiddenViewport, repository.frame.value!!.viewport)
            val canceled = SystemClock.uptimeMillis()
            touch(view,canceled,MotionEvent.ACTION_DOWN,.4f,.3f)
            touch(view,canceled,MotionEvent.ACTION_CANCEL,.4f,.3f)
            gesture(view,.5f,.3f,.5f,.65f,500)
            await("finger down sends wheel up") { counter("U") > 0 }
            gesture(view,.5f,.65f,.5f,.3f,500)
            await("finger up sends wheel down") { counter("D") > 0 }
            assertEquals("cancel and drags never synthesize another tap",3,counter("P"))
            assertEquals(3,counter("R"))
            assertEquals(0uL,repository.frame.value!!.historyOffset)
            ui.runOnIdle { repository.text("s") }
            await("all earlier wheel input processed") { counter("Z") == 2 }
            val counts = content().lineSequence().first()
            longPress(view,.15f,.2f)
            await("child long press is local selection") { repository.frame.value?.selection != null }
            assertFalse("long press keeps IME hidden", keyboardVisible())
            gesture(view,.5f,.3f,.5f,.6f,500)
            assertEquals("selection never sends child clicks or wheels", counts, content().lineSequence().first())
            val selectionEpoch = repository.frame.value!!.inputEpoch
            val selectionRequests = repository.frame.value!!.stats.requests
            // No text/key barrier here: it would itself restore Live and mask
            // a broken selection exit. The dismissing tap belongs to selection.
            tap(view,.8f,.8f)
            await("cancel restores child mouse without keyboard input") { repository.frame.value?.selection == null && repository.frame.value?.pointerMode == io.github.leonfox28.zterm.nativebridge.NativePointerMode.MOUSE }
            assertEquals("cancel tap stays local",3,counter("P"))
            tap(view,.4f,.3f)
            await("tap after cancel reaches child") { counter("P") == 4 && counter("R") == 4 }
            val wheelBeforeCancel = counter("U")
            gesture(view,.5f,.3f,.5f,.65f,500)
            await("wheel after cancel reaches child") { counter("U") > wheelBeforeCancel }

            longPress(view,.01f,.005f)
            await("selection for native Copy") { repository.frame.value?.selection != null }
            assertTrue("system Copy in child TUI",clickSystemText("Copy"))
            await("Copy restores child mouse without keyboard input") { repository.frame.value?.selection == null && repository.frame.value?.pointerMode == io.github.leonfox28.zterm.nativebridge.NativePointerMode.MOUSE }
            tap(view,.4f,.3f)
            await("tap after Copy reaches child") { counter("P") == 5 && counter("R") == 5 }
            val wheelBeforeCopy = counter("D")
            gesture(view,.5f,.65f,.5f,.3f,500)
            await("wheel after Copy reaches child") { counter("D") > wheelBeforeCopy }
            assertEquals("selection exit needs no synchronization",selectionEpoch,repository.frame.value!!.inputEpoch)
            assertEquals("alternate selection exit needs no history request",selectionRequests,repository.frame.value!!.stats.requests)
            assertFalse("selection exit keeps IME hidden",keyboardVisible())

            ui.runOnIdle { repository.text("m") }
            await("alternate-scroll mode") { repository.frame.value?.pointerMode == io.github.leonfox28.zterm.nativebridge.NativePointerMode.ALTERNATE_SCROLL }
            longPress(view,.15f,.2f)
            await("alternate-scroll local selection") { repository.frame.value?.selection != null }
            tap(view,.8f,.8f)
            await("cancel restores alternate-scroll without keyboard input") { repository.frame.value?.selection == null && repository.frame.value?.pointerMode == io.github.leonfox28.zterm.nativebridge.NativePointerMode.ALTERNATE_SCROLL }
            awaitDraw(view)
            val alternateTap = SystemClock.uptimeMillis()
            touch(view,alternateTap,MotionEvent.ACTION_DOWN,.5f,.5f)
            touch(view,alternateTap,MotionEvent.ACTION_UP,.5f,.5f)
            SystemClock.sleep(400)
            assertFalse("alternate-screen tap keeps IME hidden", keyboardVisible())
            gesture(view,.5f,.3f,.5f,.65f,500)
            await("alternate scroll cursor up") { counter("A") > 0 }
            gesture(view,.5f,.65f,.5f,.3f,500)
            await("alternate scroll cursor down") { counter("B") > 0 }
            assertEquals(0uL, repository.frame.value!!.historyOffset)
            ui.runOnIdle { repository.text("q") }
            await("child exit restores local gestures") { repository.frame.value?.pointerMode == io.github.leonfox28.zterm.nativebridge.NativePointerMode.NONE && content().contains("POINTER_DONE") }
        } finally {
            ui.runOnIdle { ui.activity.window.insetsController?.hide(android.view.WindowInsets.Type.ime()); repository.goHome(); repository.setPreferences(preferences) }
            ui.waitUntil(15_000) { repository.frame.value == null }
            runBlocking { repository.runtime.listSessions(host.id).filter { it.name == name }.forEach { repository.runtime.closeSession(host.id,it.sessionId) } }
            ui.waitUntil(5_000) { ui.activity.window.decorView.rootWindowInsets?.isVisible(android.view.WindowInsets.Type.ime()) == false }
        }
    }
    @Test fun isolatedHerdrReceivesTabClicksAndPaneScroll() {
        instrumentation.uiAutomation.serviceInfo = instrumentation.uiAutomation.serviceInfo.apply {
            flags = flags or android.accessibilityservice.AccessibilityServiceInfo.FLAG_RETRIEVE_INTERACTIVE_WINDOWS
        }
        val directory = InstrumentationRegistry.getArguments().getString("herdrDirectory")
        org.junit.Assume.assumeTrue("explicit isolated host fixture", directory?.startsWith("/tmp/zterm-herdr-android-") == true)
        ui.waitUntil(20_000) { repository.state.value.initialized }
        val preferences = repository.state.value.saved.preferences
        val host = repository.state.value.saved.hosts.first { it.name == (InstrumentationRegistry.getArguments().getString("hostName") ?: "my-mac") }
        val name = "android-ui-pointer-herdr-${System.nanoTime()}"
        try {
            ui.runOnIdle { repository.setPreferences(Preferences("en","dark",12)); repository.goHome() }
            ui.waitUntil { repository.frame.value == null && !repository.state.value.busy }
            ui.runOnIdle { repository.connectHost(host.id) }
            await("Herdr host") { repository.state.value.route == Route.Terminal && !repository.state.value.busy }
            ui.runOnIdle { repository.createSession(name, directory!!) }
            await("Herdr Session") { repository.frame.value?.state == "active" && repository.state.value.sessions.any { it.name == name } && !repository.state.value.busy }
            await("Herdr shell") { content().isNotBlank() && terminal().hasWindowFocus() }
            ui.runOnIdle { repository.text("env -u HERDR_ENV -u HERDR_SOCKET_PATH -u HERDR_PANE_ID -u HERDR_TAB_ID -u HERDR_WORKSPACE_ID -u HERDR_BIN_PATH XDG_CONFIG_HOME='$directory/config' XDG_STATE_HOME='$directory/state' /opt/homebrew/bin/herdr --no-session\r") }
            await("Herdr mouse capture") { repository.frame.value?.pointerMode == io.github.leonfox28.zterm.nativebridge.NativePointerMode.MOUSE }
            // Its pane shell must finish starting before sending the fixture.
            await("Herdr pane prompt") { content().contains("%") }
            ui.runOnIdle { repository.text("i=1; while [ \"\$i\" -le 160 ]; do printf 'HERDR_%03d\\n' \"\$i\"; i=\$((i+1)); done\r") }
            await("Herdr pane output") { content().contains("HERDR_160") }
            gesture(terminal(),.5f,.3f,.5f,.7f,600)
            await("Herdr scrolls its pane history") { !content().contains("HERDR_160") && content().contains("HERDR_") }
            assertEquals("child wheel never moves outer history",0uL,repository.frame.value!!.historyOffset)
            repeat(2) { gesture(terminal(),.5f,.7f,.5f,.2f,600) }
            await("Herdr pane bottom") { content().contains("HERDR_160") }
            ui.runOnIdle { repository.text("/opt/homebrew/bin/herdr tab create --label Alpha --no-focus\r") }
            await("Herdr Alpha tab") { content().lineSequence().take(3).any { it.contains("1/2") || it.contains("Alpha") } }
            ui.runOnIdle { repository.text("/opt/homebrew/bin/herdr tab create --label Beta --focus\r") }
            await("Herdr Beta tab") { content().lineSequence().take(3).any { it.contains("Beta") } && !content().contains("HERDR_160") && content().contains("%") }
            ui.runOnIdle { repository.text("printf 'BETA_%s\\n' marker\r") }
            await("Beta pane marker") { content().contains("BETA_marker") }
            val beforeTabs = repository.frame.value!!.viewport
            val beforeSelectionEpoch = repository.frame.value!!.inputEpoch
            longPress(terminal(),.3f,.4f)
            await("Herdr local selection") { repository.frame.value?.selection != null }
            tap(terminal(),.8f,.8f)
            await("Herdr cancel restores touch") { repository.frame.value?.selection == null && repository.frame.value?.pointerMode == io.github.leonfox28.zterm.nativebridge.NativePointerMode.MOUSE }
            if (!content().lineSequence().take(3).any { it.contains("Alpha") }) {
                tapCellText("switch")
                await("Herdr tab chooser") { content().contains("Alpha") }
                assertFalse("Herdr tab chooser keeps IME hidden", keyboardVisible())
            }
            tapCellText("Alpha")
            await("actual tap focuses Alpha") { !content().contains("BETA_marker") && content().contains("%") && repository.frame.value?.state == "active" }
            assertFalse("Herdr tab click keeps IME hidden", keyboardVisible())
            assertEquals("Herdr tab click preserves viewport", beforeTabs, repository.frame.value!!.viewport)
            ui.runOnIdle { repository.text("i=1; while [ \"\$i\" -le 160 ]; do printf 'ALPHA_ROW_%03d\\n' \"\$i\"; i=\$((i+1)); done; printf 'ALPHA_%s\\n' marker\r") }
            await("Alpha pane marker") { content().contains("ALPHA_marker") }
            longPress(terminal(),.15f,.4f)
            await("Herdr selection for Copy") { repository.frame.value?.selection != null }
            assertTrue("Herdr system Copy",clickSystemText("Copy"))
            await("Herdr Copy restores touch") { repository.frame.value?.selection == null && repository.frame.value?.pointerMode == io.github.leonfox28.zterm.nativebridge.NativePointerMode.MOUSE }
            gesture(terminal(),.5f,.3f,.5f,.7f,600)
            await("Herdr scroll after Copy") { !content().contains("ALPHA_marker") && content().contains("ALPHA_ROW_") }
            repeat(2) { gesture(terminal(),.5f,.7f,.5f,.2f,600) }
            await("Herdr returns to pane bottom after Copy") { content().contains("ALPHA_marker") }
            assertEquals("Herdr selection exit uses the existing synchronized stream",beforeSelectionEpoch,repository.frame.value!!.inputEpoch)
            if (!content().lineSequence().take(3).any { it.contains("Beta") }) {
                tapCellText("switch")
                await("Herdr tab chooser again") { content().contains("Beta") }
            }
            tapCellText("Beta")
            await("actual tap restores Beta") { content().contains("BETA_marker") && !content().contains("ALPHA_marker") }
            ui.waitForIdle()
            SystemClock.sleep(150) // Let the new complete frame reach SurfaceFlinger before the evidence capture.
            instrumentation.uiAutomation.takeScreenshot()?.let { bitmap ->
                java.io.File(instrumentation.targetContext.cacheDir,"herdr-touch.png").outputStream().use { bitmap.compress(android.graphics.Bitmap.CompressFormat.PNG,100,it) }; bitmap.recycle()
            }
        } finally {
            ui.runOnIdle { ui.activity.window.insetsController?.hide(android.view.WindowInsets.Type.ime()); repository.goHome(); repository.setPreferences(preferences) }
            ui.waitUntil(15_000) { repository.frame.value == null }
            runBlocking { repository.runtime.listSessions(host.id).filter { it.name == name }.forEach { repository.runtime.closeSession(host.id,it.sessionId) } }
        }
    }
    private fun keyboardVisible(): Boolean = ui.activity.window.decorView.rootWindowInsets?.isVisible(android.view.WindowInsets.Type.ime()) == true
    private fun geometrySettled(): Boolean {
        val frame = repository.frame.value ?: return false
        val view = terminal()
        val paint = android.graphics.Paint().apply {
            typeface = android.graphics.Typeface.MONOSPACE
            textSize = android.util.TypedValue.applyDimension(android.util.TypedValue.COMPLEX_UNIT_SP,repository.state.value.saved.preferences.fontSize.toFloat(),view.resources.displayMetrics)
        }
        val rows = (view.height / kotlin.math.ceil((paint.fontMetrics.descent-paint.fontMetrics.ascent).toDouble())).toInt().coerceIn(1,80)
        val columns = (view.width / paint.measureText("M")).toInt().coerceIn(1,240)
        return !view.imeAnimating() && frame.state == "active" && frame.viewport.rows.toInt() == rows && frame.viewport.columns.toInt() == columns
    }
    private fun tapCellText(label: String) {
        awaitDraw(terminal())
        val lines = content().lines()
        val row = lines.indexOfFirst { it.contains(label) }
        check(row >= 0)
        val column = lines[row].indexOf(label)+label.length/2
        val view = terminal()
        val paint = android.graphics.Paint().apply {
            typeface = android.graphics.Typeface.MONOSPACE
            textSize = android.util.TypedValue.applyDimension(android.util.TypedValue.COMPLEX_UNIT_SP,12f,view.resources.displayMetrics)
        }
        val cellHeight = kotlin.math.ceil((paint.fontMetrics.descent-paint.fontMetrics.ascent).toDouble()).toFloat()
        val x = (column+.5f)*paint.measureText("M")/view.width
        val y = (row+.5f)*cellHeight/view.height
        val down = SystemClock.uptimeMillis()
        touch(view,down,MotionEvent.ACTION_DOWN,x,y); touch(view,down,MotionEvent.ACTION_UP,x,y)
    }

    private fun content() = repository.frame.value?.let { visibleRows(it) }?.joinToString("\n") { row -> row.cells.joinToString("") { cell -> if (cell.width == 0.toUByte()) "" else cell.text.ifEmpty { " " } } }.orEmpty()
    private fun counter(name: String) = Regex("$name=(\\d+)").find(content())?.groupValues?.get(1)?.toInt() ?: 0
    private val pointerFixture = """
import os, re, signal, termios, tty
fd=0
old=termios.tcgetattr(fd)
counts=dict(P=0,R=0,U=0,D=0,A=0,B=0,X=0,Y=0,Z=0)
buffer=b''
def write(s): os.write(1,s.encode())
def render(*_):
    write('\x1b[H\x1b[2J'+' '.join(k+'='+str(v) for k,v in counts.items())+'\r\n\r\nTOUCH TEST\r\nSelect this text locally')
try:
    tty.setraw(fd)
    write('\x1b[?1049h\x1b[?1000h\x1b[?1006h\x1b[?25l')
    signal.signal(signal.SIGWINCH,render)
    render()
    done=False
    while not done:
        buffer+=os.read(fd,1024)
        while buffer:
            match=re.match(rb'\x1b\[<(\d+);(\d+);(\d+)([Mm])',buffer)
            if match:
                code=int(match[1]); release=match[4]==b'm'
                key='U' if code==64 else 'D' if code==65 else 'R' if release else 'P'
                counts['X']=int(match[2]); counts['Y']=int(match[3])
                counts[key]+=1; buffer=buffer[match.end():]
            elif buffer[:3] in (b'\x1b[A',b'\x1b[B',b'\x1bOA',b'\x1bOB'):
                counts['A' if buffer[2:3]==b'A' else 'B']+=1; buffer=buffer[3:]
            elif buffer[:1]==b'\x1b': break
            else:
                char=buffer[:1]; buffer=buffer[1:]
                if char==b'm': write('\x1b[?1000l\x1b[?1006l\x1b[?1007h')
                if char==b's': counts['Z']+=1
                if char in (b'q',b'\x03'): done=True
        render()
finally:
    write('\x1b[?1000l\x1b[?1006l\x1b[?1007l\x1b[?1049l\x1b[?25h')
    termios.tcsetattr(fd,termios.TCSANOW,old)
    print('POINTER_DONE')
""".trimIndent()

    private fun await(stage: String, condition: () -> Boolean) {
        android.util.Log.i("ZtermAcceptance", "UI waiting: $stage")
        try { ui.waitUntil(20_000, condition) }
        catch (error: Throwable) {
            val state = repository.state.value
            val frame = repository.frame.value
            if (state.sessions.any { it.sessionId == state.sessionId && it.name.startsWith("android-ui-pointer-") }) {
                java.io.File(instrumentation.targetContext.cacheDir,"pointer-fixture-failure.txt").writeText("mode=${frame?.pointerMode}\n"+content())
            }
            android.util.Log.e("ZtermAcceptance", "UI timeout: $stage route=${state.route} busy=${state.busy} error=${state.error} sessions=${state.sessions.size} frame=${frame?.state} frameError=${frame?.error} inputEpoch=${frame?.inputEpoch} viewport=${frame?.viewport} viewHeight=${terminal().height} ime=${ui.activity.window.decorView.rootWindowInsets?.getInsets(android.view.WindowInsets.Type.ime())}")
            throw error
        }
    }
    private fun terminal(): TerminalView {
        fun find(view: View): TerminalView? {
            if (view is TerminalView) return view
            if (view is ViewGroup) for (index in 0 until view.childCount) find(view.getChildAt(index))?.let { return it }
            return null
        }
        return requireNotNull(find(ui.activity.window.decorView))
    }
    private fun awaitDraw(view: View) {
        val painted = java.util.concurrent.CountDownLatch(1)
        val listener = android.view.ViewTreeObserver.OnDrawListener { painted.countDown() }
        instrumentation.runOnMainSync { view.viewTreeObserver.addOnDrawListener(listener); view.invalidate() }
        try { assertTrue("native frame reached a draw", painted.await(5, java.util.concurrent.TimeUnit.SECONDS)) }
        finally { instrumentation.runOnMainSync { view.viewTreeObserver.removeOnDrawListener(listener) } }
    }
    private fun touch(view: View, down: Long, action: Int, x: Float, y: Float) {
        instrumentation.runOnMainSync {
            MotionEvent.obtain(down, SystemClock.uptimeMillis(), action, x * view.width, y * view.height, 0).also {
                view.dispatchTouchEvent(it); it.recycle()
            }
        }
    }
    private fun gesture(view: View, x: Float, y: Float, endX: Float, endY: Float, duration: Long) {
        awaitDraw(view)
        val down = SystemClock.uptimeMillis()
        touch(view, down, MotionEvent.ACTION_DOWN, x, y)
        for (step in 1..24) {
            SystemClock.sleep(duration / 24)
            touch(view, down, MotionEvent.ACTION_MOVE, x + (endX-x)*step/24, y + (endY-y)*step/24)
        }
        touch(view, down, MotionEvent.ACTION_UP, endX, endY)
    }
    private fun dragToEdge(view: View, x: Float, y: Float) {
        awaitDraw(view)
        val down = SystemClock.uptimeMillis()
        touch(view,down,MotionEvent.ACTION_DOWN,x,y)
        for (step in 1..30) {
            SystemClock.sleep(20)
            touch(view,down,MotionEvent.ACTION_MOVE,x+(.8f-x)*step/30,y+(.99f-y)*step/30)
        }
        try {
            ui.waitUntil(15_000) {
                repository.frame.value?.let { frame -> frame.selection?.let { kotlin.math.abs(it.focusRow-it.anchorRow) >= frame.viewport.rows.toInt()*2 } } == true
            }
        } finally { touch(view,down,MotionEvent.ACTION_UP,.8f,.99f) }
    }
    private fun longPress(view: View, x: Float, y: Float) {
        awaitDraw(view)
        val down = SystemClock.uptimeMillis()
        touch(view, down, MotionEvent.ACTION_DOWN, x, y)
        SystemClock.sleep(android.view.ViewConfiguration.getLongPressTimeout().toLong() + 150)
        touch(view, down, MotionEvent.ACTION_UP, x, y)
    }
    private fun tap(view: View, x: Float, y: Float) {
        awaitDraw(view)
        val down = SystemClock.uptimeMillis()
        touch(view,down,MotionEvent.ACTION_DOWN,x,y)
        touch(view,down,MotionEvent.ACTION_UP,x,y)
    }
    private fun clickSystemText(label: String): Boolean {
        repeat(30) {
            val roots = instrumentation.uiAutomation.windows.mapNotNull { it.root }
            for (root in roots) {
                val node = root.findAccessibilityNodeInfosByText(label).firstOrNull { it.text?.toString()?.equals(label, true) == true }
                if (node != null) {
                    var target: AccessibilityNodeInfo? = node
                    while (target != null && !target.isClickable) target = target.parent
                    if (target?.performAction(AccessibilityNodeInfo.ACTION_CLICK) == true) return true
                }
            }
            SystemClock.sleep(100)
        }
        return false
    }
    private fun text(frame: io.github.leonfox28.zterm.nativebridge.NativeFrame) = visibleRows(frame).joinToString("\n") { it.cells.joinToString("") { cell -> cell.text } }
}

private fun visibleRows(frame: io.github.leonfox28.zterm.nativebridge.NativeFrame): List<io.github.leonfox28.zterm.nativebridge.NativeRow> {
    val start = (frame.windowOffset - frame.historyOffset).toInt()
    return frame.rows.drop(start).take(frame.viewport.rows.toInt())
}
