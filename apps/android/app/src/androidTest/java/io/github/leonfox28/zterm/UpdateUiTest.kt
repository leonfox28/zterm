package io.github.leonfox28.zterm

import android.content.res.Configuration
import android.os.LocaleList
import androidx.compose.foundation.layout.*
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.*
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.test.*
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.unit.Density
import androidx.compose.ui.unit.dp
import androidx.core.content.FileProvider
import androidx.test.platform.app.InstrumentationRegistry
import kotlinx.coroutines.*
import org.junit.Assert.*
import org.junit.Rule
import org.junit.Test
import java.io.File
import java.util.Locale

class UpdateUiTest {
    @get:Rule val ui = createComposeRule()

    @Test fun manualCurrentVersionShowsNativeToastOnceAfterVisiblePendingState() {
        val instrumentation = InstrumentationRegistry.getInstrumentation()
        val context = instrumentation.targetContext
        val automation = instrumentation.uiAutomation
        val scope = CoroutineScope(SupervisorJob() + Dispatchers.Main.immediate)
        val reply = CompletableDeferred<UpdateCandidate?>()
        val count = java.util.concurrent.atomic.AtomicInteger()
        val source = object : UpdateSource {
            override suspend fun check() = reply.await()
            override suspend fun download(candidate: UpdateCandidate, file: File, progress: (Long) -> Unit) = error("no candidate")
        }
        val updates = AppUpdates(source, File(context.cacheDir, "update-toast-test"), { null }, {}, scope)
        automation.setOnAccessibilityEventListener { event ->
            if (event.eventType == android.view.accessibility.AccessibilityEvent.TYPE_NOTIFICATION_STATE_CHANGED &&
                event.className?.toString() == "android.widget.Toast" &&
                event.text.any { it.toString() == context.getString(R.string.update_latest) }) count.incrementAndGet()
        }
        try {
            ui.setContent { MaterialTheme { Column { AboutSettings(updates, true); AppUpdateHost(updates, AppState()) } } }
            ui.onNodeWithText("Check for updates").performClick()
            ui.onNodeWithText(context.getString(R.string.update_checking)).assertIsDisplayed()
            ui.runOnIdle { reply.complete(null) }
            ui.waitUntil(5_000) { count.get() > 0 }
            ui.waitForIdle()
            assertEquals(1, count.get())
            ui.onNodeWithText(context.getString(R.string.update_latest)).assertDoesNotExist()
            assertNull(updates.state.value.notice)
        } finally { automation.setOnAccessibilityEventListener(null); scope.cancel() }
    }

    @Test fun failureAndRetryCardsRetainWidthAndFitBothLocalesThemesAndLargeText() {
        var language by mutableStateOf("en")
        var busy by mutableStateOf(false)
        var dark by mutableStateOf(true)
        var large by mutableStateOf(false)
        var compact by mutableStateOf(false)
        ui.setContent {
            val base = LocalContext.current
            val configuration = Configuration(LocalConfiguration.current).apply { setLocales(LocaleList(Locale.forLanguageTag(language))) }
            val context = base.createConfigurationContext(configuration)
            val density = LocalDensity.current
            CompositionLocalProvider(LocalContext provides context, LocalConfiguration provides configuration,
                LocalDensity provides Density(density.density, if (large) 1.6f else 1f)) {
                MaterialTheme(colorScheme = if (dark) darkColorScheme() else lightColorScheme()) {
                    Box(Modifier.width(if (compact) 320.dp else 390.dp).heightIn(max = if (compact) 300.dp else 680.dp)) {
                        WideStatusCard(stringResource(if (busy) R.string.connecting else R.string.connection_failed_title),
                            stringResource(if (busy) R.string.connection_pending_body else R.string.connection_failure_body),
                            Modifier.padding(24.dp).testTag("status-card"), busy = busy) {
                            Row(horizontalArrangement = Arrangement.spacedBy(12.dp)) {
                                FilledTonalButton({}, Modifier.weight(1f).heightIn(min = 48.dp)) { Text(stringResource(R.string.sessions)) }
                                Button({}, Modifier.weight(1f).heightIn(min = 48.dp), enabled = !busy) { Text(stringResource(R.string.retry)) }
                            }
                        }
                    }
                }
            }
        }
        for (locale in listOf("en", "zh")) for (night in listOf(true, false)) {
            ui.runOnIdle { language = locale; dark = night; large = false; compact = false; busy = false }
            val card = ui.onNodeWithTag("status-card")
            card.assertWidthIsEqualTo(342.dp).assertHeightIsAtLeast(284.dp)
            val previous = card.getUnclippedBoundsInRoot()
            ui.runOnIdle { busy = true }
            assertEquals(previous, card.getUnclippedBoundsInRoot())
            ui.runOnIdle { large = true; compact = true; busy = false }
            card.assertWidthIsEqualTo(272.dp)
            ui.onNodeWithText(if (locale == "en") "Retry" else "重试").performScrollTo().assertIsDisplayed()
        }
    }

    @Test fun automaticPromptDefersWhileBlockedAndLaterNeverStartsDownload() {
        val context = InstrumentationRegistry.getInstrumentation().targetContext
        val scope = CoroutineScope(SupervisorJob() + Dispatchers.Main.immediate)
        var checks = 0
        var downloads = 0
        val source = object : UpdateSource {
            override suspend fun check(): UpdateCandidate {
                checks++
                return object : UpdateCandidate {
                    override val details = UpdateDetails("99.0.0", 999_999, 3)
                    override fun close() {}
                }
            }
            override suspend fun download(candidate: UpdateCandidate, file: File, progress: (Long) -> Unit) { downloads++ }
        }
        val updates = AppUpdates(source, File(context.cacheDir, "update-ui-test"), { null }, {}, scope)
        var route by mutableStateOf(Route.Terminal)
        val blocks = mutableIntStateOf(0)
        try {
            ui.setContent { CompositionLocalProvider(LocalUpdateBlocks provides blocks) {
                MaterialTheme { AppUpdateHost(updates, AppState(route = route)) }
            } }
            ui.runOnIdle { updates.startup() }
            ui.onNodeWithText("Update available").assertDoesNotExist()
            ui.runOnIdle { route = Route.Home; blocks.intValue = 1 }
            ui.onNodeWithText("Update available").assertDoesNotExist()
            ui.runOnIdle { blocks.intValue = 0 }
            ui.onNodeWithText("Update available").assertIsDisplayed()
            ui.onNodeWithText("Later").performClick()
            assertEquals(1, checks)
            assertEquals(0, downloads)
        } finally { scope.cancel() }
    }

    @Test fun privateApkUriIsReadableButOtherCacheFilesCannotBeShared() {
        val context = InstrumentationRegistry.getInstrumentation().targetContext
        val file = File(context.cacheDir, "updates/update-uri-test.apk").apply { parentFile!!.mkdirs(); writeText("test-only fixture") }
        try {
            val uri = FileProvider.getUriForFile(context, context.packageName + ".updates", file)
            assertEquals("content", uri.scheme)
            assertEquals("test-only fixture", context.contentResolver.openInputStream(uri)!!.bufferedReader().use { it.readText() })
            assertTrue(runCatching {
                FileProvider.getUriForFile(context, context.packageName + ".updates", File(context.cacheDir, "private-state.json"))
            }.exceptionOrNull() is IllegalArgumentException)
        } finally { file.delete() }
    }
}
