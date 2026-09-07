package io.github.leonfox28.zterm

import androidx.compose.ui.test.*
import androidx.compose.ui.test.junit4.createEmptyComposeRule
import androidx.test.core.app.ActivityScenario
import androidx.test.core.app.ApplicationProvider
import androidx.test.platform.app.InstrumentationRegistry
import io.github.leonfox28.zterm.nativebridge.*
import kotlinx.coroutines.runBlocking
import org.junit.Assert.*
import org.junit.Assume.assumeTrue
import org.junit.Rule
import org.junit.Test

/** Run alone: seeds an exact disposable resume target before the lazy Repository starts. */
class OccupiedRecoveryTest {
    @get:Rule val ui = createEmptyComposeRule()

    @Test fun occupiedResumeAndLostLeaseRequireExplicitConfirmation() = runBlocking {
        assumeTrue("explicit isolated recovery fixture", InstrumentationRegistry.getArguments().getString("occupiedRecovery") == "1")
        val application = ApplicationProvider.getApplicationContext<ZtermApplication>()
        val store = AppStore(application)
        val saved = store.load()
        val host = saved.hosts.first { it.name == "my-mac" }
        val runtime = application.runtime
        val seed = store.loadOrCreateSeed()
        try { runtime.initialize(seed,PlatformNetwork(application) {}.current(),saved.hosts.map { it.native() }) }
        finally { seed.fill(0) }
        runtime.listSessions(host.id)
        val owned = runtime.createSession(host.id,"android-ui-occupied-${System.nanoTime()}",null,NativeViewport(39u,140u),true)
        try {
            runtime.connectTerminal(host.id,owned.sessionId,NativeViewport(39u,140u),true,false).use { other ->
                ui.waitUntil(20_000) { state(other) == "active" }
                InstrumentationRegistry.getArguments().getString("herdrDirectory")?.let { directory ->
                    require(Regex("/tmp/zterm-herdr-android-[a-z0-9]+").matches(directory))
                    val epoch = inspect(other) { it.inputEpoch }
                    other.commitText(epoch,"XDG_CONFIG_HOME='$directory/config' XDG_STATE_HOME='$directory/state' /opt/homebrew/bin/herdr --no-session\r",0u,false)
                    ui.waitUntil(20_000) {
                        inspect(other) { frame -> frame.pointerMode == NativePointerMode.MOUSE && frame.rows.any { row -> row.cells.any { cell -> cell.text == "%" } } }
                    }
                    other.commitText(inspect(other) { it.inputEpoch },"i=1; while [ \"\$i\" -le 160 ]; do printf 'HANDOFF_%03d 中文 é 色彩 terminal resize\\n' \"\$i\"; i=\$((i+1)); done\r",0u,false)
                    ui.waitUntil(20_000) {
                        inspect(other) { frame -> frame.rows.any { row -> row.cells.joinToString("") { it.text }.contains("HANDOFF_160") } }
                    }
                }
                store.save(saved.copy(
                    hosts = saved.hosts.map { if (it.id == host.id) it.copy(lastSession = owned.sessionId) else it },
                    recent = RecentConnection(host.id,owned.sessionId), preferences = Preferences("en","dark",12),
                ))
                ActivityScenario.launch(MainActivity::class.java).use {
                    val repository = application.repository
                    try {
                        ui.waitUntil(20_000) { repository.state.value.initialized }
                        ui.onAllNodesWithText(host.name).onLast().performClick()
                        ui.waitUntil(20_000) { !repository.state.value.busy && repository.state.value.error == "session_occupied" }
                        assertEquals(owned.sessionId,repository.state.value.sessionId)
                        assertNull(repository.frame.value)

                        // Recovery must be reachable even before the Session list loaded.
                        ui.onNodeWithText("Take over").performClick()
                        ui.onNodeWithText("Cancel").performClick()
                        assertEquals("cancel preserves the other controller", "active",state(other))
                        assertNull(repository.frame.value)

                        ui.onNodeWithText("Sessions").performClick()
                        ui.waitUntil(20_000) { repository.state.value.sessions.any { row -> row.sessionId == owned.sessionId } }
                        ui.onNodeWithText("Current").assertDoesNotExist()
                        ui.onAllNodesWithText(owned.name).onLast().performClick()
                        ui.onNodeWithText("Cancel").assertExists()
                        ui.onAllNodes(hasText("Take over") and hasClickAction()).onLast().performClick()
                        ui.waitUntil(20_000) { repository.frame.value?.state == "active" && !repository.state.value.busy }
                        ui.waitUntil(20_000) { state(other) == "lease_lost" }
                        assertEquals(owned.sessionId,repository.state.value.sessionId)

                        // A subsequent lost lease also must not be labelled Current.
                        runtime.connectTerminal(host.id,owned.sessionId,NativeViewport(24u,80u),true,true).use { next ->
                            ui.waitUntil(20_000) { state(next) == "active" && repository.frame.value?.state == "lease_lost" }
                            ui.onNodeWithText("Take over").performClick()
                            ui.onNodeWithText("Cancel").performClick()
                            assertEquals("active",state(next))
                            ui.onNodeWithText("Sessions").performClick()
                            ui.onNodeWithText("Current").assertDoesNotExist()
                            ui.onAllNodesWithText(owned.name).onLast().performClick()
                            ui.onAllNodes(hasText("Take over") and hasClickAction()).onLast().performClick()
                            ui.waitUntil(20_000) { repository.frame.value?.state == "active" && state(next) == "lease_lost" }
                            assertEquals(owned.sessionId,repository.state.value.sessionId)
                        }
                    } finally {
                        ui.runOnIdle { repository.goHome() }
                        ui.waitUntil(15_000) { repository.frame.value == null && !repository.state.value.busy }
                    }
                }
            }
        } finally {
            try { runtime.closeSession(host.id,owned.sessionId) }
            finally { store.save(saved) }
        }
    }

    private fun state(terminal: NativeTerminal): String = inspect(terminal) { it.state }
    private fun <T> inspect(terminal: NativeTerminal, read: (NativeFrame) -> T): T = terminal.currentFrame().let {
        try { read(it) } finally { it.source?.close() }
    }
}
