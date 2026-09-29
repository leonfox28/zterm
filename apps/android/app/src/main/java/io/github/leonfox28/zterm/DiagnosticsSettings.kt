package io.github.leonfox28.zterm

import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.layout.*
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.collectAsStateWithLifecycle

@Composable internal fun DiagnosticsSettings() {
    val app = LocalContext.current.applicationContext as ZtermApplication
    val diagnostics = app.diagnostics
    val state by diagnostics.state.collectAsStateWithLifecycle()
    var includeDebug by rememberSaveable { mutableStateOf(false) }
    var exportIncludesDebug by rememberSaveable { mutableStateOf(false) }
    val export = rememberLauncherForActivityResult(ActivityResultContracts.CreateDocument("application/x-ndjson")) { uri ->
        if (uri != null) diagnostics.export(uri, exportIncludesDebug)
    }
    Column(Modifier.fillMaxWidth().padding(horizontal = 12.dp, vertical = 16.dp)) {
        Text(stringResource(R.string.diagnostics), style = MaterialTheme.typography.titleSmall)
        Text(stringResource(R.string.diagnostics_description), style = MaterialTheme.typography.bodySmall, modifier = Modifier.padding(vertical = 8.dp))
        val detailLabel = stringResource(R.string.diagnostics_detail)
        Row(verticalAlignment = Alignment.CenterVertically) {
            Text(detailLabel, Modifier.weight(1f))
            Switch(state.remainingSeconds > 0, { diagnostics.setEnabled(it) }, enabled = !state.busy, modifier = Modifier.testTag("diagnostics-detail").semantics { contentDescription = detailLabel })
        }
        if (state.remainingSeconds > 0) {
            Text(stringResource(R.string.diagnostics_remaining, state.remainingSeconds), style = MaterialTheme.typography.bodySmall)
            TextButton({ diagnostics.setEnabled(true) }, enabled = !state.busy) { Text(stringResource(R.string.diagnostics_renew)) }
        }
        Row(verticalAlignment = Alignment.CenterVertically) {
            Checkbox(includeDebug, { includeDebug = it }, enabled = !state.busy, modifier = Modifier.testTag("diagnostics-include-detail"))
            Text(stringResource(R.string.diagnostics_include_detail), style = MaterialTheme.typography.bodySmall)
        }
        OutlinedButton({ exportIncludesDebug = includeDebug; export.launch("zterm-logs.jsonl") }, enabled = !state.busy) { Text(stringResource(R.string.diagnostics_export)) }
        if (state.busy) LinearProgressIndicator(Modifier.fillMaxWidth())
        if (state.failed) Text(stringResource(R.string.diagnostics_failed), color = MaterialTheme.colorScheme.error)
    }
}
