package io.github.leonfox28.zterm

import android.Manifest
import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.app.Service
import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import android.content.pm.ServiceInfo
import android.content.res.Configuration
import android.os.Build
import android.os.IBinder
import android.os.LocaleList
import android.provider.Settings
import kotlinx.coroutines.*
import kotlinx.coroutines.flow.*
import java.util.Locale

/** Only Application state decides whether a service is useful. Notification permission is independent. */
internal data class BackgroundConnectionState(
    val wanted: Boolean = false,
    val phase: String = "connecting",
    val language: String = "system",
)

internal fun backgroundConnectionWanted(enabled: Boolean, terminalRoute: Boolean, connecting: Boolean, phase: String?): Boolean =
    enabled && terminalRoute && (connecting || phase in setOf("connecting", "synchronizing", "active", "reconnecting"))

/** Starts only while an Activity is visible. Never retries a failed/system-stopped service in the background. */
internal class BackgroundConnection(context: Context) {
    private val context = context.applicationContext
    private val manager = this.context.getSystemService(NotificationManager::class.java)
    private val mutable = MutableStateFlow(BackgroundConnectionState())
    val state = mutable.asStateFlow()
    private val mutableFailure = MutableStateFlow(false)
    val failed = mutableFailure.asStateFlow()
    private var requested = false
    private var visible = false
    private var attempted = false

    fun update(next: BackgroundConnectionState, appVisible: Boolean) {
        if (appVisible && !visible || !next.wanted) attempted = false
        visible = appVisible
        mutable.value = next
        if (!next.wanted) {
            mutableFailure.value = false
            if (requested) {
                requested = false
                context.stopService(Intent(context, TerminalConnectionService::class.java))
            }
        } else if (appVisible && !requested && !attempted) {
            attempted = true
            try {
                ensureChannel(localized(next.language))
                context.startForegroundService(Intent(context, TerminalConnectionService::class.java))
                requested = true
                mutableFailure.value = false
            } catch (_: RuntimeException) { startFailed() }
        }
    }

    fun startFailed() { requested = false; mutableFailure.value = true }
    fun stopped() { requested = false }
    fun visibleNotification(): Boolean =
        (Build.VERSION.SDK_INT < 33 || context.checkSelfPermission(Manifest.permission.POST_NOTIFICATIONS) == PackageManager.PERMISSION_GRANTED) &&
            manager.areNotificationsEnabled() && manager.getNotificationChannel(CHANNEL)?.importance != NotificationManager.IMPORTANCE_NONE

    fun settingsIntent(): Intent = Intent(Settings.ACTION_CHANNEL_NOTIFICATION_SETTINGS)
        .putExtra(Settings.EXTRA_APP_PACKAGE, context.packageName).putExtra(Settings.EXTRA_CHANNEL_ID, CHANNEL)

    fun ensureChannel(resources: Context = context) {
        manager.createNotificationChannel(NotificationChannel(CHANNEL, resources.getString(R.string.background_connection),
            NotificationManager.IMPORTANCE_LOW).apply { description = resources.getString(R.string.background_connection_description) })
    }

    fun notification(snapshot: BackgroundConnectionState): Notification {
        val resources = localized(snapshot.language)
        ensureChannel(resources)
        val open = PendingIntent.getActivity(context, 100, Intent(context, MainActivity::class.java)
            .addFlags(Intent.FLAG_ACTIVITY_CLEAR_TOP or Intent.FLAG_ACTIVITY_SINGLE_TOP),
            PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE)
        val disconnect = PendingIntent.getService(context, 101,
            Intent(context, TerminalConnectionService::class.java).setAction(DISCONNECT),
            PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE)
        val status = resources.getString(when (snapshot.phase) {
            "active" -> R.string.background_connected
            "reconnecting" -> R.string.reconnecting
            else -> R.string.connecting
        })
        return Notification.Builder(context, CHANNEL).setSmallIcon(R.drawable.ic_notification)
            .setContentTitle(resources.getString(R.string.app_name)).setContentText(status)
            .setCategory(Notification.CATEGORY_SERVICE).setVisibility(Notification.VISIBILITY_PRIVATE)
            .setOngoing(true).setOnlyAlertOnce(true).setContentIntent(open)
            .addAction(Notification.Action.Builder(null, resources.getString(R.string.return_to_terminal), open).build())
            .addAction(Notification.Action.Builder(null, resources.getString(R.string.disconnect), disconnect).build())
            .build()
    }

    private fun localized(language: String): Context = if (language == "system") context else context.createConfigurationContext(
        Configuration(context.resources.configuration).apply { setLocales(LocaleList(Locale.forLanguageTag(language))) })

    companion object {
        const val CHANNEL = "background_connection"
        const val NOTIFICATION_ID = 1001
        const val DISCONNECT = "io.github.leonfox28.zterm.DISCONNECT_TERMINAL"
    }
}

/** A lifetime/notification adapter only: the Application continues to own Iroh and reconnection. */
class TerminalConnectionService : Service() {
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.Main.immediate)
    private val repository get() = (application as ZtermApplication).repository
    override fun onBind(intent: Intent?): IBinder? = null

    override fun onCreate() {
        super.onCreate()
        val owner = repository.backgroundConnection
        try {
            // Promote immediately, including when notification permission is denied.
            val notification = owner.notification(owner.state.value)
            if (Build.VERSION.SDK_INT >= 29) startForeground(BackgroundConnection.NOTIFICATION_ID, notification,
                ServiceInfo.FOREGROUND_SERVICE_TYPE_CONNECTED_DEVICE)
            else startForeground(BackgroundConnection.NOTIFICATION_ID, notification)
        } catch (_: RuntimeException) {
            owner.startFailed(); stopSelf(); return
        }
        scope.launch {
            owner.state.collect { snapshot ->
                if (!snapshot.wanted) {
                    stopForeground(STOP_FOREGROUND_REMOVE); stopSelf()
                } else {
                    try { getSystemService(NotificationManager::class.java).notify(BackgroundConnection.NOTIFICATION_ID, owner.notification(snapshot)) }
                    catch (_: RuntimeException) { /* Posting denial does not mean foreground promotion failed. */ }
                }
            }
        }
    }

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        if (intent?.action == BackgroundConnection.DISCONNECT) {
            repository.goHome()
            stopForeground(STOP_FOREGROUND_REMOVE)
            stopSelf()
        } else if (!repository.backgroundConnection.state.value.wanted) {
            stopForeground(STOP_FOREGROUND_REMOVE); stopSelf()
        }
        return START_NOT_STICKY
    }

    override fun onDestroy() {
        scope.cancel()
        repository.backgroundConnection.stopped()
        super.onDestroy()
    }
}
