package io.github.leonfox28.zterm

import android.content.ClipData
import android.content.Intent
import android.provider.Settings
import android.widget.Toast
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.compose.ui.window.Dialog
import androidx.compose.ui.window.DialogProperties
import androidx.core.content.FileProvider
import androidx.core.net.toUri
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.LifecycleEventObserver
import androidx.lifecycle.compose.LocalLifecycleOwner
import androidx.lifecycle.compose.collectAsStateWithLifecycle

internal val LocalUpdateBlocks = staticCompositionLocalOf<MutableIntState?> { null }

/** Existing modal owners can defer an unsolicited startup prompt. */
@Composable internal fun BlockStartupUpdatePrompt() {
    val blocks = LocalUpdateBlocks.current
    DisposableEffect(blocks) {
        if (blocks != null) blocks.intValue++
        onDispose { if (blocks != null) blocks.intValue-- }
    }
}

@Composable internal fun WideStatusCard(
    title: String, body: String, modifier: Modifier = Modifier, icon: String = "download",
    busy: Boolean = false, minimumHeight: Dp = 284.dp,
    actions: @Composable ColumnScope.() -> Unit,
) {
    Surface(modifier.widthIn(max = 560.dp).fillMaxWidth(),
        shape = RoundedCornerShape(28.dp), color = MaterialTheme.colorScheme.surfaceContainer) {
        Column(Modifier.verticalScroll(rememberScrollState()).heightIn(min = minimumHeight).padding(24.dp),
            verticalArrangement = Arrangement.spacedBy(16.dp)) {
            Surface(shape = RoundedCornerShape(16.dp), color = MaterialTheme.colorScheme.surfaceContainerHigh) {
                Box(Modifier.size(48.dp), contentAlignment = Alignment.Center) {
                    if (busy) CircularProgressIndicator(Modifier.size(24.dp), strokeWidth = 2.dp)
                    else LineIcon(icon, color = MaterialTheme.colorScheme.primary)
                }
            }
            Text(title, fontSize = 22.sp, fontWeight = FontWeight.SemiBold)
            Text(body, fontSize = 14.sp, color = MaterialTheme.colorScheme.onSurfaceVariant)
            actions()
        }
    }
}

@Composable private fun UpdateDialog(title: String, body: String, dismiss: () -> Unit,
    actions: @Composable ColumnScope.() -> Unit) {
    Dialog(onDismissRequest = dismiss, properties = DialogProperties(usePlatformDefaultWidth = false)) {
        WideStatusCard(title, body, Modifier.padding(24.dp), minimumHeight = 324.dp, actions = actions)
    }
}

@Composable internal fun AppUpdateHost(updates: AppUpdates, appState: AppState) {
    val state by updates.state.collectAsStateWithLifecycle()
    val context = LocalContext.current
    val lifecycle = LocalLifecycleOwner.current.lifecycle
    var resumed by remember(lifecycle) { mutableStateOf(lifecycle.currentState.isAtLeast(Lifecycle.State.RESUMED)) }
    val blocks = LocalUpdateBlocks.current?.intValue ?: 0
    val automaticAllowed = appState.route in setOf(Route.Home, Route.Settings) &&
        !appState.busy && appState.error == null && blocks == 0
    DisposableEffect(lifecycle) {
        val observer = LifecycleEventObserver { _, _ -> resumed = lifecycle.currentState.isAtLeast(Lifecycle.State.RESUMED) }
        lifecycle.addObserver(observer)
        onDispose { lifecycle.removeObserver(observer) }
    }
    LaunchedEffect(appState.initialized, resumed) {
        if (appState.initialized && resumed) {
            withFrameNanos { }
            updates.startup()
        }
    }
    val message = state.notice?.let { stringResource(updateMessage(it.code)) }
    LaunchedEffect(state.notice?.id, resumed) {
        val notice = state.notice
        if (resumed && notice != null && updates.consumeNotice(notice.id))
            Toast.makeText(context, message, Toast.LENGTH_SHORT).show()
    }
    val install = rememberLauncherForActivityResult(ActivityResultContracts.StartActivityForResult()) {
        updates.installationReturned()
    }
    val permission = rememberLauncherForActivityResult(ActivityResultContracts.StartActivityForResult()) {
        updates.permissionReturned(context.packageManager.canRequestPackageInstalls())
    }
    LaunchedEffect(state.phase, resumed, blocks) {
        if (state.phase == UpdatePhase.Ready && resumed && blocks == 0) {
            if (!context.packageManager.canRequestPackageInstalls()) updates.requireInstallPermission()
            else {
                val file = updates.takeInstallFile() ?: return@LaunchedEffect
                try {
                    val uri = FileProvider.getUriForFile(context, context.packageName + ".updates", file)
                    install.launch(Intent(Intent.ACTION_VIEW).setDataAndType(uri, "application/vnd.android.package-archive")
                        .addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION).apply { clipData = ClipData.newRawUri("zterm update", uri) })
                } catch (_: RuntimeException) { updates.fail("update_install_failed") }
            }
        }
    }
    if (!resumed) return
    when (state.phase) {
        UpdatePhase.Available -> if (blocks == 0 && (state.manual || automaticAllowed)) {
            UpdateDialog(stringResource(R.string.update_available),
                stringResource(R.string.update_confirm_body, BuildConfig.VERSION_NAME, state.details!!.version), updates::dismiss) {
                Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.spacedBy(12.dp)) {
                    FilledTonalButton(updates::dismiss, Modifier.weight(1f).heightIn(min = 48.dp)) { Text(stringResource(R.string.update_later)) }
                    Button(updates::download, Modifier.weight(1.35f).heightIn(min = 48.dp)) { Text(stringResource(R.string.update_download_install)) }
                }
            }
        }
        UpdatePhase.Downloading -> UpdateDialog(stringResource(R.string.update_downloading),
            stringResource(R.string.update_downloading_body, state.details!!.version), updates::cancel) {
            val progress = (state.downloaded.toFloat() / state.details!!.length).coerceIn(0f, 1f)
            LinearProgressIndicator(progress = { progress }, modifier = Modifier.fillMaxWidth())
            Text(stringResource(R.string.update_percent, (progress * 100).toInt()), fontSize = 13.sp)
            FilledTonalButton(updates::cancel, Modifier.fillMaxWidth().heightIn(min = 48.dp)) { Text(stringResource(R.string.update_cancel_download)) }
        }
        UpdatePhase.Permission -> UpdateDialog(stringResource(R.string.update_install_permission),
            stringResource(R.string.update_install_permission_body), updates::cancel) {
            Button(onClick = {
                try { permission.launch(Intent(Settings.ACTION_MANAGE_UNKNOWN_APP_SOURCES, ("package:" + context.packageName).toUri())) }
                catch (_: RuntimeException) { updates.fail("update_install_failed") }
            }, modifier = Modifier.fillMaxWidth().heightIn(min = 48.dp)) { Text(stringResource(R.string.open_system_settings)) }
            TextButton(updates::cancel, Modifier.fillMaxWidth()) { Text(stringResource(R.string.cancel)) }
        }
        else -> Unit
    }
}

