package io.github.leonfox28.zterm

import androidx.compose.ui.semantics.SemanticsActions
import androidx.compose.ui.semantics.SemanticsProperties
import androidx.compose.ui.test.*
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.test.platform.app.InstrumentationRegistry
import io.github.leonfox28.zterm.nativebridge.diagnosticRemainingMs
import org.junit.Assert.*
import org.junit.Rule
import org.junit.Test

class SettingsUiTest {
    @get:Rule val ui = createAndroidComposeRule<MainActivity>()

    @Test fun diagnosticsExportRetainsDetailWhileTheSystemPickerRecreatesActivity() {
        val instrumentation = InstrumentationRegistry.getInstrumentation()
        org.junit.Assume.assumeTrue(InstrumentationRegistry.getArguments().getString("diagnosticsExportFixture") == "1")
        val application = ui.activity.application as ZtermApplication
        val repository = application.repository
        val diagnostics = application.diagnostics
        ui.waitUntil(20_000) { repository.state.value.initialized }
        val previous = repository.state.value.saved.preferences
        val filename = "zterm-diagnostics-test-${java.util.UUID.randomUUID()}.jsonl"
        val output = "/sdcard/Download/$filename"
        val automation = instrumentation.uiAutomation
        fun shell(command: String): String = automation.executeShellCommand(command).use {
            android.os.ParcelFileDescriptor.AutoCloseInputStream(it).bufferedReader().use { reader -> reader.readText() }
        }
        val monitor = object : android.app.Instrumentation.ActivityMonitor() {
            override fun onStartActivity(intent: android.content.Intent): android.app.Instrumentation.ActivityResult? {
                if (intent.action == android.content.Intent.ACTION_CREATE_DOCUMENT) intent.putExtra(android.content.Intent.EXTRA_TITLE, filename)
                return null
            }
        }
        instrumentation.addMonitor(monitor)
        try {
            ui.runOnIdle {
                repository.setPreferences(previous.copy(language = "en"))
                repository.show(Route.Settings)
                diagnostics.setEnabled(true)
            }
            ui.waitUntil(5_000) { !diagnostics.state.value.busy && diagnostics.native.detailEnabled() }
            diagnostics.record(io.github.leonfox28.zterm.nativebridge.AppDiagnostic.VIEWPORT, columns = 80u, rows = 24u)
            assertTrue(diagnostics.native.flush())
            ui.onNodeWithTag("diagnostics-include-detail").performScrollTo().assertIsOff().performClick()
            val originalActivity = ui.activity
            ui.onNodeWithText("Export logs").performScrollTo().performClick()
            ui.waitUntil(10_000) { automation.rootInActiveWindow?.findAccessibilityNodeInfosByViewId("android:id/button1")?.isNotEmpty() == true }
            // ActivityScenario.recreate requires RESUMED; the real picker has
            // stopped this Activity. Request Android's recreation directly.
            instrumentation.runOnMainSync { originalActivity.recreate() }
            ui.waitUntil(10_000) { originalActivity.isDestroyed }
            automation.rootInActiveWindow.findAccessibilityNodeInfosByViewId("android:id/button1").first()
                .performAction(android.view.accessibility.AccessibilityNodeInfo.ACTION_CLICK)
            ui.waitUntil(10_000) { shell("cat $output").startsWith("{\"export_schema\"") }
            ui.waitUntil { !diagnostics.state.value.busy }
            assertFalse(diagnostics.state.value.failed)
            val exported = shell("cat $output")
            assertTrue(org.json.JSONObject(exported.lineSequence().first()).getBoolean("includes_debug"))
            assertTrue(exported.contains("viewport_changed"))
            ui.onNodeWithTag("diagnostics-include-detail").performScrollTo().assertIsOn()
        } finally {
            instrumentation.removeMonitor(monitor)
            if (automation.rootInActiveWindow?.packageName?.contains("documentsui") == true) shell("input keyevent KEYCODE_BACK")
            shell("rm -f $output")
            ui.runOnIdle { diagnostics.setEnabled(false); repository.setPreferences(previous); repository.show(Route.Home) }
            ui.waitUntil { !diagnostics.state.value.busy && repository.state.value.saved.preferences == previous }
        }
    }

