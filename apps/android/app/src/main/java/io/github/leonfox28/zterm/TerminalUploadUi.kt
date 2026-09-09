package io.github.leonfox28.zterm

import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.PickVisualMediaRequest
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.layout.*
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.ui.Modifier
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.window.DialogProperties
import androidx.lifecycle.compose.collectAsStateWithLifecycle

@Composable internal fun UploadToolbarButtons(
    uploads: TerminalUploads,
    inputReady: Boolean,
    modifier: Modifier,
    restoreFocus: () -> Unit,
) {
    val upload by uploads.state.collectAsStateWithLifecycle()
    var imageRequest by rememberSaveable { mutableLongStateOf(0) }
    var fileRequest by rememberSaveable { mutableLongStateOf(0) }
    var hadUpload by remember { mutableStateOf(false) }
    val images = rememberLauncherForActivityResult(ActivityResultContracts.PickVisualMedia()) { uri -> uploads.selected(imageRequest, uri) }
    val files = rememberLauncherForActivityResult(ActivityResultContracts.OpenDocument()) { uri -> uploads.selected(fileRequest, uri) }
    LaunchedEffect(upload?.id, upload?.phase) {
        val current = upload
        if (current == null) { if (hadUpload) restoreFocus(); hadUpload = false; return@LaunchedEffect }
        hadUpload = true
        if (current.phase == "picking" && uploads.pickerLaunched(current.id)) {
            try {
                when (current.picker) {
                    UploadPicker.Image -> { imageRequest = current.id; images.launch(PickVisualMediaRequest(ActivityResultContracts.PickVisualMedia.ImageOnly)) }
                    UploadPicker.File -> { fileRequest = current.id; files.launch(arrayOf("*/*")) }
                }
            } catch (_: Exception) { uploads.pickerFailed(current.id) }
        }
    }
    for (picker in UploadPicker.entries) {
        val label = stringResource(if (picker == UploadPicker.Image) R.string.upload_image else R.string.upload_file)
        TextButton(onClick = { uploads.choose(picker) }, enabled = inputReady && upload?.active != true,
            modifier = modifier.semantics { contentDescription = label }, contentPadding = PaddingValues(0.dp)) {
            LineIcon(if (picker == UploadPicker.Image) "image" else "attachment", Modifier.size(18.dp))
        }
    }
    upload?.takeIf { it.showDialog }?.let { current ->
        UploadProgressDialog(current, uploads::cancel, uploads::retry)
    }
}

@Composable internal fun UploadProgressDialog(upload: UploadUi, cancel: () -> Unit, retry: () -> Unit) {
    val active = upload.active
    AlertDialog(onDismissRequest = cancel,
        properties = DialogProperties(dismissOnBackPress = true, dismissOnClickOutside = false),
        title = { Text(stringResource(if (active) R.string.upload else R.string.upload_failed)) },
        text = {
            Column(verticalArrangement = Arrangement.spacedBy(12.dp)) {
                Text(upload.filename.ifBlank { stringResource(R.string.upload_selected_file) }, maxLines = 2, overflow = TextOverflow.Ellipsis)
                Text(upload.totalBytes?.let { stringResource(R.string.upload_size, it / 1_000_000.0) } ?: stringResource(R.string.upload_size_unknown))
                if (active) {
                    if (upload.phase == "preparing") {
                        LinearProgressIndicator(Modifier.fillMaxWidth())
                        Text(stringResource(R.string.upload_preparing))
                    } else {
                        val total = upload.totalBytes ?: 0L
                        val fraction = if (total == 0L) 1f else (upload.acceptedBytes.toDouble() / total).toFloat().coerceIn(0f, 1f)
                        LinearProgressIndicator(progress = { fraction }, modifier = Modifier.fillMaxWidth())
                        Text(stringResource(R.string.upload_percent, (fraction * 100).toInt()))
                        Text(stringResource(R.string.upload_speed, upload.bytesPerSecond / 1_000_000.0))
                        if (upload.phase in setOf("finishing", "inserting")) Text(stringResource(R.string.upload_finishing))
                    }
                }
                upload.error?.let { ErrorText(it) }
            }
        },
        confirmButton = { TextButton(if (active) cancel else retry) { Text(stringResource(if (active) R.string.cancel else R.string.retry)) } },
        dismissButton = { if (!active) TextButton(cancel) { Text(stringResource(R.string.close)) } })
}
