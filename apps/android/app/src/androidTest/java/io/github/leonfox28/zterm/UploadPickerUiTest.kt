package io.github.leonfox28.zterm

import android.view.KeyEvent
import androidx.compose.ui.test.*
import androidx.compose.ui.test.junit4.createEmptyComposeRule
import androidx.test.core.app.ActivityScenario
import androidx.test.core.app.ApplicationProvider
import androidx.test.platform.app.InstrumentationRegistry
import org.junit.Assert.*
import org.junit.Assume.assumeTrue
import org.junit.Rule
import org.junit.Test

/** Explicit test-owned host; uses both actual system pickers and Android Back. */
class UploadPickerUiTest {
    @get:Rule val ui = createEmptyComposeRule()
    private val instrumentation = InstrumentationRegistry.getInstrumentation()
    private val app = ApplicationProvider.getApplicationContext<ZtermApplication>()

    @Test fun separateButtonsCancelBothSystemPickersWithoutRelaunchOrDetach() {
        val hostName = InstrumentationRegistry.getArguments().getString("uploadPickerHost")
        assumeTrue("explicit upload fixture", hostName?.startsWith("upload-") == true)
        val repository = app.repository
        await { repository.state.value.initialized }
        val host = repository.state.value.saved.hosts.first { it.name == hostName }
        ActivityScenario.launch(MainActivity::class.java).use {
            try {
                main { repository.connectHost(host.id) }
                await { repository.frame.value?.state == "active" }
                val session = repository.state.value.sessionId
                val epoch = requireNotNull(repository.frame.value).inputEpoch
                val photo = ui.onNodeWithContentDescription(app.getString(R.string.upload_image))
                val file = ui.onNodeWithContentDescription(app.getString(R.string.upload_file))
                photo.assertIsDisplayed().assertIsEnabled()
                file.assertIsDisplayed().assertIsEnabled()
                assertTrue(photo.fetchSemanticsNode().boundsInRoot.right <= file.fetchSemanticsNode().boundsInRoot.left)
                assertTrue(file.fetchSemanticsNode().boundsInRoot.right <= ui.onNodeWithText("Esc").fetchSemanticsNode().boundsInRoot.left)
                ui.onNodeWithText("Ctrl").assertIsDisplayed()
                ui.onNodeWithContentDescription(app.getString(R.string.show_keyboard)).assertIsDisplayed()
                for (picker in listOf(UploadPicker.File, UploadPicker.Image, UploadPicker.File, UploadPicker.Image)) {
                    val label = app.getString(if (picker == UploadPicker.Image) R.string.upload_image else R.string.upload_file)
                    ui.onNodeWithContentDescription(label).performClick()
                    await { repository.uploads.state.value?.let { it.picker == picker && it.phase == "picking" && it.pickerLaunched } == true }
                    await { instrumentation.uiAutomation.rootInActiveWindow?.packageName?.toString()?.let { it != app.packageName } == true }
                    assertTrue(repository.uploads.paused)
                    instrumentation.sendKeyDownUpSync(KeyEvent.KEYCODE_BACK)
                    await { repository.uploads.state.value == null && instrumentation.uiAutomation.rootInActiveWindow?.packageName?.toString() == app.packageName }
                    assertFalse(repository.uploads.paused)
                    assertEquals("cancel keeps the same Session", session, repository.state.value.sessionId)
                    assertEquals("cancel preserves the IME epoch", epoch, repository.frame.value?.inputEpoch)
                    assertEquals(Route.Terminal, repository.state.value.route)
                    ui.onNodeWithContentDescription(label).assertIsEnabled()
                    // A later frame/recomposition must not relaunch a cancelled picker.
                    ui.waitForIdle()
                    assertNull(repository.uploads.state.value)
                }
            } finally { main { repository.goHome() } }
        }
    }

    private fun main(action: () -> Unit) = instrumentation.runOnMainSync(action)
    // Drive Compose's test clock as well as native/network state. A coroutine
    // delay alone does not advance the LaunchedEffect which opens the picker.
    private fun await(predicate: () -> Boolean) = ui.waitUntil(60_000, predicate)
}
