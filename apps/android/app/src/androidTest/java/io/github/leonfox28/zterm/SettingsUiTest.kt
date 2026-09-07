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
