package io.github.leonfox28.zterm

import android.content.ContentValues
import android.os.SystemClock
import android.provider.MediaStore
import android.view.accessibility.AccessibilityNodeInfo
import androidx.compose.ui.test.*
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.test.platform.app.InstrumentationRegistry
import kotlinx.coroutines.runBlocking
import org.junit.Assert.*
import org.junit.Assume.assumeTrue
import org.junit.Rule
import org.junit.Test
import java.io.File

class ScannerUiTest {
    @get:Rule val ui = createAndroidComposeRule<MainActivity>()
    private val instrumentation = InstrumentationRegistry.getInstrumentation()
    private val repository get() = (ui.activity.application as ZtermApplication).repository

    @Test fun cameraPreviewPairsAnExplicitVirtualScenePoster() {
        assumeTrue("explicit emulator camera poster", InstrumentationRegistry.getArguments().getString("cameraPoster") == "1")
        ui.waitUntil(20_000) { repository.state.value.initialized }
        try {
            ui.runOnIdle { repository.show(Route.Scanner) }
            ui.waitUntil(35_000) { repository.state.value.route == Route.Terminal && !repository.state.value.busy }
            assertTrue(repository.state.value.saved.hosts.any { it.name == "my-mac" })
        } finally { ui.runOnIdle { repository.goHome() } }
    }

    @Test fun manualAndSystemImagePickerPairTheRealHost() {
        val directory = instrumentation.targetContext.filesDir
        val ticket = File(directory,"qr-manual.txt")
        val png = File(directory,"qr-gallery.png")
        assumeTrue("explicit host-created tickets", ticket.isFile && png.isFile)
        ui.waitUntil(20_000) { repository.state.value.initialized }
        val preferences = repository.state.value.saved.preferences
        val automation = instrumentation.uiAutomation
        automation.serviceInfo = automation.serviceInfo.apply { flags = flags or android.accessibilityservice.AccessibilityServiceInfo.FLAG_RETRIEVE_INTERACTIVE_WINDOWS }
        val resolver = instrumentation.targetContext.contentResolver
        val photo = requireNotNull(resolver.insert(MediaStore.Images.Media.EXTERNAL_CONTENT_URI,ContentValues().apply {
            put(MediaStore.Images.Media.DISPLAY_NAME,"zterm-acceptance-${System.nanoTime()}.png")
            put(MediaStore.Images.Media.MIME_TYPE,"image/png")
        }))
        try {
            resolver.openOutputStream(photo).use { output -> png.inputStream().use { it.copyTo(requireNotNull(output)) } }
            ui.runOnIdle { repository.setPreferences(Preferences("en","dark")); repository.show(Route.Scanner) }
            ui.waitUntil { repository.state.value.saved.preferences.language == "en" }
            ui.onNodeWithText("Enter credentials").performClick()
            ui.onNodeWithText("Connect").assertIsNotEnabled()
            ui.onNode(hasSetTextAction()).performTextInput("invalid")
            ui.onNodeWithText("Connect").performClick()
            ui.onNodeWithText("Invalid credentials").assertIsDisplayed()
            ui.onNode(hasSetTextAction()).performTextReplacement(ticket.readText().trim())
            ui.onNodeWithText("Connect").performClick()
            ui.waitUntil(30_000) { repository.state.value.route == Route.Terminal && !repository.state.value.busy }
            assertTrue("confirmed pairing saved before success", repository.state.value.saved.hosts.any { it.name == "my-mac" })
            ui.runOnIdle { repository.goHome() }
            ui.waitUntil { repository.frame.value == null && !repository.state.value.busy }
            ui.runOnIdle { repository.show(Route.Scanner) }
            ui.onNodeWithText("Choose image").performClick()
            var chosen = false
            repeat(50) {
                if (!chosen) {
                    fun find(node: AccessibilityNodeInfo): AccessibilityNodeInfo? {
                        val description = node.contentDescription?.toString().orEmpty()
                        if (description.startsWith("Photo taken") || description.startsWith("Photo, taken")) return node
                        for (index in 0 until node.childCount) node.getChild(index)?.let { find(it) }?.let { return it }
                        return null
                    }
                    for (window in automation.windows) window.root?.let(::find)?.let { node ->
                        var target: AccessibilityNodeInfo? = node
                        while (target != null && !target.isClickable) target = target.parent
                        if (target?.performAction(AccessibilityNodeInfo.ACTION_CLICK) == true) chosen = true
                    }
                    SystemClock.sleep(100)
                }
            }
            assertTrue("selected fixture using system photo picker", chosen)
            ui.waitUntil(30_000) { repository.state.value.route == Route.Terminal && !repository.state.value.busy }
            val host = repository.state.value.saved.hosts.first { it.name == "my-mac" }
            runBlocking { repository.runtime.listSessions(host.id) }
            assertEquals("saved public host identity is unique", 1, repository.state.value.saved.hosts.count { it.id == host.id })
        } finally {
            resolver.delete(photo,null,null)
            ticket.delete(); png.delete()
            ui.runOnIdle { repository.goHome(); repository.setPreferences(preferences) }
        }
    }
}
