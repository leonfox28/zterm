package io.github.leonfox28.zterm

import androidx.test.platform.app.InstrumentationRegistry
import androidx.compose.ui.test.*
import androidx.compose.ui.test.junit4.createEmptyComposeRule
import androidx.test.core.app.ActivityScenario
import androidx.test.core.app.ApplicationProvider
import io.github.leonfox28.zterm.nativebridge.*
import kotlinx.coroutines.delay
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.withTimeout
import org.json.JSONObject
import org.junit.Assert.*
import org.junit.Assume.assumeTrue
import org.junit.Test
import org.junit.Rule
import java.io.File

class DesktopHandoffTest {
    @get:Rule val ui = createEmptyComposeRule()
    @Test fun localDesktopFirstThenPhoneTakeover() = runBlocking {
        assumeTrue("explicit desktop-first orchestration",InstrumentationRegistry.getArguments().getString("desktopFirst") == "1")
        val context = InstrumentationRegistry.getInstrumentation().targetContext
        val saved = AppStore(context).load()
        val hostName = InstrumentationRegistry.getArguments().getString("hostName") ?: "my-mac"
        val host = saved.hosts.first { it.name == hostName }
        val seed = AppStore(context).loadOrCreateSeed()
        val metadata = File(context.cacheDir,"handoff.json")
        val resume = File(context.cacheDir,"handoff-return")
        metadata.delete(); resume.delete()
        try {
            val application = ApplicationProvider.getApplicationContext<ZtermApplication>()
            application.runtime.let { runtime ->
                runtime.initialize(seed,PlatformNetwork(context) {}.current(),saved.hosts.map { it.native() })
                runtime.listSessions(host.id)
                val session = runtime.createSession(host.id,"android-desktop-first-${System.nanoTime()}",null,NativeViewport(39u,140u),false)
                try {
                    metadata.writeText(JSONObject().put("session",session.sessionId).toString())
                    withTimeout(40_000) { while (!resume.exists()) delay(50) }
                    val before = runtime.listSessions(host.id).first { it.sessionId == session.sessionId }
                    assertTrue(before.occupied)
                    assertEquals(140.toUShort(),before.columns)
                    if (InstrumentationRegistry.getArguments().getString("actualUi") == "1") {
                        AppStore(context).save(saved.copy(hosts = saved.hosts.map { if (it.id == host.id) it.copy(lastSession = session.sessionId) else it },preferences = Preferences("en","dark",12)))
                        ActivityScenario.launch(MainActivity::class.java).use {
                            val repository = application.repository
                            try {
                                ui.waitUntil(20_000) { repository.state.value.initialized }
                                ui.onAllNodesWithText(host.name).onLast().performClick()
                                ui.waitUntil(20_000) { !repository.state.value.busy && repository.state.value.error == "session_occupied" }
                                ui.onNodeWithText("Take over").performClick()
                                ui.onAllNodes(hasText("Take over") and hasClickAction()).onLast().performClick()
                                ui.waitUntil(20_000) {
                                    val frame = repository.frame.value
                                    assertNull("takeover state=${frame?.state}",frame?.error)
                                    frame?.state == "active" && frame.viewport.columns < before.columns && !repository.state.value.busy
                                }
                                if (InstrumentationRegistry.getArguments().getString("wideClip") == "1") {
                                    assertClippedFrame(repository.frame.value!!)
                                    delay(1_000)
                                    assertNull(repository.frame.value?.error)
                                    assertEquals("active", repository.frame.value?.state)
                                }
                                val actual = runtime.listSessions(host.id).first { it.sessionId == session.sessionId }
                                assertEquals(repository.frame.value!!.viewport.columns,actual.columns)
                                assertEquals(repository.frame.value!!.viewport.rows,actual.rows)
                                assertEquals(session.sessionId,repository.state.value.sessionId)
                                assertNull(repository.state.value.error)
                            } finally {
                                ui.runOnIdle { repository.goHome() }
                                ui.waitUntil(15_000) { repository.frame.value == null && !repository.state.value.busy }
                            }
                        }
                        return@let
                    }
                    val phone = NativeViewport(49u,54u)
                    runtime.connectTerminal(host.id,session.sessionId,phone,true,true).use { terminal ->
                        wait(terminal) {
                            assertNull("takeover state=${it.state}",it.error)
                            assertNotEquals("takeover must retain its connection","closed",it.state)
                            it.state == "active" && it.viewport == phone
                        }
                        val frame = terminal.currentFrame()
                        if (InstrumentationRegistry.getArguments().getString("wideClip") == "1") {
                            try { assertClippedFrame(frame) } catch (error: Throwable) { frame.source?.close(); throw error }
                        }
                        val epoch = frame.inputEpoch
                        frame.source?.close()
                        terminal.commitText(epoch,"printf 'PHONE_TAKEOVER_%s\\n' ok\r",0u,false)
                        wait(terminal) {
                            assertNull("post-takeover state=${it.state}",it.error)
                            it.state == "active" && text(it).contains("PHONE_TAKEOVER_ok")
                        }
                        assertEquals(phone.columns,runtime.listSessions(host.id).first { it.sessionId == session.sessionId }.columns)
                        terminal.detach()
                    }
                } finally { runtime.closeSession(host.id,session.sessionId) }
            }
        } finally { seed.fill(0); metadata.delete(); resume.delete(); AppStore(context).save(saved) }
    }

