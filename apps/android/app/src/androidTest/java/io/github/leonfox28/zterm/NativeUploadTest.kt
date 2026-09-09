package io.github.leonfox28.zterm

import androidx.test.platform.app.InstrumentationRegistry
import android.content.ContentValues
import android.provider.MediaStore
import io.github.leonfox28.zterm.nativebridge.*
import java.io.File
import java.security.MessageDigest
import java.security.SecureRandom
import kotlinx.coroutines.*
import org.junit.Assert.*
import org.junit.Assume.assumeTrue
import org.junit.Test

/** A fresh disposable host ticket, never an existing user's Session or clipboard. */
class NativeUploadTest {
    @Test fun realHostUploadPreservesBytesPausesInputAndInsertsWithoutEnter() = runBlocking {
        assumeTrue("explicit disposable upload host", InstrumentationRegistry.getArguments().getString("uploadFixture") == "1")
        val context = InstrumentationRegistry.getInstrumentation().targetContext
        val ticket = File(context.filesDir, "upload-ticket.txt")
        val seed = ByteArray(32).also { SecureRandom().nextBytes(it) }
        val source = File.createTempFile("upload-test-", ".pdf", context.cacheDir)
        source.outputStream().use { output -> repeat(2_000_013 / 251) { output.write(ByteArray(251) { it.toByte() }) } }
        val digest = MessageDigest.getInstance("SHA-256").digest(source.readBytes()).joinToString("") { "%02x".format(it) }
        try {
            NativeRuntime().use { runtime ->
                try {
                    runtime.initialize(seed, PlatformNetwork(context) {}.current(), emptyList())
                    val host = withTimeout(25_000) { runtime.pairTicket(ticket.readText().trim()) }.use { pairing ->
                        val host = pairing.host()
                        assertTrue("test-owned host", host.name.startsWith("upload-"))
                        runtime.commitPairing(pairing)
                        host
                    }
                    ticket.delete()
                    val viewport = NativeViewport(24u, 100u)
                    val session = runtime.createSession(host.deviceId, "upload-acceptance", null, viewport, true)
                    try {
                        runtime.connectTerminal(host.deviceId, session.sessionId, viewport, true, false).use { terminal ->
                            val epoch = active(terminal)
                            // The host prints a marker only after receiving Enter and verifies the
                            // original file's digest before removing this test-owned upload.
                            val command = "stty -echo; printf '\\033[2J\\033[H'; printf 'UPLOAD_%s\\n' READY; IFS= read -r uploaded; actual=\$(shasum -a 256 \"\$uploaded\"); case \"\$actual\" in '$digest '*) printf 'UPLOAD_%s\\n' EXACT;; *) printf 'UPLOAD_%s\\n' BAD;; esac; rm -f -- \"\$uploaded\"; rmdir -- \"\${uploaded%/*}\"; stty echo\r"
                            terminal.commitText(epoch, command, 0u, false)
                            awaitText(terminal, "UPLOAD_READY")
                            terminal.reserveUpload(epoch).use { cancelled ->
                                cancelled.cancel()
                                assertEquals("cancelled", finished(cancelled).phase)
                            }
                            terminal.reserveUpload(epoch).use { upload ->
                                val failure = runCatching { terminal.commitText(epoch, "MUST_NOT_REPLAY", 0u, false) }.exceptionOrNull()
                                assertTrue(failure is NativeException.RequestFailed && failure.code == "input_not_ready")
                                upload.start(source.absolutePath, source.length().toULong(), "pdf")
                                val done = finished(upload)
                                assertEquals(done.error, "completed", done.phase)
                                assertEquals(source.length().toULong(), done.acceptedBytes)
                                val frame = terminal.currentFrame()
                                assertEquals("upload pause preserves IME epoch", epoch, frame.inputEpoch)
                                assertFalse("no automatic Enter", text(frame).contains("UPLOAD_EXACT"))
                            }
                            terminal.commitText(epoch, "\r", 0u, false)
                            awaitText(terminal, "UPLOAD_EXACT")
                            if (android.os.Build.VERSION.SDK_INT >= 29) {
                                // Exercise the production ContentResolver staging/retained owner
                                // with a real system URI and duplicate picker-result delivery.
                                terminal.commitText(epoch, command.replace("READY", "READY_STAGE").replace("EXACT", "EXACT_STAGE"), 0u, false)
                                awaitText(terminal, "UPLOAD_READY_STAGE")
                                val resolver = context.contentResolver
                                val uri = requireNotNull(resolver.insert(MediaStore.Downloads.EXTERNAL_CONTENT_URI, ContentValues().apply {
                                    put(MediaStore.MediaColumns.DISPLAY_NAME, "zterm-upload-test-${System.nanoTime()}.pdf")
                                    put(MediaStore.MediaColumns.MIME_TYPE, "application/pdf")
                                    put(MediaStore.MediaColumns.RELATIVE_PATH, "Download")
                                }))
                                val scope = CoroutineScope(SupervisorJob() + Dispatchers.Main.immediate)
                                val uploads = TerminalUploads(context, scope) { terminal to epoch }
                                try {
                                    requireNotNull(resolver.openOutputStream(uri)).use { output -> source.inputStream().use { it.copyTo(output) } }
                                    withContext(Dispatchers.Main.immediate) { uploads.choose(UploadPicker.File) }
                                    withTimeout(10_000) { while (uploads.state.value?.phase != "picking") { check(uploads.state.value?.phase != "failed"); delay(10) } }
                                    withContext(Dispatchers.Main.immediate) {
                                        val id = requireNotNull(uploads.state.value).id
                                        assertTrue(uploads.paused)
                                        assertTrue(uploads.pickerLaunched(id))
                                        assertFalse("no duplicate launch on recreation", uploads.pickerLaunched(id))
                                        uploads.selected(id, uri)
                                        uploads.selected(id, uri)
                                    }
                                    withTimeout(45_000) {
                                        while (uploads.state.value != null) {
                                            check(uploads.state.value?.phase != "failed") { "URI upload failed: ${uploads.state.value?.error}" }
                                            delay(10)
                                        }
                                    }
                                    terminal.commitText(epoch, "\r", 0u, false)
                                    awaitText(terminal, "UPLOAD_EXACT_STAGE")
                                } finally {
                                    withContext(Dispatchers.Main.immediate) { uploads.retire() }
                                    scope.cancel()
                                    resolver.delete(uri, null, null)
                                }
                            }
                            terminal.detach()
                        }
                    } finally { runtime.closeSession(host.deviceId, session.sessionId) }
                } finally { runtime.shutdown() }
            }
        } finally { seed.fill(0); source.delete(); ticket.delete() }
    }

    private suspend fun finished(upload: NativeUpload): NativeUploadState = withTimeout(45_000) {
        var state = upload.currentState()
        while (state.phase !in setOf("completed", "failed", "cancelled")) state = upload.waitForProgress(state.generation)
        state
    }
    private suspend fun active(terminal: NativeTerminal): ULong = withTimeout(20_000) {
        var frame = terminal.currentFrame()
        while (frame.state != "active") {
            val generation = frame.generation; frame.source?.close()
            frame = terminal.waitForFrame(generation)
        }
        frame.source?.close(); frame.inputEpoch
    }
    private suspend fun awaitText(terminal: NativeTerminal, marker: String) = withTimeout(15_000) {
        var frame = terminal.currentFrame()
        while (!text(frame).contains(marker)) {
            assertFalse(frame.state in setOf("closed", "lease_lost", "ended"))
            frame = terminal.waitForFrame(frame.generation)
        }
    }
    private fun text(frame: NativeFrame): String = try {
        (frame.source?.presentationRows() ?: frame.rows).joinToString("\n") { row -> row.cells.joinToString("") { it.text } }
    } finally { frame.source?.close() }
}
