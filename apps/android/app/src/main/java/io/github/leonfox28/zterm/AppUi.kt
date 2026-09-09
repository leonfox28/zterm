package io.github.leonfox28.zterm

import android.content.res.Configuration
import android.os.LocaleList
import androidx.activity.compose.BackHandler
import androidx.compose.foundation.selection.selectable
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.foundation.Canvas
import androidx.compose.foundation.combinedClickable
import androidx.compose.foundation.isSystemInDarkTheme
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.Path
import androidx.compose.ui.graphics.drawscope.Stroke
import androidx.compose.ui.platform.LocalConfiguration
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import io.github.leonfox28.zterm.nativebridge.NativeSession
import kotlin.math.roundToInt
import java.util.Locale

@Composable internal fun ZtermApp(repository: AppRepository) {
    val state by repository.state.collectAsStateWithLifecycle()
    val system = LocalConfiguration.current
    val context = LocalContext.current
    val language = state.saved.preferences.language
    val localized = remember(context, system, language) {
        val configuration = Configuration(system)
        if (language != "system") configuration.setLocales(LocaleList(Locale.forLanguageTag(language)))
        // Preserve the Activity in the wrapper chain for launchers, dialogs and
        // CameraX while giving resources an independent language configuration.
        android.view.ContextThemeWrapper(context, context.theme).apply { applyOverrideConfiguration(configuration) }
    }
    val dark = when (state.saved.preferences.theme) { "dark" -> true; "light" -> false; else -> isSystemInDarkTheme() }
    LaunchedEffect(dark) {
        repository.setDark(dark)
        (context as? android.app.Activity)?.window?.let { window ->
            androidx.core.view.WindowCompat.getInsetsController(window, window.decorView).apply {
                isAppearanceLightStatusBars = !dark
                isAppearanceLightNavigationBars = !dark
            }
        }
    }
    val colors = if (dark) darkColorScheme(primary = Color(0xFFD9FBA7), onPrimary = Color(0xFF101713),
        background = Color(0xFF101713), surface = Color(0xFF101713), surfaceContainer = Color(0xFF1A241D),
        surfaceContainerHigh = Color(0xFF223027), onSurface = Color(0xFFF0F4ED), onSurfaceVariant = Color(0xFFACB9AF),
        outline = Color(0xFF7A8F80), outlineVariant = Color(0xFF314036), error = Color(0xFFFFB4AB))
    else lightColorScheme(primary = Color(0xFF476A20), onPrimary = Color.White,
        background = Color(0xFFF5F8F3), surface = Color(0xFFF5F8F3), surfaceContainer = Color.White,
        surfaceContainerHigh = Color(0xFFE6EEE3), onSurface = Color(0xFF182219), onSurfaceVariant = Color(0xFF53634F),
        outline = Color(0xFF71816B), outlineVariant = Color(0xFFD4DED2))
    CompositionLocalProvider(LocalContext provides localized, LocalConfiguration provides localized.resources.configuration) {
        MaterialTheme(colorScheme = colors) {
            Surface(Modifier.fillMaxSize()) {
                when (state.route) {
                    Route.Home -> HomeScreen(state, repository)
                    Route.Settings -> SettingsScreen(state, repository)
                    Route.Scanner -> ScannerScreen(state, repository)
                    Route.Terminal -> TerminalScreen(state, repository)
                }
            }
        }
    }
}