    @Test fun diagnosticsSwitchRenewRecreationAndPickerCancellation() {
        val application = ui.activity.application as ZtermApplication
        val repository = application.repository
        val diagnostics = application.diagnostics
        ui.waitUntil(20_000) { repository.state.value.initialized }
        val previous = repository.state.value.saved.preferences
        try {
            ui.runOnIdle {
                repository.setPreferences(previous.copy(language = "en"))
                repository.show(Route.Settings)
                diagnostics.setEnabled(false)
            }
            ui.waitUntil { !diagnostics.state.value.busy && diagnostics.state.value.remainingSeconds == 0L }
            ui.onNodeWithTag("diagnostics-detail").performScrollTo().assertIsOff().performClick()
            ui.waitUntil(5_000) { diagnostics.state.value.remainingSeconds > 0 && diagnostics.native.detailEnabled() }
            ui.onNodeWithText("Renew for 15 minutes").performScrollTo().performClick()
            ui.waitUntil { !diagnostics.state.value.busy }
            ui.onNodeWithTag("diagnostics-include-detail").performScrollTo().assertIsOff().performClick()
            ui.activityRule.scenario.recreate()
            ui.onNodeWithTag("diagnostics-include-detail").performScrollTo().assertIsOn()
            ui.onNodeWithTag("diagnostics-detail").performScrollTo().assertIsOn()
            assertTrue(diagnosticRemainingMs(diagnostics.files.control()) > 0uL)
            val instrumentation = InstrumentationRegistry.getInstrumentation()
            var opened: android.content.Intent? = null
            val monitor = object : android.app.Instrumentation.ActivityMonitor() {
                override fun onStartActivity(intent: android.content.Intent): android.app.Instrumentation.ActivityResult? {
                    if (intent.action != android.content.Intent.ACTION_CREATE_DOCUMENT) return null
                    opened = intent
                    return android.app.Instrumentation.ActivityResult(android.app.Activity.RESULT_CANCELED, null)
                }
            }
            instrumentation.addMonitor(monitor)
            try {
                ui.onNodeWithText("Export logs").performScrollTo().performClick()
                ui.waitForIdle()
                assertEquals("application/x-ndjson", opened?.type)
                assertFalse(diagnostics.state.value.busy)
                assertFalse(diagnostics.state.value.failed)
            } finally { instrumentation.removeMonitor(monitor) }
            ui.onNodeWithTag("diagnostics-detail").performScrollTo().performClick()
            ui.waitUntil(5_000) { diagnostics.state.value.remainingSeconds == 0L && !diagnostics.native.detailEnabled() }
            ui.onNodeWithTag("diagnostics-detail").assertIsOff()
        } finally {
            ui.runOnIdle { diagnostics.setEnabled(false); repository.setPreferences(previous); repository.show(Route.Home) }
            ui.waitUntil { repository.state.value.saved.preferences == previous && !diagnostics.state.value.busy }
        }
    }

    @Test fun failedNotificationSaveRestoresCommittedGate() {
        org.junit.Assume.assumeTrue(InstrumentationRegistry.getArguments().getString("notificationSaveFailureFixture") == "1")
        val repository = (ui.activity.application as ZtermApplication).repository
        ui.waitUntil(20_000) { repository.state.value.initialized }
        val previous = repository.state.value.saved.preferences
        val blocked = java.io.File(ui.activity.noBackupFilesDir, "zterm/state.json.new")
        var ownsBlocker = false
        assertFalse(blocked.exists())
        try {
            val configured = previous.copy(notificationsEnabled = true, fontSize = if (previous.fontSize == 16) 15 else 16)
            ui.runOnIdle { repository.setPreferences(configured) }
            ui.waitUntil { repository.state.value.saved.preferences == configured }
            assertTrue(blocked.mkdir())
            ownsBlocker = true
            java.io.File(blocked, "test-owned-blocker").writeText("force AtomicFile write failure")
            ui.runOnIdle {
                repository.setNotificationsEnabled(false)
                assertFalse("Off applies before disk completion", repository.notifications.appEnabled.value)
            }
            ui.waitUntil { !repository.state.value.notificationsSaving && repository.state.value.error == "storage_unavailable" }
            assertTrue(repository.notifications.appEnabled.value)
            assertTrue(repository.state.value.saved.preferences.notificationsEnabled)
            assertTrue(AppStore(ui.activity).load().preferences.notificationsEnabled)
        } finally {
            if (ownsBlocker) blocked.deleteRecursively()
            ui.runOnIdle { repository.clearError(); repository.setPreferences(previous); repository.show(Route.Home) }
            ui.waitUntil { repository.state.value.saved.preferences == previous && !repository.state.value.busy }
        }
    }

