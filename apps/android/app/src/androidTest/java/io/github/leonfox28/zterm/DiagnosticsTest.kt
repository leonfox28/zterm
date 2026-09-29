package io.github.leonfox28.zterm

import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import io.github.leonfox28.zterm.nativebridge.*
import java.io.ByteArrayOutputStream
import java.io.File
import java.util.UUID
import org.json.JSONObject
import org.junit.Assert.*
import org.junit.Test
import org.junit.runner.RunWith

@RunWith(AndroidJUnit4::class)
class DiagnosticsTest {
    @Test fun installingDiagnosticsOnMainResolvesItsDirectoryOnlyOnTheWorker() {
        val instrumentation = InstrumentationRegistry.getInstrumentation()
        val parent = File(instrumentation.targetContext.cacheDir, "diagnostics-${UUID.randomUUID()}")
        assertTrue(parent.mkdir())
        val lookups = java.util.concurrent.atomic.AtomicInteger()
        val context = object : android.content.ContextWrapper(instrumentation.targetContext) {
            override fun getNoBackupFilesDir(): File {
                assertNotEquals(android.os.Looper.getMainLooper().thread, Thread.currentThread())
                lookups.incrementAndGet()
                return parent
            }
        }
        var recorder: NativeDiagnostics? = null
        try {
            instrumentation.runOnMainSync {
                val files = DiagnosticFiles(context)
                assertEquals(0, lookups.get())
                recorder = installDiagnostics(files)
            }
            val active = checkNotNull(recorder)
            active.recordApp(AppDiagnostic.STARTED, null, 0u, 0u)
            assertTrue(active.flush())
            assertEquals(1, lookups.get())
            assertTrue(File(parent, "diagnostics/events.jsonl").readText().contains("app_started"))
        } finally { recorder?.close(); parent.deleteRecursively() }
    }

    @Test fun nativeRecordsPersistRotateAndExportWithoutContentInEveryBuildVariant() {
        val context = InstrumentationRegistry.getInstrumentation().targetContext
        val directory = File(context.cacheDir, "diagnostics-${UUID.randomUUID()}")
        val files = DiagnosticFiles(directory, 8192)
        val recorder = installDiagnostics(files)
        try {
            recorder.recordApp(AppDiagnostic.OPERATION_FAILED, "SECRET_TERMINAL_SENTINEL", 0u, 0u)
            assertTrue(recorder.flush())
            val initial = File(directory, "events.jsonl").readText()
            assertTrue(initial.contains("app_operation_failed"))
            assertFalse(initial.contains("SECRET_TERMINAL_SENTINEL"))
            assertFalse(File(directory, "detail.jsonl").exists())
            files.setControl(diagnosticControl(true))
            waitUntil { recorder.detailEnabled() }
            for (index in 0..70) {
                recorder.recordApp(AppDiagnostic.OPERATION_FAILED, "transport_unavailable", 0u, 0u)
                if (index % 5 == 0) assertTrue(recorder.flush())
            }
            assertTrue(recorder.flush())
            val keyBefore = File(directory, "events.jsonl").readBytes()
            recorder.recordApp(AppDiagnostic.VIEWPORT, null, 80u, 24u)
            assertTrue(recorder.flush())
            assertArrayEquals(keyBefore, File(directory, "events.jsonl").readBytes())
            assertTrue(File(directory, "detail.jsonl").exists())
            assertTrue(directory.listFiles()!!.filter { it.name.endsWith("jsonl") || it.name.endsWith(".1") }.all { it.length() <= 8192 })
            val reopened = DiagnosticFiles(directory, 8192)
            assertTrue(diagnosticRemainingMs(reopened.control()) > 0uL)
            val output = ByteArrayOutputStream()
            reopened.export(output, true, recorder.pendingLost())
            val lines = output.toString("UTF-8").lineSequence().filter { it.isNotBlank() }.toList()
            assertEquals(1, JSONObject(lines.first()).getInt("export_schema"))
            assertTrue(lines.drop(1).all { diagnosticExportRecord(it.toByteArray()) != null })
            assertTrue(lines.any { it.contains("viewport_changed") })
            assertFalse(output.toString("UTF-8").contains("SECRET_TERMINAL_SENTINEL"))
            val expiredAt = System.currentTimeMillis() - 100
            files.setControl(DiagnosticControl(1u, expiredAt - 900_000, expiredAt))
            waitUntil { !recorder.detailEnabled() }
            val detailBefore = File(directory, "detail.jsonl").readBytes()
            recorder.recordApp(AppDiagnostic.VIEWPORT, null, 90u, 30u)
            assertTrue(recorder.flush())
            assertArrayEquals(detailBefore, File(directory, "detail.jsonl").readBytes())
        } finally { recorder.close(); directory.deleteRecursively() }
    }
    @Test fun inspectionCreatesNothingAndUnsafeDestinationsFailWithoutChangingTheirTarget() {
        val context = InstrumentationRegistry.getInstrumentation().targetContext
        val directory = File(context.cacheDir, "diagnostics-${UUID.randomUUID()}")
        val outside = File(context.cacheDir, "diagnostic-target-${UUID.randomUUID()}")
        val files = DiagnosticFiles(directory)
        try {
            assertEquals(0uL, diagnosticRemainingMs(files.control()))
            val empty = ByteArrayOutputStream(); files.export(empty, false, 0u)
            assertFalse(directory.exists())
            files.setControl(diagnosticControl(false))
            outside.writeText("unchanged")
            android.system.Os.symlink(outside.path, File(directory, "events.jsonl").path)
            assertFalse(files.append(false, "safe\n"))
            assertEquals("unchanged", outside.readText())
        } finally { directory.deleteRecursively(); outside.delete() }
    }
    @Test fun oversizedExistingFilesNormalizeBeforeRotation() {
        val context = InstrumentationRegistry.getInstrumentation().targetContext
        val directory = File(context.cacheDir, "diagnostics-${UUID.randomUUID()}")
        val files = DiagnosticFiles(directory, 8192)
        try {
            files.setControl(diagnosticControl(false))
            val current = File(directory, "events.jsonl")
            current.writeText("x".repeat(20_000) + "\ncomplete legacy event\n")
            android.system.Os.chmod(current.path, 0x180)
            assertTrue(files.append(false, "new event\n"))
            assertEquals("complete legacy event\nnew event\n", current.readText())
            assertTrue(directory.listFiles()!!.all { it.length() <= 8192 })
        } finally { directory.deleteRecursively() }
    }
    private fun waitUntil(predicate: () -> Boolean) {
        val deadline = android.os.SystemClock.elapsedRealtime() + 5000
        while (!predicate()) { assertTrue(android.os.SystemClock.elapsedRealtime() < deadline); Thread.sleep(10) }
    }
}