internal fun updateMessage(code: String): Int = when (code) {
    "update_latest" -> R.string.update_latest
    "update_invalid" -> R.string.update_invalid
    "update_unsupported" -> R.string.update_unsupported
    "update_rate_limit" -> R.string.update_rate_limit
    "update_storage" -> R.string.update_storage
    "update_install_failed" -> R.string.update_install_failed
    "update_download_failed" -> R.string.update_download_failed
    else -> R.string.update_network
}

@Composable internal fun AboutSettings(updates: AppUpdates, enabled: Boolean) {
    val state by updates.state.collectAsStateWithLifecycle()
    val context = LocalContext.current
    Text(stringResource(R.string.about), Modifier.padding(start = 12.dp, top = 20.dp, bottom = 8.dp),
        fontSize = 13.sp, color = MaterialTheme.colorScheme.onSurfaceVariant)
    Surface(shape = RoundedCornerShape(16.dp), color = MaterialTheme.colorScheme.surfaceContainer) {
        Column {
            Surface(onClick = {
                try { context.startActivity(Intent(Intent.ACTION_VIEW, "https://github.com/leonfox28/zterm".toUri())) }
                catch (_: android.content.ActivityNotFoundException) {
                    Toast.makeText(context, R.string.browser_unavailable, Toast.LENGTH_SHORT).show()
                }
            }, color = MaterialTheme.colorScheme.surfaceContainer) {
                Row(Modifier.fillMaxWidth().heightIn(min = 72.dp).padding(16.dp), verticalAlignment = Alignment.CenterVertically) {
                    Icon(androidx.compose.ui.res.painterResource(R.drawable.ic_github), null, Modifier.size(24.dp))
                    Column(Modifier.weight(1f).padding(horizontal = 16.dp)) {
                        Text("GitHub", fontSize = 15.sp)
                        Text("leonfox28/zterm", fontSize = 12.sp, color = MaterialTheme.colorScheme.onSurfaceVariant)
                    }
                    LineIcon("external", Modifier.size(18.dp), MaterialTheme.colorScheme.onSurfaceVariant)
                }
            }
            HorizontalDivider(Modifier.padding(start = 56.dp, end = 16.dp), color = MaterialTheme.colorScheme.outlineVariant)
            val checking = state.phase == UpdatePhase.Checking && state.manual
            Surface(onClick = { updates.check() }, enabled = enabled && state.phase in setOf(UpdatePhase.Idle, UpdatePhase.Checking) && !checking,
                color = MaterialTheme.colorScheme.surfaceContainer) {
                Row(Modifier.fillMaxWidth().heightIn(min = 72.dp).padding(16.dp), verticalAlignment = Alignment.CenterVertically) {
                    LineIcon("download")
                    Column(Modifier.weight(1f).padding(start = 16.dp, end = 8.dp)) {
                        Text(stringResource(if (checking) R.string.update_checking else R.string.check_updates), fontSize = 15.sp)
                        Text(stringResource(R.string.installed_version, BuildConfig.VERSION_NAME), fontSize = 12.sp, color = MaterialTheme.colorScheme.onSurfaceVariant)
                    }
                    if (checking) CircularProgressIndicator(Modifier.size(18.dp), strokeWidth = 2.dp)
                }
            }
        }
    }
    Spacer(Modifier.height(24.dp))
}