    @Test fun settingsFooterFitsBothLanguagesAndThemes() {
        val repository = (ui.activity.application as ZtermApplication).repository
        ui.waitUntil(20_000) { repository.state.value.initialized }
        val previous = repository.state.value.saved.preferences
        try {
            ui.runOnIdle { repository.show(Route.Settings) }
            for (language in listOf("en", "zh")) for (theme in listOf("dark", "light")) {
                ui.runOnIdle { repository.updatePreferences { it.copy(language = language, theme = theme) } }
                ui.waitUntil { repository.state.value.saved.preferences.let { it.language == language && it.theme == theme } }
                ui.onNodeWithText(if (language == "en") "Check for updates" else "检查更新").performScrollTo().assertIsDisplayed()
                ui.onNodeWithText("GitHub").assertIsDisplayed()
                ui.onNodeWithTag("terminal-notifications").performScrollTo().assertIsDisplayed()
                if (InstrumentationRegistry.getArguments().getString("visualReview") == "1") {
                    val bitmap = InstrumentationRegistry.getInstrumentation().uiAutomation.takeScreenshot()
                    java.io.File(ui.activity.cacheDir, "settings-$language-$theme.png").outputStream().use {
                        bitmap.compress(android.graphics.Bitmap.CompressFormat.PNG, 100, it)
                    }
                    bitmap.recycle()
                }
            }
        } finally {
            ui.runOnIdle { repository.setPreferences(previous); repository.show(Route.Home) }
            ui.waitUntil { repository.state.value.saved.preferences == previous }
        }
    }

    @Test fun notificationSwitchPersistsAlongsideOtherSettingsAndAboutActionsStayVisible() {
        val repository = (ui.activity.application as ZtermApplication).repository
        ui.waitUntil(20_000) { repository.state.value.initialized }
        val previous = repository.state.value.saved.preferences
        val store = AppStore(ui.activity)
        try {
            ui.runOnIdle {
                repository.goHome()
                repository.setPreferences(previous.copy(language = "en", theme = "dark"))
            }
            ui.waitUntil { repository.state.value.saved.preferences.language == "en" && !repository.state.value.busy }
            ui.onNodeWithContentDescription("Settings").performClick()
            ui.onNodeWithText("Terminal notifications").performScrollTo().assertIsDisplayed()
            if (repository.notifications.enabled()) {
                ui.onNodeWithTag("terminal-notifications").assertIsOn().performClick()
                ui.waitUntil { !repository.state.value.notificationsSaving && !repository.state.value.saved.preferences.notificationsEnabled }
                assertFalse(store.load().preferences.notificationsEnabled)
                ui.runOnIdle { repository.updatePreferences { it.copy(fontSize = 14) } }
                ui.waitUntil { repository.state.value.saved.preferences.fontSize == 14 }
                assertFalse(store.load().preferences.notificationsEnabled)
                ui.activityRule.scenario.recreate()
                ui.onNodeWithText("Terminal notifications").performScrollTo()
                ui.onNodeWithTag("terminal-notifications").assertIsOff()
            } else ui.onNodeWithTag("terminal-notifications").assertIsOff()
            ui.onNodeWithText("GitHub").performScrollTo().assertIsDisplayed()
            val instrumentation = InstrumentationRegistry.getInstrumentation()
            var opened: android.content.Intent? = null
            val monitor = object : android.app.Instrumentation.ActivityMonitor() {
                override fun onStartActivity(intent: android.content.Intent): android.app.Instrumentation.ActivityResult? {
                    if (intent.action != android.content.Intent.ACTION_VIEW) return null
                    opened = intent
                    return android.app.Instrumentation.ActivityResult(android.app.Activity.RESULT_CANCELED, null)
                }
            }
            instrumentation.addMonitor(monitor)
            try {
                ui.onNodeWithText("GitHub").performClick()
                assertEquals("https://github.com/leonfox28/zterm", opened?.dataString)
            } finally { instrumentation.removeMonitor(monitor) }
            ui.onNodeWithText("Check for updates").performScrollTo().assertIsDisplayed()
            ui.onNodeWithText("Current version " + BuildConfig.VERSION_NAME).assertIsDisplayed()
        } finally {
            ui.runOnIdle { repository.setPreferences(previous); repository.show(Route.Home) }
            ui.waitUntil { repository.state.value.saved.preferences == previous }
        }
    }