@Composable internal fun LineIcon(name: String, modifier: Modifier = Modifier.size(24.dp), color: Color = LocalContentColor.current) {
    Canvas(modifier) {
        val factor = size.width / 24
        val stroke = 1.7f * factor
        fun line(x: Float, y: Float, a: Float, b: Float) = drawLine(color, Offset(x * factor, y * factor), Offset(a * factor, b * factor), stroke)
        fun circle(x: Float, y: Float, radius: Float) = drawCircle(color, radius * factor, Offset(x * factor, y * factor), style = Stroke(stroke))
        when (name) {
            "back" -> { line(15f,5f,8f,12f); line(8f,12f,15f,19f) }
            "right" -> { line(9f,5f,16f,12f); line(16f,12f,9f,19f) }
            "close" -> { line(6f,6f,18f,18f); line(18f,6f,6f,18f) }
            "down" -> { line(6f,9f,12f,15f); line(12f,15f,18f,9f) }
            "plus" -> { line(12f,5f,12f,19f); line(5f,12f,19f,12f) }
            "arrow-left" -> { line(21f,12f,3f,12f); line(3f,12f,10f,5f); line(3f,12f,10f,19f) }
            "arrow-down" -> { line(12f,3f,12f,21f); line(12f,21f,5f,14f); line(12f,21f,19f,14f) }
            "arrow-up" -> { line(12f,21f,12f,3f); line(12f,3f,5f,10f); line(12f,3f,19f,10f) }
            "arrow-right" -> { line(3f,12f,21f,12f); line(21f,12f,14f,5f); line(21f,12f,14f,19f) }
            "more" -> for (y in listOf(5f,12f,19f)) drawCircle(color, 1.5f*factor, Offset(12f*factor,y*factor))
            "terminal" -> { line(4f,6f,10f,12f); line(10f,12f,4f,18f); line(13f,18f,21f,18f) }
            "attachment" -> {
                val path = Path().apply {
                    moveTo(21f*factor, 11f*factor)
                    lineTo(12f*factor, 20f*factor)
                    cubicTo(9.8f*factor,22.2f*factor,6.2f*factor,22.2f*factor,4f*factor,20f*factor)
                    cubicTo(1.8f*factor,17.8f*factor,1.8f*factor,14.2f*factor,4f*factor,12f*factor)
                    lineTo(13f*factor, 3f*factor)
                    cubicTo(14.4f*factor,1.6f*factor,16.6f*factor,1.6f*factor,18f*factor,3f*factor)
                    cubicTo(19.4f*factor,4.4f*factor,19.4f*factor,6.6f*factor,18f*factor,8f*factor)
                    lineTo(9f*factor,17f*factor)
                    cubicTo(8.3f*factor,17.7f*factor,7.2f*factor,17.7f*factor,6.5f*factor,17f*factor)
                    cubicTo(5.8f*factor,16.3f*factor,5.8f*factor,15.2f*factor,6.5f*factor,14.5f*factor)
                    lineTo(15f*factor,6f*factor)
                }
                drawPath(path,color,style=Stroke(stroke, cap=androidx.compose.ui.graphics.StrokeCap.Round, join=androidx.compose.ui.graphics.StrokeJoin.Round))
            }
            "keyboard" -> {
                drawRoundRect(color, Offset(2f*factor,5f*factor), androidx.compose.ui.geometry.Size(20f*factor,14f*factor), androidx.compose.ui.geometry.CornerRadius(2f*factor), style=Stroke(stroke))
                for (y in listOf(9f,12f)) for (x in listOf(6f,10f,14f,18f)) line(x,y,x+.5f,y)
                line(7f,16f,17f,16f)
            }
            "settings" -> {
                val p = Path()
                for (i in 0..32) {
                    val angle = i * Math.PI / 16
                    val radius = if (i % 4 in 1..2) 10f else 8f
                    val x = (12 + kotlin.math.cos(angle)*radius).toFloat()*factor
                    val y = (12 + kotlin.math.sin(angle)*radius).toFloat()*factor
                    if (i == 0) p.moveTo(x,y) else p.lineTo(x,y)
                }; p.close(); drawPath(p,color,style=Stroke(stroke)); circle(12f,12f,3f)
            }
            "image" -> { drawRoundRect(color, Offset(3f*factor,3f*factor), androidx.compose.ui.geometry.Size(18f*factor,18f*factor), androidx.compose.ui.geometry.CornerRadius(2f*factor), style=Stroke(stroke)); circle(8f,8f,1.5f); line(4f,18f,10f,12f); line(10f,12f,14f,16f); line(14f,16f,18f,11f); line(18f,11f,21f,14f) }
            else -> {
                for (y in listOf(4f,14f)) {
                    drawRoundRect(color, Offset(3f*factor,y*factor), androidx.compose.ui.geometry.Size(18f*factor,6f*factor), androidx.compose.ui.geometry.CornerRadius(factor), style=Stroke(stroke))
                    line(6f,y+3,8f,y+3)
                }
            }
        }
    }
}
@Composable internal fun IconAction(icon: String, label: String, enabled: Boolean = true, onClick: () -> Unit) {
    IconButton(onClick, enabled = enabled) {
        Box(Modifier.semanticsDescription(label)) { LineIcon(icon) }
    }
}
private fun Modifier.semanticsDescription(label: String): Modifier = this.then(Modifier.semantics { contentDescription = label })

