package io.github.leonfox28.zterm

import androidx.activity.compose.BackHandler
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.layout.Layout
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.unit.Constraints
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.compose.ui.viewinterop.AndroidView
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import io.github.leonfox28.zterm.nativebridge.NativeKey
import io.github.leonfox28.zterm.nativebridge.NativeSession

@OptIn(ExperimentalLayoutApi::class)
@Composable internal fun TerminalScreen(state: AppState, repository: AppRepository) {
    val frame by repository.frame.collectAsStateWithLifecycle()
    var modifiers by remember { mutableIntStateOf(0) }
    var creating by remember { mutableStateOf(false) }
    var renaming by remember { mutableStateOf<NativeSession?>(null) }
    var deleting by remember { mutableStateOf<NativeSession?>(null) }
    var taking by remember { mutableStateOf<String?>(null) }
    var cellHeight by remember { mutableIntStateOf(1) }
    var terminalView by remember { mutableStateOf<TerminalView?>(null) }
    val session = state.sessions.firstOrNull { it.sessionId == state.sessionId }
    val host = state.saved.hosts.firstOrNull { it.id == state.hostId }
    val imeVisible = WindowInsets.isImeVisible
    val imeAnimation = LocalImeAnimation.current
    DisposableEffect(terminalView, imeAnimation) {
        val stop = terminalView?.let { imeAnimation.observe(it::updateImeAnimation) }
        onDispose { stop?.invoke() }
    }
    BackHandler(enabled = state.panel || !imeVisible) { if (state.panel) repository.closePanel() else repository.goHome() }
    LaunchedEffect(state.needsSession) { if (state.needsSession) { creating = true; repository.sessionPromptShown() } }
    LaunchedEffect(frame?.inputEpoch, state.sessionId) { modifiers = 0 }
    BoxWithConstraints(Modifier.fillMaxSize().statusBarsPadding().navigationBarsPadding().imePadding()) {
        val panelHeight = (maxHeight * .65f).coerceAtMost(420.dp)
        TerminalGridLayout(cellHeight, imeAnimation, Modifier.fillMaxSize()) {
            Row(Modifier.fillMaxWidth().height(48.dp), verticalAlignment = Alignment.CenterVertically) {
                IconAction("back", stringResource(R.string.back)) { repository.goHome() }
                Row(Modifier.weight(1f).fillMaxHeight().clickable { repository.togglePanel() }, verticalAlignment = Alignment.CenterVertically) {
                    Column(Modifier.weight(1f)) {
                        Text(session?.name ?: host?.name ?: "zterm", maxLines = 1, fontSize = 16.sp, fontWeight = FontWeight.SemiBold)
                        if (session != null) Text(host?.name.orEmpty(), maxLines = 1, fontSize = 11.sp, color = MaterialTheme.colorScheme.onSurfaceVariant)
                    }
                    LineIcon("down", Modifier.padding(horizontal = 12.dp).size(20.dp))
                }
            }
            Box(Modifier.fillMaxWidth()) {
                AndroidView(factory = { context -> TerminalView(context).apply {
                        this.repository = repository; terminalView = this
                        onCellHeightChanged = { cellHeight = it }; onCellHeightChanged(gridCellHeight)
                    } },
                    modifier = Modifier.fillMaxSize(), onRelease = { terminalView = null }, update = { view ->
                        view.imeAnimating = { imeAnimation.running }
                        view.pendingModifiers = { modifiers }; view.consumedModifiers = { modifiers = 0 }
                        view.update(frame, state.saved.preferences.fontSize)
                    })
                val terminalState = frame?.state
                val error = state.error ?: frame?.error
                val canTakeover = state.sessionId != null && (terminalState == "lease_lost" || error in setOf("session_occupied", "controller_busy", "lease_lost"))
                val inactive = terminalState in setOf("closed", "ended", "lease_lost") || state.error != null && frame == null
                if (inactive || frame == null && !state.panel) {
                    Surface(Modifier.align(Alignment.Center).padding(24.dp), shape = RoundedCornerShape(16.dp), color = MaterialTheme.colorScheme.surfaceContainer) {
                        Column(Modifier.padding(20.dp), horizontalAlignment = Alignment.CenterHorizontally) {
                            if (state.busy) CircularProgressIndicator(Modifier.size(24.dp))
                            else {
                                Text(if (error != null) errorMessage(error) else stringResource(when (terminalState) {
                                    "ended" -> R.string.session_ended; "lease_lost" -> R.string.lease_lost; else -> R.string.connection_error
                                }))
                                Row {
                                    if (canTakeover) TextButton({ taking = state.sessionId }) { Text(stringResource(R.string.takeover)) }
                                    else TextButton(repository::retry) { Text(stringResource(R.string.retry)) }
                                    TextButton(repository::togglePanel) { Text(stringResource(R.string.sessions)) }
                                }
                            }
                        }
                    }
                }
                if (terminalState == "reconnecting" || terminalState == "synchronizing" && frame?.inputReady != true) {
                    LinearProgressIndicator(Modifier.align(Alignment.TopCenter).fillMaxWidth())
                }
                val notice = state.error ?: frame?.notice
                if (notice != null && frame != null && !inactive) {
                    Surface(Modifier.align(Alignment.TopCenter).padding(8.dp), shape = RoundedCornerShape(8.dp)) {
                        Row(verticalAlignment = Alignment.CenterVertically) {
                            ErrorText(notice, Modifier.padding(horizontal = 12.dp))
                            IconAction("close", stringResource(R.string.close)) { repository.clearError() }
                        }
                    }
                }
            }
            HorizontalDivider(color = MaterialTheme.colorScheme.outlineVariant)
            Row(Modifier.fillMaxWidth().height(48.dp).padding(horizontal = 2.dp), verticalAlignment = Alignment.CenterVertically) {
                val keys = listOf("Esc" to NativeKey.Escape, "Tab" to NativeKey.Tab, "Ctrl" to null, "Alt" to null,
                    "←" to NativeKey.Left, "↓" to NativeKey.Down, "↑" to NativeKey.Up, "→" to NativeKey.Right)
                keys.forEach { (label,key) ->
                    val bit = when(label) { "Ctrl" -> 4; "Alt" -> 2; else -> 0 }
                    TextButton(onClick = {
                        if (bit != 0) modifiers = modifiers xor bit
                        else if (key != null) { repository.key(key,modifiers); repository.key(key,modifiers,3); modifiers = 0 }
                    }, enabled = frame?.inputReady == true, modifier = Modifier.weight(1f).fillMaxHeight(), contentPadding = PaddingValues(0.dp),
                        colors = ButtonDefaults.textButtonColors(containerColor = if (bit != 0 && modifiers and bit != 0) MaterialTheme.colorScheme.primaryContainer else androidx.compose.ui.graphics.Color.Transparent)) {
                        Text(label, fontSize = 13.sp, fontFamily = FontFamily.Monospace)
                    }
                }
                val keyboardLabel = stringResource(if (imeVisible) R.string.hide_keyboard else R.string.show_keyboard)
                TextButton(onClick = { terminalView?.setKeyboardVisible(!imeVisible) },
                    enabled = frame?.inputReady == true || imeVisible,
                    modifier = Modifier.weight(1f).fillMaxHeight().semantics { contentDescription = keyboardLabel },
                    contentPadding = PaddingValues(0.dp)) {
                    LineIcon("keyboard", Modifier.size(18.dp))
                }
            }
        }
        if (state.panel) {
            Box(Modifier.fillMaxSize().padding(top = 48.dp).clickable(indication = null, interactionSource = remember { androidx.compose.foundation.interaction.MutableInteractionSource() }) { repository.closePanel() })
            Surface(Modifier.padding(top = 48.dp).fillMaxWidth().heightIn(max = panelHeight), color = MaterialTheme.colorScheme.surfaceContainer, shadowElevation = 12.dp,
                shape = RoundedCornerShape(bottomStart = 16.dp, bottomEnd = 16.dp)) {
                Column {
                    if (state.busy) LinearProgressIndicator(Modifier.fillMaxWidth())
                    LazyColumn(Modifier.weight(1f, fill = false)) {
                        items(state.sessions, key = { it.sessionId }) { item ->
                            var menu by remember { mutableStateOf(false) }
                            // sessionId is also the failed/retry target; only a
                            // retained connection can own the current row.
                            val current = item.sessionId == state.sessionId && frame?.state in setOf("active", "synchronizing", "reconnecting")
                            Row(Modifier.fillMaxWidth().clickable(enabled = !state.busy) {
                                when {
                                    current -> repository.closePanel()
                                    item.occupied -> taking = item.sessionId
                                    else -> repository.selectSession(item.sessionId)
                                }
                            }.padding(start = 20.dp), verticalAlignment = Alignment.CenterVertically) {
                                Column(Modifier.weight(1f).padding(vertical = 12.dp)) {
                                    Text(item.name, maxLines = 1, color = if (current) MaterialTheme.colorScheme.primary else MaterialTheme.colorScheme.onSurface)
                                    if (current || item.occupied) Text(stringResource(if(current) R.string.current_session else R.string.occupied), fontSize = 11.sp, color = MaterialTheme.colorScheme.onSurfaceVariant)
                                }
                                Box {
                                    IconAction("more", stringResource(R.string.session_menu), !state.busy) { menu = true }
                                    DropdownMenu(menu, { menu = false }) {
                                        DropdownMenuItem(text = { Text(stringResource(R.string.rename)) }, onClick = { menu = false; renaming = item })
                                        DropdownMenuItem(text = { Text(stringResource(R.string.delete)) }, onClick = { menu = false; deleting = item })
                                    }
                                }
                            }
                        }
                    }
                    ErrorText(state.error, Modifier.padding(horizontal = 20.dp))
                    TextButton({ creating = true }, Modifier.fillMaxWidth().height(52.dp), enabled = !state.busy) {
                        LineIcon("plus"); Spacer(Modifier.width(8.dp)); Text(stringResource(R.string.new_session))
                    }
                }
            }
        }
    }
    if (creating) SessionForm(null, state.busy, state.error, { creating = false; repository.clearError() }) { name,directory -> repository.createSession(name,directory) }
    renaming?.let { row -> SessionForm(row.name, state.busy, state.error, { renaming = null; repository.clearError() }) { name,_ -> repository.renameSession(row.sessionId,name) } }
    LaunchedEffect(state.busy, state.error, state.sessionId, state.sessions) {
        if (!state.busy && state.error == null) {
            if (frame != null) creating = false
            renaming?.let { old -> if (state.sessions.any { it.sessionId == old.sessionId && it.name != old.name }) renaming = null }
        }
    }
    deleting?.let { row -> ConfirmDialog(stringResource(R.string.delete_session), row.name + "\n" + stringResource(R.string.delete_session_message), stringResource(R.string.delete), { deleting = null }) { deleting = null; repository.deleteSession(row.sessionId) } }
    taking?.let { id ->
        val name = state.sessions.firstOrNull { it.sessionId == id }?.name
        ConfirmDialog(stringResource(R.string.takeover), name?.let { "$it\n" }.orEmpty() + stringResource(R.string.takeover_message), stringResource(R.string.takeover), { taking = null }) {
            taking = null; repository.selectSession(id,true)
        }
    }
}
/** The fifth, unoccupied area is below the fixed toolbar, inside the already consumed insets. */
@OptIn(ExperimentalLayoutApi::class)
@Composable private fun TerminalGridLayout(cellHeight: Int, imeAnimation: ImeAnimationState, modifier: Modifier, content: @Composable () -> Unit) {
    val density = LocalDensity.current
    val ime = WindowInsets.ime
    val start = WindowInsets.imeAnimationSource
    val end = WindowInsets.imeAnimationTarget
    val navigation = WindowInsets.navigationBars
    Layout(content = content, modifier = modifier) { children, constraints ->
        val chromeConstraints = constraints.copy(minHeight = 0)
        val header = children[0].measure(chromeConstraints)
        val divider = children[2].measure(chromeConstraints)
        val toolbar = children[3].measure(chromeConstraints)
        val available = (constraints.maxHeight - header.height - divider.height - toolbar.height).coerceAtLeast(0)
        val nav = navigation.getBottom(density)
        val remainder = terminalBottomRemainder(available, cellHeight, maxOf(nav, ime.getBottom(density)),
            maxOf(nav, start.getBottom(density)), maxOf(nav, end.getBottom(density)), imeAnimation.running)
        val terminal = children[1].measure(Constraints.fixed(constraints.maxWidth, available - remainder))
        layout(constraints.maxWidth, constraints.maxHeight) {
            header.placeRelative(0, 0)
            terminal.placeRelative(0, header.height)
            divider.placeRelative(0, header.height + terminal.height)
            toolbar.placeRelative(0, header.height + terminal.height + divider.height)
        }
    }
}
@Composable private fun SessionForm(previous: String?, busy: Boolean, error: String?, dismiss: () -> Unit, submit: (String,String) -> Unit) {
    var name by remember(previous) { mutableStateOf(previous.orEmpty()) }
    var directory by remember { mutableStateOf("") }
    AlertDialog(onDismissRequest = dismiss, title = { Text(stringResource(if (previous == null) R.string.new_session else R.string.rename)) },
        text = { Column(verticalArrangement = Arrangement.spacedBy(12.dp)) {
            OutlinedTextField(name, { if (it.length <= 128) name = it }, label = { Text(stringResource(R.string.name)) }, singleLine = true, enabled = !busy)
            if (previous == null) OutlinedTextField(directory, { if (it.length <= 4096) directory = it }, label = { Text(stringResource(R.string.directory_optional)) }, singleLine = true, enabled = !busy)
            ErrorText(error)
        } }, confirmButton = { TextButton({ submit(name,directory) }, enabled = name.isNotBlank() && !busy) { Text(stringResource(if (previous == null) R.string.create else R.string.save)) } },
        dismissButton = { TextButton(dismiss) { Text(stringResource(R.string.cancel)) } })
}
