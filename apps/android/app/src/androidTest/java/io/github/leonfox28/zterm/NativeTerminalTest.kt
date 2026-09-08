package io.github.leonfox28.zterm

import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import io.github.leonfox28.zterm.nativebridge.*
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.withTimeout
import org.junit.Assert.*
import org.junit.Assume.assumeTrue
import org.junit.Test
import org.junit.runner.RunWith

@RunWith(AndroidJUnit4::class)
class NativeTerminalTest {
    @Test fun realHostSessionInputResizeRenameAndDetach() = runBlocking {
        val context = InstrumentationRegistry.getInstrumentation().targetContext
        val store = AppStore(context)
        val saved = store.load()
        assumeTrue("explicitly paired host retained for acceptance", saved.hosts.isNotEmpty())
        val hostIndex = InstrumentationRegistry.getArguments().getString("hostIndex")?.toIntOrNull() ?: 0
        val host = saved.hosts[hostIndex]
        val seed = store.loadOrCreateSeed()
        try {
            NativeRuntime().use { runtime ->
                try {
                    runtime.initialize(seed, PlatformNetwork(context) {}.current(), saved.hosts.map { it.native() })
                    android.util.Log.i("ZtermAcceptance", "list before create")
                    runtime.listSessions(host.id)
                    val name = "android-check-${System.nanoTime()}"
                    android.util.Log.i("ZtermAcceptance", "before create: ${runtime.connectionInfo(host.id)}")
                    val supplied = InstrumentationRegistry.getArguments().getString("sessionId")
                    val session = try {
                        if (supplied == null) withTimeout(15_000) { runtime.createSession(host.id,name,null,NativeViewport(24u,80u),true) }
                        else runtime.listSessions(host.id).first { it.sessionId == supplied && it.name.startsWith("android-check-") && !it.occupied }
                    } catch (error: Exception) {
                        android.util.Log.e("ZtermAcceptance", "create failed: ${error.javaClass.simpleName} ${runtime.connectionInfo(host.id)}")
                        throw error
                    }
                    android.util.Log.i("ZtermAcceptance", "create complete; attach begin: ${runtime.connectionInfo(host.id)}")
                    try {
                        withTimeout(15_000) { runtime.connectTerminal(host.id,session.sessionId,NativeViewport(24u,80u),true,false) }.use { terminal ->
                            awaitFrame(terminal, "initial active") { it.state == "active" }
                            InstrumentationRegistry.getArguments().getString("expectedPath")?.let { expected ->
                                assertEquals("actual transport path", expected, runtime.connectionInfo(host.id)?.path)
                            }
                            terminal.commitText(terminal.currentFrame().let { frame -> frame.source?.close(); frame.inputEpoch }, "printf 'ANDROID_NATIVE_%s\\n' '中文é'\r",0u,false)
                            val output = awaitFrame(terminal, "unicode output") { frame -> text(frame).contains("ANDROID_NATIVE_中文é") }
                            assertEquals(80,output.viewport.columns.toInt())
                            terminal.commitText(terminal.currentFrame().let { frame -> frame.source?.close(); frame.inputEpoch }, "i=1; while [ \"\$i\" -le 160 ]; do printf 'ROW_%03d 中文é\\n' \"\$i\"; i=\$((i+1)); done\r",0u,false)
                            awaitFrame(terminal, "retained output") { it.historyMaximum >= 120uL && text(it).contains("ROW_160") }
                            terminal.scrollTo(80u,true,4u)
                            awaitFrame(terminal, "historical page") { it.historyOffset == 80uL }
                            var selectedFrame = terminal.currentFrame()
                            terminal.beginSelection(requireNotNull(selectedFrame.source),0u,0u)
                            selectedFrame.source?.close()
                            for (offset in listOf(56uL,32uL)) {
                                terminal.scrollTo(offset,false,4u)
                                awaitFrame(terminal,"cross-screen selection") { it.historyOffset == offset }
                                selectedFrame = terminal.currentFrame()
                                terminal.extendSelection(requireNotNull(selectedFrame.source),23u,79u,false)
                                selectedFrame.source?.close()
                            }
                            val copied = terminal.copySelection()
                            assertTrue("three screens of exact Unicode text", copied.count { it == '\n' } >= 70 && copied.contains("中文é"))
                            terminal.commitText(terminal.currentFrame().let { frame -> frame.source?.close(); frame.inputEpoch }, "printf 'AFTER_RETURN_%s\\n' 'ok'\r",0u,false)
                            awaitFrame(terminal,"first input after history") { it.historyOffset == 0uL && text(it).contains("AFTER_RETURN_ok") }
                            val beforeResize = terminal.currentFrame()
                            val oldEpoch = beforeResize.inputEpoch
                            terminal.resize(NativeViewport(20u,70u))
                            terminal.commitText(oldEpoch,"printf 'RESIZE_INPUT_%s\\n' '中文'\r",0u,false)
                            awaitFrame(terminal, "resize") { it.state == "active" && it.viewport.rows == 20.toUShort() && it.viewport.columns == 70.toUShort() }
                            awaitFrame(terminal,"healthy resize preserves ordered input") { text(it).contains("RESIZE_INPUT_中文") }
                            terminal.resize(beforeResize.viewport)
                            val restored = awaitFrame(terminal,"A-B-A resize") { it.state == "active" && it.viewport == beforeResize.viewport }
                            assertEquals("keyboard lifetime survives resize",oldEpoch,restored.inputEpoch)
                            assertTrue(restored.geometryGeneration > beforeResize.geometryGeneration)
                            val retired = runCatching { terminal.beginSelection(requireNotNull(beforeResize.source),0u,0u) }.exceptionOrNull()
                            beforeResize.source?.close()
                            assertTrue("A-B-A does not revive old coordinates",retired is NativeException.RequestFailed && retired.code == "selection_changed")
                            for (alternate in listOf(false,true)) {
                                val enter = if (alternate) "\\033[?1049h" else ""
                                val leave = if (alternate) "\\033[?1049l" else ""
                                val active = terminal.currentFrame().let { it.source?.close(); it.inputEpoch }
                                terminal.commitText(active,"printf '$enter\\033[?25l\\033[4;7H'; read -r answer; printf '$leave\\033[?25h\\n'\r",0u,false)
                                val hidden = awaitFrame(terminal,"hidden cursor anchor on screen") { !it.cursorVisible && it.cursorRow == 3.toUShort() && it.cursorColumn == 6.toUShort() }
                                terminal.commitText(hidden.inputEpoch,"\r",0u,false)
                                awaitFrame(terminal,"cursor restored after TUI") { it.cursorVisible }
                            }
                            val renamed = runtime.renameSession(host.id,session.sessionId,"$name-renamed")
                            assertEquals(session.sessionId,renamed.sessionId)
                            assertEquals("$name-renamed",runtime.listSessions(host.id).first { it.sessionId == session.sessionId }.name)
                            terminal.detach()
                        }
                        assertTrue(runtime.listSessions(host.id).any { it.sessionId == session.sessionId })
                    } catch (error: Exception) {
                        android.util.Log.e("ZtermAcceptance", "terminal stage failed: ${error.javaClass.simpleName} ${runtime.connectionInfo(host.id)}")
                        throw error
                    } finally {
                        try { withTimeout(15_000) { runtime.closeSession(host.id,session.sessionId) } }
                        catch (error: Exception) { android.util.Log.e("ZtermAcceptance", "cleanup failed: ${error.javaClass.simpleName}") }
                    }
                    assertFalse(runtime.listSessions(host.id).any { it.sessionId == session.sessionId })
                } finally { withTimeout(5_000) { runtime.shutdown() } }
            }
        } finally { seed.fill(0) }
    }
    private suspend fun awaitFrame(terminal: NativeTerminal, stage: String, predicate: (NativeFrame) -> Boolean): NativeFrame = withTimeout(15_000) {
        android.util.Log.i("ZtermAcceptance", "waiting: $stage")
        var frame = terminal.currentFrame()
        while (!predicate(frame)) {
            assertFalse("attachment failed: ${frame.error}", frame.state in setOf("closed","ended","lease_lost"))
            android.util.Log.i("ZtermAcceptance", "$stage frame=${frame.generation} state=${frame.state} rows=${frame.viewport.rows} columns=${frame.viewport.columns} retained=${frame.historyMaximum} offset=${frame.historyOffset} finalRow=${text(frame).contains("ROW_160")} ascii=${text(frame).contains("ANDROID_NATIVE_")} cjk=${text(frame).contains("中文")}")
            val generation = frame.generation
            frame.source?.close()
            frame = terminal.waitForFrame(generation)
        }
        frame.source?.close()
        frame
    }
    private fun text(frame: NativeFrame): String = frame.rows.joinToString("\n") { row -> row.cells.joinToString("") { it.text } }
}