    @Test fun notificationPermissionDenialStaysOffAndLaterGrantEnablesDelivery() {
        org.junit.Assume.assumeTrue(InstrumentationRegistry.getArguments().getString("notificationPermissionFixture") == "1")
        val instrumentation = InstrumentationRegistry.getInstrumentation()
        val automation = instrumentation.uiAutomation
        val repository = (ui.activity.application as ZtermApplication).repository
        ui.waitUntil(20_000) { repository.state.value.initialized }
        val previous = repository.state.value.saved.preferences
        try {
            ui.runOnIdle {
                repository.setPreferences(previous.copy(language = "en", notificationsEnabled = false, notificationPermissionRequested = false))
                repository.show(Route.Settings)
            }
            ui.waitUntil { !repository.state.value.saved.preferences.notificationsEnabled && !repository.state.value.busy }
            assertTrue(repository.notifications.needsPermission())
            ui.onNodeWithText("Terminal notifications").performScrollTo()
            ui.onNodeWithTag("terminal-notifications").assertIsOff().performClick()
            ui.waitUntil(10_000) { automation.rootInActiveWindow?.findAccessibilityNodeInfosByViewId("com.android.permissioncontroller:id/permission_deny_button")?.isNotEmpty() == true }
            automation.rootInActiveWindow.findAccessibilityNodeInfosByViewId("com.android.permissioncontroller:id/permission_deny_button").first()
                .performAction(android.view.accessibility.AccessibilityNodeInfo.ACTION_CLICK)
            ui.onNodeWithTag("terminal-notifications").assertIsOff()
            assertFalse(repository.state.value.saved.preferences.notificationsEnabled)
            ui.onNodeWithTag("terminal-notifications").performClick()
            ui.waitUntil(10_000) { automation.rootInActiveWindow?.findAccessibilityNodeInfosByViewId("com.android.permissioncontroller:id/permission_allow_button")?.isNotEmpty() == true }
            automation.rootInActiveWindow.findAccessibilityNodeInfosByViewId("com.android.permissioncontroller:id/permission_allow_button").first()
                .performAction(android.view.accessibility.AccessibilityNodeInfo.ACTION_CLICK)
            ui.waitUntil(10_000) { repository.notifications.enabled() && repository.state.value.saved.preferences.notificationsEnabled }
            ui.onNodeWithTag("terminal-notifications").assertIsOn()
        } finally {
            ui.runOnIdle { repository.setPreferences(previous); repository.show(Route.Home) }
            ui.waitUntil { repository.state.value.saved.preferences == previous }
        }
    }

    @Test fun fontSliderUsesUnitStepsAndRetainsEverySizeAfterRecreation() {
        val repository = (ui.activity.application as ZtermApplication).repository
        ui.waitUntil(20_000) { repository.state.value.initialized }
        val previous = repository.state.value.saved.preferences
        val store = AppStore(InstrumentationRegistry.getInstrumentation().targetContext)
        try {
            ui.runOnIdle { repository.goHome(); repository.setPreferences(Preferences("en", "dark", 12)) }
            ui.waitUntil { repository.state.value.saved.preferences == Preferences("en", "dark", 12) && !repository.state.value.busy }
            ui.onNodeWithContentDescription("Settings").performClick()
            val slider = ui.onNodeWithContentDescription("Font size")
            slider.performScrollTo().assertIsDisplayed()
            val range = slider.fetchSemanticsNode().config[SemanticsProperties.ProgressBarRangeInfo]
            assertEquals(8f..16f, range.range)
            assertEquals("nine values including both endpoints", 7, range.steps)

            slider.performTouchInput { swipe(center, centerRight, 500) }
            ui.waitForIdle()
            assertEquals("drag thumb to upper endpoint", 16f, slider.fetchSemanticsNode().config[SemanticsProperties.ProgressBarRangeInfo].current)
            ui.waitUntil { repository.state.value.saved.preferences.fontSize == 16 }
            slider.performTouchInput { swipe(center, centerLeft, 500) }
            ui.waitForIdle()
            assertEquals("drag track to lower endpoint", 8f, slider.fetchSemanticsNode().config[SemanticsProperties.ProgressBarRangeInfo].current)
            ui.waitUntil { repository.state.value.saved.preferences.fontSize == 8 }

            for (font in 8..16) {
                slider.performSemanticsAction(SemanticsActions.SetProgress) { it(font.toFloat()) }
                ui.waitUntil { repository.state.value.saved.preferences.fontSize == font }
                assertEquals("persist integer size $font without changing other preferences",
                    Preferences("en", "dark", font), store.load().preferences)
            }
            slider.performSemanticsAction(SemanticsActions.SetProgress) { it(11f) }
            ui.waitUntil { repository.state.value.saved.preferences.fontSize == 11 }
            ui.activityRule.scenario.recreate()
            ui.onNodeWithContentDescription("Font size").performScrollTo()
                .assertRangeInfoEquals(androidx.compose.ui.semantics.ProgressBarRangeInfo(11f, 8f..16f, 7))
            assertEquals(11, store.load().preferences.fontSize)
        } finally {
            ui.runOnIdle { repository.setPreferences(previous); repository.show(Route.Home) }
            ui.waitUntil { repository.state.value.saved.preferences == previous }
        }
    }
}
