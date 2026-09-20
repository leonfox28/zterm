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
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.selection.toggleable
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.Surface
import androidx.compose.material3.Switch
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.ui.Alignment
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.unit.sp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
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
import kotlinx.coroutines.flow.asStateFlow

/** Application-owned platform presentation. No notification is retained for a later grant. */
internal class TerminalNotifications(context: Context) {
    private val context = context.applicationContext
    private val manager = this.context.getSystemService(NotificationManager::class.java)
    private val tag = "terminal:${UUID.randomUUID()}"
    private var nextId = 0
    private val appGate = kotlinx.coroutines.flow.MutableStateFlow(true)
    val appEnabled = appGate.asStateFlow()

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

    /** Serialize application preference changes with posting. */
    @Synchronized fun setAppEnabled(enabled: Boolean) { appGate.value = enabled }

    /** Called on Main immediately after checking both native and repository epochs. */
    @Synchronized fun post(event: NativeNotification, language: String): Boolean {
        return try {
            if (!appGate.value || !enabled()) return false
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

@Composable internal fun TerminalNotificationSettings(state: AppState, repository: AppRepository) {
    val notifications = repository.notifications
    val context = LocalContext.current
    val activity = androidx.activity.compose.LocalActivity.current
    val lifecycle = LocalLifecycleOwner.current.lifecycle
    val appEnabled by notifications.appEnabled.collectAsStateWithLifecycle()
    var systemEnabled by remember { mutableStateOf(notifications.enabled()) }
    var pending by rememberSaveable { mutableStateOf(false) }
    var guidance by rememberSaveable { mutableStateOf(false) }
    if (pending) BlockStartupUpdatePrompt()
    fun returned() {
        systemEnabled = notifications.enabled()
        if (pending && systemEnabled) repository.setNotificationsEnabled(true)
        pending = false
    }
    val permission = rememberLauncherForActivityResult(ActivityResultContracts.RequestPermission()) { returned() }
    val settings = rememberLauncherForActivityResult(ActivityResultContracts.StartActivityForResult()) { returned() }
    DisposableEffect(context, lifecycle, notifications) {
        notifications.ensureChannel(context)
        systemEnabled = notifications.enabled()
        val observer = LifecycleEventObserver { _, event ->
            if (event == Lifecycle.Event.ON_RESUME) systemEnabled = notifications.enabled()
        }
        lifecycle.addObserver(observer)
        onDispose { lifecycle.removeObserver(observer) }
    }
    val change: (Boolean) -> Unit = { value ->
        when {
            !value -> { pending = false; repository.setNotificationsEnabled(false) }
            notifications.enabled() -> repository.setNotificationsEnabled(true)
            notifications.needsPermission() && (!state.saved.preferences.notificationPermissionRequested ||
                activity?.shouldShowRequestPermissionRationale(Manifest.permission.POST_NOTIFICATIONS) == true) -> {
                pending = true
                repository.updatePreferences { it.copy(notificationPermissionRequested = true) }
                permission.launch(Manifest.permission.POST_NOTIFICATIONS)
            }
            else -> guidance = true
        }
    }
    Text(stringResource(R.string.notifications), Modifier.padding(start = 12.dp, top = 20.dp, bottom = 8.dp),
        fontSize = 13.sp, color = MaterialTheme.colorScheme.onSurfaceVariant)
    val effective = appEnabled && systemEnabled
    Surface(shape = RoundedCornerShape(16.dp), color = MaterialTheme.colorScheme.surfaceContainer) {
        Row(Modifier.fillMaxWidth().heightIn(min = 74.dp)
            .toggleable(effective, enabled = state.initialized && !state.notificationsSaving && !pending,
                role = Role.Switch, onValueChange = change)
            .padding(horizontal = 16.dp, vertical = 12.dp), verticalAlignment = Alignment.CenterVertically) {
            Column(Modifier.weight(1f).padding(end = 12.dp)) {
                Text(stringResource(R.string.terminal_notifications), fontSize = 15.sp)
                Text(stringResource(when {
                    !systemEnabled -> R.string.notifications_system_blocked
                    appEnabled -> R.string.notifications_on
                    else -> R.string.notifications_off
                }), fontSize = 12.sp, color = MaterialTheme.colorScheme.onSurfaceVariant)
            }
            Switch(checked = effective, onCheckedChange = null,
                enabled = state.initialized && !state.notificationsSaving && !pending)
        }
    }
    if (guidance) ConfirmDialog(stringResource(R.string.enable_notifications),
        stringResource(R.string.notifications_permission_guidance), stringResource(R.string.notification_system_settings),
        dismiss = { guidance = false }, confirm = {
            guidance = false
            try { pending = true; settings.launch(notifications.settingsIntent()) }
            catch (_: android.content.ActivityNotFoundException) {
                pending = false
                android.widget.Toast.makeText(context, R.string.system_settings_unavailable, android.widget.Toast.LENGTH_SHORT).show()
            }
        })
}
