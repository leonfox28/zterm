package io.github.leonfox28.zterm

import android.content.res.Configuration
import android.os.LocaleList
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.width
import androidx.compose.material3.MaterialTheme
import androidx.compose.runtime.*
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalConfiguration
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.test.*
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.unit.dp
import io.github.leonfox28.zterm.nativebridge.NativeConnectionPath
import org.junit.Rule
import org.junit.Test
import java.util.Locale

class ConnectionStatusUiTest {
    @get:Rule val ui = createComposeRule()

    @Test fun routesStayEnglishAndConnectionTransitionsRetireLatency() {
        var locale by mutableStateOf("en")
        var status by mutableStateOf<TerminalStatus?>(null)
        var busy by mutableStateOf(true)
        ui.setContent {
            val base = LocalContext.current
            val system = LocalConfiguration.current
            val configuration = remember(system, locale) {
                Configuration(system).apply { if (locale != "system") setLocales(LocaleList(Locale.forLanguageTag(locale))) }
            }
            val localized = remember(base, configuration) { base.createConfigurationContext(configuration) }
            CompositionLocalProvider(LocalContext provides localized, LocalConfiguration provides configuration) {
                MaterialTheme {
                    Box(Modifier.width(240.dp)) {
                        TerminalConnectionSubtitle("a-very-long-test-machine-name", status, busy, null)
                    }
                }
            }
        }
        ui.onNodeWithText(" · Connecting…").assertIsDisplayed()
        for (language in listOf("system", "zh", "en")) {
            ui.runOnIdle { locale = language; busy = false; status = connected(NativeConnectionPath.DIRECT, 23u) }
            ui.onNodeWithText(" · Direct · 23 ms").assertIsDisplayed()
            ui.onNodeWithText("a-very-long-test-machine-name").assertIsDisplayed()
            ui.runOnIdle { status = connected(NativeConnectionPath.RELAY, 87u) }
            ui.onNodeWithText(" · Relay · 87 ms").assertIsDisplayed()
            ui.runOnIdle { status = status!!.copy(state = "synchronizing") }
            ui.onNodeWithText(" · Relay · 87 ms").assertIsDisplayed()
            ui.runOnIdle { status = connected(NativeConnectionPath.DIRECT, null) }
            ui.onNodeWithText(" · Direct · — ms").assertIsDisplayed()
        }
        // Even a delayed old sample cannot leak into disconnected chrome.
        for ((state, label) in listOf("reconnecting" to "Reconnecting…", "ended" to "Session ended", "closed" to "Disconnected", "lease_lost" to "Connected elsewhere")) {
            ui.runOnIdle { status = connected(NativeConnectionPath.DIRECT, 23u).copy(state = state, inputReady = false) }
            ui.onNodeWithText(" · $label").assertIsDisplayed()
            ui.onNodeWithText("23 ms", substring = true).assertDoesNotExist()
        }
        ui.runOnIdle { status = connected(NativeConnectionPath.UNKNOWN, null) }
        ui.onNodeWithText(" · Connected · — ms").assertIsDisplayed()
        ui.runOnIdle { status = status!!.copy(state = "synchronizing", inputReady = false) }
        ui.onNodeWithText(" · Connecting…").assertIsDisplayed()
    }

    private fun connected(path: NativeConnectionPath, rtt: UInt?) =
        TerminalStatus(1u, "active", null, null, true, path, rtt)
}
