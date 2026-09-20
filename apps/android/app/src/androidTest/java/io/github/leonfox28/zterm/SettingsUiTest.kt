package io.github.leonfox28.zterm

import androidx.compose.ui.semantics.SemanticsActions
import androidx.compose.ui.semantics.SemanticsProperties
import androidx.compose.ui.test.*
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.test.platform.app.InstrumentationRegistry
import org.junit.Assert.*
import org.junit.Rule
import org.junit.Test

class SettingsUiTest {
    @get:Rule val ui = createAndroidComposeRule<MainActivity>()

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
                ui.onNode(isToggleable()).assertIsDisplayed()
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
                ui.onNode(isToggleable()).assertIsOn().performClick()
                ui.waitUntil { !repository.state.value.notificationsSaving && !repository.state.value.saved.preferences.notificationsEnabled }
                assertFalse(store.load().preferences.notificationsEnabled)
                ui.runOnIdle { repository.updatePreferences { it.copy(fontSize = 14) } }
                ui.waitUntil { repository.state.value.saved.preferences.fontSize == 14 }
                assertFalse(store.load().preferences.notificationsEnabled)
                ui.activityRule.scenario.recreate()
                ui.onNodeWithText("Terminal notifications").performScrollTo()
                ui.onNode(isToggleable()).assertIsOff()
            } else ui.onNode(isToggleable()).assertIsOff()
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
            ui.onNode(isToggleable()).assertIsOff().performClick()
            ui.waitUntil(10_000) { automation.rootInActiveWindow?.findAccessibilityNodeInfosByViewId("com.android.permissioncontroller:id/permission_deny_button")?.isNotEmpty() == true }
            automation.rootInActiveWindow.findAccessibilityNodeInfosByViewId("com.android.permissioncontroller:id/permission_deny_button").first()
                .performAction(android.view.accessibility.AccessibilityNodeInfo.ACTION_CLICK)
            ui.onNode(isToggleable()).assertIsOff()
            assertFalse(repository.state.value.saved.preferences.notificationsEnabled)
            ui.onNode(isToggleable()).performClick()
            ui.waitUntil(10_000) { automation.rootInActiveWindow?.findAccessibilityNodeInfosByViewId("com.android.permissioncontroller:id/permission_allow_button")?.isNotEmpty() == true }
            automation.rootInActiveWindow.findAccessibilityNodeInfosByViewId("com.android.permissioncontroller:id/permission_allow_button").first()
                .performAction(android.view.accessibility.AccessibilityNodeInfo.ACTION_CLICK)
            ui.waitUntil(10_000) { repository.notifications.enabled() && repository.state.value.saved.preferences.notificationsEnabled }
            ui.onNode(isToggleable()).assertIsOn()
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
