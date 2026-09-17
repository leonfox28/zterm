package io.github.leonfox28.zterm

import android.Manifest
import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.os.Build
import androidx.lifecycle.Lifecycle
import androidx.test.core.app.ActivityScenario
import androidx.test.core.app.ApplicationProvider
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import io.github.leonfox28.zterm.nativebridge.NativeNotification
import io.github.leonfox28.zterm.nativebridge.NativeException
import io.github.leonfox28.zterm.nativebridge.NativeSession
import io.github.leonfox28.zterm.nativebridge.NativeViewport
import kotlinx.coroutines.delay
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.TimeoutCancellationException
import kotlinx.coroutines.withTimeout
import org.junit.Assert.*
import org.junit.Assume.assumeTrue
import org.junit.Test
import org.junit.runner.RunWith
import java.io.File

/** Permission/channel cases run separately on a disposable app install. The live
 * fixture requires notificationFixture=1 and a private notification-ticket.txt. */
@RunWith(AndroidJUnit4::class)
class TerminalNotificationsTest {
    private val instrumentation = InstrumentationRegistry.getInstrumentation()
    private val context = instrumentation.targetContext
    private val manager = context.getSystemService(NotificationManager::class.java)
    private var phase = "platform setup"

    @Test fun grantedPermissionPostsDistinctTypedNotifications() = runBlocking {
        val notifications = TerminalNotifications(context)
        assumeTrue("grant notification permission on the disposable install", notifications.enabled())
        manager.cancelAll()
        try {
            assertTrue(notifications.post(NativeNotification(1u, null, "结果: done"), "en"))
            assertTrue(notifications.post(NativeNotification(1u, "标题", "body;结果"), "en"))
            assertTrue(notifications.post(NativeNotification(1u, null, "结果: done"), "en"))
            await { postedContent().size == 3 }
            val posted = postedContent().map { it.notification }
            assertEquals(2, posted.count { it.extras.getCharSequence(Notification.EXTRA_TEXT)?.toString() == "结果: done" })
            val titled = posted.single { it.extras.getCharSequence(Notification.EXTRA_TITLE)?.toString() == "标题" }
            assertEquals("body;结果", titled.extras.getCharSequence(Notification.EXTRA_BIG_TEXT)?.toString())
            assertEquals(TerminalNotifications.CHANNEL, titled.channelId)
            assertEquals(Notification.VISIBILITY_PRIVATE, titled.visibility)
            assertTrue(titled.flags and Notification.FLAG_AUTO_CANCEL != 0)
            assertNotNull(titled.contentIntent)
        } finally { manager.cancelAll() }
    }

    @Test fun deniedPermissionConsumesWithoutPosting() = runBlocking {
        val notifications = TerminalNotifications(context)
        assumeTrue("run with notification permission denied", Build.VERSION.SDK_INT >= 33 && notifications.needsPermission())
        val before = postedContent().size
        assertFalse(notifications.enabled())
        assertFalse(notifications.post(NativeNotification(1u, null, "skipped"), "en"))
        assertEquals(before, postedContent().size)
        instrumentation.uiAutomation.grantRuntimePermission(context.packageName, Manifest.permission.POST_NOTIFICATIONS)
        await { notifications.enabled() }
        try {
            assertTrue(notifications.post(NativeNotification(1u, null, "after grant"), "en"))
            await { postedContent().size == before + 1 }
            assertFalse(postedContent().any { it.notification.extras.getCharSequence(Notification.EXTRA_TEXT)?.toString() == "skipped" })
        } finally { manager.cancelAll() }
    }

    @Test fun disabledChannelConsumesWithoutOverridingUserPolicy() {
        assumeTrue("explicit disposable channel fixture", InstrumentationRegistry.getArguments().getString("blockedChannel") == "1")
        manager.createNotificationChannel(NotificationChannel(TerminalNotifications.CHANNEL, "Blocked fixture", NotificationManager.IMPORTANCE_NONE))
        val notifications = TerminalNotifications(context)
        assertFalse(notifications.enabled())
        assertFalse(notifications.post(NativeNotification(1u, "title", "skipped"), "zh"))
        assertEquals(NotificationManager.IMPORTANCE_NONE, manager.getNotificationChannel(TerminalNotifications.CHANNEL).importance)
        assertTrue(postedContent().isEmpty())
    }

