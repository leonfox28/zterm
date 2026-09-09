package io.github.leonfox28.zterm

import androidx.compose.material3.MaterialTheme
import androidx.compose.runtime.*
import androidx.compose.ui.test.*
import androidx.compose.ui.test.junit4.createComposeRule
import org.junit.Assert.assertEquals
import org.junit.Rule
import org.junit.Test

class UploadUiTest {
    @get:Rule val ui = createComposeRule()

    @Test fun progressSizeSpeedAndExplicitRetryUseTheSameDialog() {
        var upload by mutableStateOf(UploadUi(1, UploadPicker.File, phase = "preparing", filename = "中文 report.pdf"))
        var cancelled = 0
        var retried = 0
        ui.setContent { MaterialTheme { UploadProgressDialog(upload, { cancelled++ }, { retried++ }) } }
        ui.onNodeWithText("中文 report.pdf").assertIsDisplayed()
        ui.onAllNodes(isDialog()).assertCountEquals(1)
        ui.runOnIdle { upload = upload.copy(phase = "uploading", totalBytes = 50_000_000, acceptedBytes = 25_000_000, bytesPerSecond = 2_000_000.0) }
        ui.onNodeWithText("50%").assertIsDisplayed()
        ui.onNodeWithText("Size: 50.00 MB").assertIsDisplayed()
        ui.onNodeWithText("Speed: 2.00 MB/s").assertIsDisplayed()
        ui.onNodeWithText("Cancel").performClick()
        ui.runOnIdle { assertEquals(1, cancelled); upload = upload.copy(phase = "failed", error = "upload_outcome_unknown") }
        ui.onNodeWithText("Retry").performClick()
        ui.runOnIdle { assertEquals(1, retried) }
        ui.onNodeWithText("Close").performClick()
        ui.runOnIdle { assertEquals(2, cancelled) }
    }
}
