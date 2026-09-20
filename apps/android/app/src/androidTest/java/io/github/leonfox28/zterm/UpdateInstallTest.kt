package io.github.leonfox28.zterm

import android.accessibilityservice.AccessibilityService
import androidx.compose.material3.MaterialTheme
import androidx.compose.ui.test.*
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.test.platform.app.InstrumentationRegistry
import kotlinx.coroutines.*
import org.junit.Assert.*
import org.junit.Assume.assumeTrue
import org.junit.Rule
import org.junit.Test
import java.io.File

/** Explicitly enabled only on the task-owned disposable install. Trust is tested in Rust;
 * this fixture copies our installed, matching-signature APK to exercise Android handoff. */
class UpdateInstallTest {
    @get:Rule val ui = createComposeRule()

    @Test fun deniedSourcePermissionReturnsToPromptThenMatchingApkReachesSystemInstaller() {
        assumeTrue(InstrumentationRegistry.getArguments().getString("installerFixture") == "1")
        val instrumentation = InstrumentationRegistry.getInstrumentation()
        val context = instrumentation.targetContext
        val automation = instrumentation.uiAutomation
        fun shell(command: String) = automation.executeShellCommand(command).use {
            android.os.ParcelFileDescriptor.AutoCloseInputStream(it).bufferedReader().use { reader -> reader.readText() }
        }
        val scope = CoroutineScope(SupervisorJob() + Dispatchers.Main.immediate)
        val sourceApk = File(context.applicationInfo.sourceDir)
        val source = object : UpdateSource {
            override suspend fun check() = object : UpdateCandidate {
                override val details = UpdateDetails(BuildConfig.VERSION_NAME, BuildConfig.VERSION_CODE.toLong(), sourceApk.length())
                override fun close() {}
            }
            override suspend fun download(candidate: UpdateCandidate, file: File, progress: (Long) -> Unit) {
                withContext(Dispatchers.IO) { sourceApk.copyTo(file); progress(file.length()) }
            }
        }
        val updates = AppUpdates(source, File(context.cacheDir, "updates"), { null }, {}, scope)
        try {
            assertFalse("start with install-source permission denied", context.packageManager.canRequestPackageInstalls())
            ui.setContent { MaterialTheme { AppUpdateHost(updates, AppState()) } }
            ui.runOnIdle { updates.check() }
            ui.onNodeWithText("Download and install").performClick()
            ui.waitUntil(20_000) { updates.state.value.phase == UpdatePhase.Permission }
            ui.onNodeWithText("Open system settings").performClick()
            ui.waitUntil(10_000) { automation.rootInActiveWindow?.packageName?.toString() == "com.android.settings" }
            automation.performGlobalAction(AccessibilityService.GLOBAL_ACTION_BACK)
            ui.onNodeWithText("Open system settings").assertIsDisplayed()
            assertEquals(UpdatePhase.Permission, updates.state.value.phase)
            shell("appops set ${context.packageName} REQUEST_INSTALL_PACKAGES allow")
            ui.onNodeWithText("Open system settings").performClick()
            ui.waitUntil(10_000) { automation.rootInActiveWindow?.packageName?.toString() == "com.android.settings" }
            automation.performGlobalAction(AccessibilityService.GLOBAL_ACTION_BACK)
            ui.waitUntil(15_000) { automation.rootInActiveWindow?.packageName?.toString()?.contains("packageinstaller") == true }
            ui.waitUntil(15_000) {
                automation.rootInActiveWindow?.findAccessibilityNodeInfosByViewId("android:id/button1")
                    ?.any { it.isEnabled && it.text?.toString() in listOf("Update", "Install") } == true
            }
            assertEquals(UpdatePhase.HandedOff, updates.state.value.phase)
            automation.waitForIdle(300, 5_000)
            val screenshot = automation.takeScreenshot()
            if (screenshot != null) {
                File(context.cacheDir, "update-installer.png").outputStream().use { screenshot.compress(android.graphics.Bitmap.CompressFormat.PNG, 100, it) }
                screenshot.recycle()
            }
            automation.performGlobalAction(AccessibilityService.GLOBAL_ACTION_BACK)
            ui.waitUntil(10_000) { updates.state.value.phase == UpdatePhase.Idle }
        } finally {
            scope.cancel()
            // Reset appops from the host after instrumentation finishes: Android
            // kills the app when this particular permission is revoked.
        }
    }
}