    @Test fun liveConnectionPostsInBackgroundWithoutRecreationOrReconnectReplay() = runBlocking {
        assumeTrue("explicit disposable host", InstrumentationRegistry.getArguments().getString("notificationFixture") == "1")
        val app = ApplicationProvider.getApplicationContext<ZtermApplication>()
        val runtime = app.runtime
        val store = AppStore(app)
        val saved = store.load()
        val seed = store.loadOrCreateSeed()
        try { runtime.initialize(seed, PlatformNetwork(app) {}.current(), saved.hosts.map { it.native() }) }
        finally { seed.fill(0) }
        val host = if (saved.hosts.isEmpty()) {
            val ticket = File(app.filesDir, "notification-ticket.txt")
            runtime.pairTicket(ticket.readText().trim()).use { pairing ->
                val value = pairing.host()
                assertEquals("notification-acceptance", value.name)
                val host = SavedHost(value.deviceId, value.name, value.relayUrls, null)
                store.save(saved.copy(hosts = listOf(host), recent = null, preferences = Preferences("en", "dark", 12)))
                runtime.commitPairing(pairing)
                ticket.delete()
                host
            }
        } else {
            saved.hosts.single().also { assertEquals("notification-acceptance", it.name) }
        }
        val repository = app.repository
        phase = "repository initialization"
        await { repository.state.value.initialized }
        assertTrue(repository.notifications.enabled())
        // Discovery on the public relay can outlast a single read-only deadline.
        val sessions = withTimeout(45_000) {
            var sessions: List<NativeSession>? = null
            while (sessions == null) {
                try { sessions = runtime.listSessions(host.id) }
                catch (error: NativeException.RequestFailed) {
                    if (error.code !in setOf("deadline_exceeded", "transport_unavailable")) throw error
                    delay(500)
                }
            }
            sessions
        }
        assertTrue(sessions.isEmpty())
        manager.cancelAll()
        val directory = requireNotNull(InstrumentationRegistry.getArguments().getString("fixtureDirectory"))
        val session = runtime.createSession(host.id, "android-notifications", directory, NativeViewport(24u, 80u), true).sessionId
        try {
            ActivityScenario.launch(MainActivity::class.java).use { activity ->
                phase = "initial attachment"
                main { repository.connectHost(host.id) }
                await { repository.frame.value?.state == "active" && !repository.state.value.busy }
                assertEquals(session, repository.state.value.sessionId)
                phase = "foreground notifications"
                main { assertTrue(repository.text("printf '\\033]9;结果: done\\a\\033]777;notify;标题;body;结果\\033\\\\'\r")) }
                await { postedContent().size == 2 }
                assertTrue(postedContent().any { it.notification.extras.getCharSequence(Notification.EXTRA_TITLE)?.toString() == "标题" })
                phase = "background notification"
                main { assertTrue(repository.text("sleep 1; printf '\\033]9;background\\a'\r")) }
                activity.moveToState(Lifecycle.State.CREATED)
                await { postedContent().size == 3 }
                assertEquals("active", repository.frame.value?.state)
                activity.moveToState(Lifecycle.State.RESUMED)
                activity.recreate()
                phase = "recreation output barrier"
                main { assertTrue(repository.text("printf 'NOTIFICATION_%s\\n' BARRIER\r")) }
                await { text(repository).contains("NOTIFICATION_BARRIER") }
                assertEquals("recreation does not replay a transient event", 3, postedContent().size)
                // The file is an execution barrier for output generated while detached.
                phase = "detached output arm"
                main { assertTrue(repository.text("rm -f notification-offline-done; printf 'OFFLINE_%s\\n' ARMED; sleep 6; printf '\\033]9;missed\\a'; touch notification-offline-done\r")) }
                await { text(repository).contains("OFFLINE_ARMED") }
                phase = "detach"
                main { repository.goHome() }
                await { repository.frame.value == null && !repository.state.value.busy }
                await { !runtime.listSessions(host.id).single { it.sessionId == session }.occupied }
                delay(7_000)
                assertEquals("detached output is not posted", 3, postedContent().size)
                phase = "reattach"
                main { repository.connectHost(host.id, recent = true) }
                await { repository.frame.value?.state == "active" && !repository.state.value.busy }
                assertEquals(session, repository.state.value.sessionId)
                phase = "fresh notification after reattach"
                main { assertTrue(repository.text("test -f notification-offline-done && printf '\\033]9;fresh\\a'\r")) }
                await { postedContent().size == 4 }
                assertFalse(postedContent().any { it.notification.extras.getCharSequence(Notification.EXTRA_TEXT)?.toString() == "missed" })
                assertTrue(postedContent().any { it.notification.extras.getCharSequence(Notification.EXTRA_TEXT)?.toString() == "fresh" })
            }
        } finally {
            phase = "cleanup detach"
            main { repository.goHome() }
            await { repository.frame.value == null }
            runtime.closeSession(host.id, session)
            manager.cancelAll()
        }
    }

    private fun text(repository: AppRepository): String = repository.frame.value?.rows?.joinToString("\n") { row -> row.cells.joinToString("") { it.text } } ?: ""
    // Android may add a group summary asynchronously; it is not another event.
    private fun postedContent() = manager.activeNotifications.filter {
        it.notification.flags and Notification.FLAG_GROUP_SUMMARY == 0
    }
    private fun main(action: () -> Unit) = instrumentation.runOnMainSync(action)
    private suspend fun await(predicate: suspend () -> Boolean) {
        try { withTimeout(25_000) { while (!predicate()) delay(50) } }
        catch (error: TimeoutCancellationException) { throw AssertionError("Timed out during $phase", error) }
    }
}
