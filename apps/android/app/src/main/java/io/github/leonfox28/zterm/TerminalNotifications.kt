package io.github.leonfox28.zterm

import android.Manifest
import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import android.content.res.Configuration
import android.os.Build
import android.os.LocaleList
import android.provider.Settings
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.*
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.unit.dp
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.LifecycleEventObserver
import androidx.lifecycle.compose.LocalLifecycleOwner
import io.github.leonfox28.zterm.nativebridge.NativeNotification
import java.util.Locale
import java.util.UUID

/** Application-owned platform presentation. No notification is retained for a later grant. */
internal class TerminalNotifications(context: Context) {
    private val context = context.applicationContext
    private val manager = this.context.getSystemService(NotificationManager::class.java)
    private val tag = "terminal:${UUID.randomUUID()}"
    private var nextId = 0

    init { ensureChannel(this.context) }

    fun ensureChannel(resources: Context) {
        // Re-registering an existing channel updates its name, not the user's policy.
        manager.createNotificationChannel(NotificationChannel(CHANNEL, resources.getString(R.string.terminal_notifications),
            NotificationManager.IMPORTANCE_DEFAULT).apply {
            description = resources.getString(R.string.terminal_notifications_description)
        })
    }

    fun needsPermission(): Boolean = Build.VERSION.SDK_INT >= 33 &&
        context.checkSelfPermission(Manifest.permission.POST_NOTIFICATIONS) != PackageManager.PERMISSION_GRANTED

    fun enabled(): Boolean = !needsPermission() && manager.areNotificationsEnabled() &&
        manager.getNotificationChannel(CHANNEL)?.importance != NotificationManager.IMPORTANCE_NONE

    fun settingsIntent(): Intent = Intent(Settings.ACTION_CHANNEL_NOTIFICATION_SETTINGS)
        .putExtra(Settings.EXTRA_APP_PACKAGE, context.packageName)
        .putExtra(Settings.EXTRA_CHANNEL_ID, CHANNEL)

    /** Called on Main immediately after checking both native and repository epochs. */
    fun post(event: NativeNotification, language: String): Boolean {
        return try {
            if (!enabled()) return false
            val localized = if (language == "system") context else context.createConfigurationContext(
                Configuration(context.resources.configuration).apply { setLocales(LocaleList(Locale.forLanguageTag(language))) })
            ensureChannel(localized)
            val open = Intent(context, MainActivity::class.java)
                .addFlags(Intent.FLAG_ACTIVITY_CLEAR_TOP or Intent.FLAG_ACTIVITY_SINGLE_TOP)
            val contentIntent = PendingIntent.getActivity(context, 0, open,
                PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE)
            val title = event.title?.takeIf { it.isNotEmpty() } ?: localized.getString(R.string.app_name)
            val notification = Notification.Builder(context, CHANNEL)
                .setSmallIcon(R.drawable.ic_notification)
                .setContentTitle(title)
                .setContentText(event.body)
                .setStyle(Notification.BigTextStyle().bigText(event.body))
                .setVisibility(Notification.VISIBILITY_PRIVATE)
                .setContentIntent(contentIntent)
                .setAutoCancel(true)
                .build()
            nextId = if (nextId == Int.MAX_VALUE) 1 else nextId + 1
            manager.notify(tag, nextId, notification)
            true
        } catch (error: RuntimeException) {
            // Permission revocation or a platform posting failure must not kill the collector.
            if (BuildConfig.DEBUG) android.util.Log.d("ZtermState", "notification_skipped type=${error.javaClass.simpleName}")
            false
        }
    }

    companion object { const val CHANNEL = "terminal_notifications" }
}

@Composable internal fun TerminalNotificationSettings(notifications: TerminalNotifications) {
    val context = LocalContext.current
    val lifecycle = LocalLifecycleOwner.current.lifecycle
    var enabled by remember { mutableStateOf(notifications.enabled()) }
    var permissionRequired by remember { mutableStateOf(notifications.needsPermission()) }
    val permission = rememberLauncherForActivityResult(ActivityResultContracts.RequestPermission()) {
        enabled = notifications.enabled()
        permissionRequired = notifications.needsPermission()
    }
    DisposableEffect(context, lifecycle, notifications) {
        notifications.ensureChannel(context)
        enabled = notifications.enabled()
        permissionRequired = notifications.needsPermission()
        val observer = LifecycleEventObserver { _, event ->
            if (event == Lifecycle.Event.ON_RESUME) {
                enabled = notifications.enabled()
                permissionRequired = notifications.needsPermission()
            }
        }
        lifecycle.addObserver(observer)
        onDispose { lifecycle.removeObserver(observer) }
    }
    Column(Modifier.fillMaxWidth().padding(horizontal = 12.dp, vertical = 16.dp)) {
        Text(stringResource(R.string.terminal_notifications), style = MaterialTheme.typography.titleSmall)
        Text(stringResource(if (enabled) R.string.terminal_notifications_enabled else R.string.terminal_notifications_disabled),
            color = MaterialTheme.colorScheme.onSurfaceVariant, modifier = Modifier.padding(top = 8.dp))
        if (permissionRequired) {
            TextButton({ permission.launch(Manifest.permission.POST_NOTIFICATIONS) }) {
                Text(stringResource(R.string.enable_notifications))
            }
        }
        TextButton({ context.startActivity(notifications.settingsIntent()) }) {
            Text(stringResource(R.string.notification_system_settings))
        }
    }
}