    @Test fun desktopAndAndroidTakeOverTheSameSession() = runBlocking {
        assumeTrue("explicit desktop handoff orchestration",InstrumentationRegistry.getArguments().getString("handoff") == "1")
        val context = InstrumentationRegistry.getInstrumentation().targetContext
        val store = AppStore(context)
        val saved = store.load()
        val hostName = InstrumentationRegistry.getArguments().getString("hostName") ?: "my-mac"
        val host = saved.hosts.first { it.name == hostName }
        val seed = store.loadOrCreateSeed()
        val metadata = File(context.cacheDir,"handoff.json")
        val resume = File(context.cacheDir,"handoff-return")
        val lost = File(context.cacheDir,"handoff-lost")
        metadata.delete(); resume.delete(); lost.delete()
        try {
            NativeRuntime().use { runtime ->
                runtime.initialize(seed,PlatformNetwork(context) {}.current(),saved.hosts.map { it.native() })
                android.util.Log.i("ZtermAcceptance","Handoff: create")
                runtime.listSessions(host.id) // Match the saved-host navigation flow.
                val session = runtime.createSession(host.id,"android-handoff-${System.nanoTime()}",null,NativeViewport(24u,80u),true)
                try {
                    android.util.Log.i("ZtermAcceptance","Handoff: attach")
                    val phone = NativeViewport(32u,52u)
                    runtime.connectTerminal(host.id,session.sessionId,phone,true,false).use { first ->
                        wait(first) { it.state == "active" && it.viewport == phone }
                        assertEquals(phone.columns,runtime.listSessions(host.id).first { it.sessionId == session.sessionId }.columns)
                        android.util.Log.i("ZtermAcceptance","Handoff: publish")
                        metadata.writeText(JSONObject().put("session",session.sessionId).toString())
                        android.util.Log.i("ZtermAcceptance","Handoff: lease lost")
                        wait(first) { it.state == "lease_lost" }
                        lost.writeText("lease_lost")
                        assertFalse(first.currentFrame().let { it.source?.close(); it.inputReady })
                        withTimeout(40_000) { while (!resume.exists()) delay(50) }
                        assertEquals("desktop reserves one column for its scrollbar",79.toUShort(),runtime.listSessions(host.id).first { it.sessionId == session.sessionId }.columns)
                        android.util.Log.i("ZtermAcceptance","Handoff: take back")
                        runtime.connectTerminal(host.id,session.sessionId,phone,true,true).use { second ->
                            wait(second) { it.state == "active" && it.viewport == phone && text(it).contains("DESKTOP_HANDOFF_ok") }
                            assertEquals(phone.columns,runtime.listSessions(host.id).first { it.sessionId == session.sessionId }.columns)
                            second.detach()
                        }
                    }
                    assertTrue(runtime.listSessions(host.id).any { it.sessionId == session.sessionId })
                } finally { runtime.closeSession(host.id,session.sessionId) }
            }
        } finally { seed.fill(0); metadata.delete(); resume.delete(); lost.delete() }
    }
    private suspend fun wait(terminal: NativeTerminal,predicate: (NativeFrame)->Boolean) = withTimeout(40_000) {
        var frame = terminal.currentFrame()
        try {
            while (!predicate(frame)) {
                val generation = frame.generation
                frame.source?.close()
                frame = terminal.waitForFrame(generation)
            }
        } finally { frame.source?.close() }
    }
    private fun assertClippedFrame(frame: NativeFrame) {
        assertTrue("fixture must survive takeover", text(frame).contains("WIDE_FIXTURE_READY"))
        val row = when (frame.viewport.columns.toInt()) {
            54 -> 1
            56 -> 2
            else -> error("fixture expects 54 or 56 columns, got ${frame.viewport.columns}")
        }
        val clipped = frame.rows[row].cells.last()
        assertEquals(1.toUByte(), clipped.width)
        assertEquals(" ", clipped.text)
    }
    private fun text(frame: NativeFrame) = frame.rows.joinToString("\n") { row -> row.cells.joinToString("") { it.text } }
}
