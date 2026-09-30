package io.github.leonfox28.zterm

import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.selection.toggleable
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.LifecycleEventObserver
import androidx.lifecycle.compose.LocalLifecycleOwner
import androidx.lifecycle.compose.collectAsStateWithLifecycle

@Composable internal fun BackgroundConnectionSettings(state: AppState, repository: AppRepository) {
    val owner = repository.backgroundConnection
    val context = LocalContext.current
    val lifecycle = LocalLifecycleOwner.current.lifecycle
    val failed by owner.failed.collectAsStateWithLifecycle()
    var visible by remember { mutableStateOf(owner.visibleNotification()) }
    val settings = rememberLauncherForActivityResult(ActivityResultContracts.StartActivityForResult()) { visible = owner.visibleNotification() }
    DisposableEffect(lifecycle, owner) {
        owner.ensureChannel(context)
        visible = owner.visibleNotification()
        val observer = LifecycleEventObserver { _, event ->
            if (event == Lifecycle.Event.ON_RESUME) visible = owner.visibleNotification()
        }
        lifecycle.addObserver(observer)
        onDispose { lifecycle.removeObserver(observer) }
    }
    Text(stringResource(R.string.background_connection), Modifier.padding(start = 12.dp, top = 20.dp, bottom = 8.dp),
        fontSize = 13.sp, color = MaterialTheme.colorScheme.onSurfaceVariant)
    Surface(shape = RoundedCornerShape(16.dp), color = MaterialTheme.colorScheme.surfaceContainer) {
        Column {
            Row(Modifier.fillMaxWidth().testTag("background-connection").toggleable(
                state.saved.preferences.keepBackgroundConnection, enabled = state.initialized, role = Role.Switch,
                onValueChange = { value -> repository.updatePreferences { it.copy(keepBackgroundConnection = value) } }
            ).padding(16.dp), verticalAlignment = Alignment.CenterVertically) {
                Column(Modifier.weight(1f).padding(end = 12.dp)) {
                    Text(stringResource(R.string.keep_background_connection), fontSize = 15.sp)
                    Text(stringResource(R.string.background_connection_description), fontSize = 12.sp,
                        color = MaterialTheme.colorScheme.onSurfaceVariant)
                }
                Switch(state.saved.preferences.keepBackgroundConnection, onCheckedChange = null, enabled = state.initialized)
            }
            Text(stringResource(if (visible) R.string.background_notification_visible else R.string.background_notification_hidden),
                Modifier.padding(horizontal = 16.dp), fontSize = 12.sp, color = MaterialTheme.colorScheme.onSurfaceVariant)
            if (failed) Text(stringResource(R.string.background_start_failed), Modifier.padding(16.dp), color = MaterialTheme.colorScheme.error)
            TextButton(onClick = {
                try { settings.launch(owner.settingsIntent()) }
                catch (_: android.content.ActivityNotFoundException) {
                    android.widget.Toast.makeText(context, R.string.system_settings_unavailable, android.widget.Toast.LENGTH_SHORT).show()
                }
            }, modifier = Modifier.padding(horizontal = 8.dp)) { Text(stringResource(R.string.notification_system_settings)) }
        }
    }
}