@Composable internal fun PageHeader(title: String, back: () -> Unit) {
    Row(Modifier.fillMaxWidth().height(56.dp), verticalAlignment = Alignment.CenterVertically) {
        IconAction("back", stringResource(R.string.back), onClick = back)
        Text(title, fontSize = 20.sp, fontWeight = FontWeight.SemiBold)
    }
}
@Composable private fun HomeScreen(state: AppState, repository: AppRepository) {
    var removing by remember { mutableStateOf<SavedHost?>(null) }
    Column(Modifier.fillMaxSize().safeDrawingPadding().padding(horizontal = 24.dp)) {
        Row(Modifier.fillMaxWidth().height(64.dp), verticalAlignment = Alignment.CenterVertically) {
            LineIcon("terminal", color = MaterialTheme.colorScheme.primary)
            Text("zterm", Modifier.padding(start = 14.dp).weight(1f), fontSize = 24.sp, fontWeight = FontWeight.SemiBold)
            IconAction("settings", stringResource(R.string.settings)) { repository.show(Route.Settings) }
        }
        LazyColumn(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(12.dp), contentPadding = PaddingValues(vertical = 16.dp)) {
            val recent = state.saved.recent
            val recentHost = state.saved.hosts.firstOrNull { it.id == recent?.host }
            if (recent != null && recentHost != null) item {
                Card(onClick = { repository.connectHost(recent.host, true) }, enabled = !state.busy,
                    colors = CardDefaults.cardColors(containerColor = MaterialTheme.colorScheme.surfaceContainer), shape = RoundedCornerShape(20.dp)) {
                    Column(Modifier.fillMaxWidth().padding(20.dp)) {
                        Text(stringResource(R.string.recent_connection), color = MaterialTheme.colorScheme.primary, fontSize = 12.sp)
                        Row(verticalAlignment = Alignment.CenterVertically) {
                            Text(recentHost.name, Modifier.weight(1f).padding(vertical = 16.dp), fontSize = 23.sp, fontWeight = FontWeight.SemiBold)
                            LineIcon("right", color = MaterialTheme.colorScheme.primary)
                        }
                    }
                }
                Spacer(Modifier.height(12.dp))
            }
            item { Text(stringResource(R.string.saved_hosts), color = MaterialTheme.colorScheme.onSurfaceVariant, fontSize = 13.sp) }
            items(state.saved.hosts, key = { it.id }) { host ->
                Surface(Modifier.fillMaxWidth().combinedClickable(enabled = !state.busy,
                    onClick = { repository.connectHost(host.id) }, onLongClick = { removing = host }),
                    shape = RoundedCornerShape(16.dp), color = MaterialTheme.colorScheme.surfaceContainer) {
                    Row(Modifier.padding(18.dp).heightIn(min = 42.dp), verticalAlignment = Alignment.CenterVertically) {
                        LineIcon("server", color = MaterialTheme.colorScheme.onSurfaceVariant)
                        Text(host.name, Modifier.padding(horizontal = 18.dp).weight(1f), fontWeight = FontWeight.Medium)
                        LineIcon("right", color = MaterialTheme.colorScheme.onSurfaceVariant)
                    }
                }
            }
        }
        ErrorText(state.error)
        if (state.busy || !state.initialized && state.error == null) LinearProgressIndicator(Modifier.fillMaxWidth())
        OutlinedButton({ repository.show(Route.Scanner) }, Modifier.fillMaxWidth().padding(vertical = 16.dp).height(50.dp), enabled = state.initialized && !state.busy,
            shape = RoundedCornerShape(12.dp)) { LineIcon("plus"); Spacer(Modifier.width(10.dp)); Text(stringResource(R.string.add_host)) }
    }
    removing?.let { host -> ConfirmDialog(stringResource(R.string.remove_host), host.name + "\n" + stringResource(R.string.remove_host_message),
        stringResource(R.string.remove), { removing = null }) { removing = null; repository.removeHost(host.id) } }
}
@Composable private fun SettingsScreen(state: AppState, repository: AppRepository) {
    BackHandler { repository.show(Route.Home) }
    val preferences = state.saved.preferences
    var fontSize by remember(preferences.fontSize) { mutableIntStateOf(preferences.fontSize) }
    Column(Modifier.fillMaxSize().safeDrawingPadding().verticalScroll(rememberScrollState()).padding(horizontal = 16.dp)) {
        PageHeader(stringResource(R.string.settings)) { repository.show(Route.Home) }
        PreferenceGroup(stringResource(R.string.language), preferences.language,
            listOf("system" to stringResource(R.string.follow_system), "zh" to stringResource(R.string.chinese), "en" to stringResource(R.string.english))) {
            val value = it
            repository.updatePreferences { current -> current.copy(language = value) }
        }
        PreferenceGroup(stringResource(R.string.theme), preferences.theme,
            listOf("system" to stringResource(R.string.follow_system), "dark" to stringResource(R.string.dark), "light" to stringResource(R.string.light))) {
            val value = it
            repository.updatePreferences { current -> current.copy(theme = value) }
        }
        val fontLabel = stringResource(R.string.font_size)
        Row(Modifier.fillMaxWidth().padding(start = 12.dp, end = 12.dp, top = 20.dp, bottom = 8.dp)) {
            Text(fontLabel, Modifier.weight(1f), fontSize = 13.sp, color = MaterialTheme.colorScheme.onSurfaceVariant)
            Text(fontSize.toString(), fontSize = 13.sp)
        }
        Slider(value = fontSize.toFloat(),
            onValueChange = { fontSize = it.roundToInt().coerceIn(terminalFontSizes) },
            onValueChangeFinished = {
                val value = fontSize
                repository.updatePreferences { current -> current.copy(fontSize = value) }
            },
            valueRange = terminalFontSizes.first.toFloat()..terminalFontSizes.last.toFloat(),
            steps = terminalFontSizes.last - terminalFontSizes.first - 1,
            modifier = Modifier.fillMaxWidth().padding(horizontal = 12.dp).semantics { contentDescription = fontLabel })
        Surface(Modifier.fillMaxWidth().padding(vertical = 12.dp), color = MaterialTheme.colorScheme.surfaceContainer, shape = RoundedCornerShape(14.dp)) {
            Text(stringResource(R.string.font_preview), Modifier.padding(18.dp), fontFamily = FontFamily.Monospace, fontSize = fontSize.sp)
        }
        Row(Modifier.fillMaxWidth().padding(16.dp)) { Text(stringResource(R.string.version), Modifier.weight(1f)); Text(BuildConfig.VERSION_NAME) }
        ErrorText(state.error)
    }
}
@Composable private fun PreferenceGroup(title: String, selected: String, options: List<Pair<String,String>>, change: (String) -> Unit) {
    Text(title, Modifier.padding(start = 12.dp, top = 20.dp, bottom = 8.dp), fontSize = 13.sp, color = MaterialTheme.colorScheme.onSurfaceVariant)
    Surface(shape = RoundedCornerShape(16.dp), color = MaterialTheme.colorScheme.surfaceContainer) {
        Column {
            options.forEach { (value,label) ->
                Row(Modifier.fillMaxWidth().selectable(selected == value, onClick = { change(value) }).padding(horizontal = 12.dp).height(52.dp), verticalAlignment = Alignment.CenterVertically) {
                    Text(label, Modifier.weight(1f)); RadioButton(selected == value, onClick = null)
                }
            }
        }
    }
}
@Composable internal fun ErrorText(code: String?, modifier: Modifier = Modifier) {
    if (code != null) Text(errorMessage(code), modifier.padding(vertical = 8.dp), color = MaterialTheme.colorScheme.error, fontSize = 13.sp)
}
@Composable internal fun errorMessage(code: String): String = stringResource(when (code) {
    "invalid_ticket", "pair_ticket_invalid", "pair_ticket_consumed", "pairing_invalid_ticket", "ticket_invalid" -> R.string.invalid_ticket
    "ticket_expired", "pair_ticket_expired", "pairing_expired", "pairing_ticket_expired" -> R.string.ticket_expired
    "identity_state_mismatch", "identity_invalid" -> R.string.identity_error
    "storage_unavailable" -> R.string.storage_error
    "unauthorized", "authorization_revoked", "device_revoked", "peer_not_authorized" -> R.string.unauthorized
    "session_not_found", "session_ended" -> R.string.session_ended
    "session_occupied", "controller_busy" -> R.string.session_occupied
    "lease_lost" -> R.string.lease_lost
    "invalid_session_name" -> R.string.invalid_name
    "session_name_conflict", "session_already_exists" -> R.string.duplicate_name
    "invalid_working_directory", "working_directory_unavailable" -> R.string.invalid_directory
    "operation_outcome_unknown", "pair_outcome_unknown" -> R.string.outcome_unknown
    "resource_limit", "resource_exhausted" -> R.string.resource_limit
    "input_not_ready" -> R.string.input_unavailable
    "upload_too_large" -> R.string.upload_too_large
    "upload_source_invalid", "upload_already_started" -> R.string.upload_source_invalid
    "upload_storage_failed" -> R.string.upload_storage_failed
    "upload_outcome_unknown" -> R.string.upload_outcome_unknown
    "service_not_implemented" -> R.string.upload_upgrade
    "no_qr" -> R.string.no_qr
    "selection_changed" -> R.string.selection_changed
    "history_unavailable" -> R.string.history_unavailable
    "image_unreadable" -> R.string.image_unreadable
    "camera_unavailable" -> R.string.camera_unavailable
    else -> R.string.connection_error
})
@Composable internal fun ConfirmDialog(title: String, message: String, action: String,
    dismiss: () -> Unit, confirm: () -> Unit) {
    AlertDialog(onDismissRequest = dismiss, title = { Text(title) }, text = { Text(message) },
        confirmButton = { TextButton(confirm) { Text(action) } },
        dismissButton = { TextButton(dismiss) { Text(stringResource(R.string.cancel)) } })
}
