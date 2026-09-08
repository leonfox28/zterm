package io.github.leonfox28.zterm

import android.content.pm.ApplicationInfo
import android.os.Build
import android.os.Handler
import android.os.HandlerThread
import android.os.SystemClock
import android.view.Choreographer
import android.view.FrameMetrics
import android.view.InputDevice
import android.view.MotionEvent
import android.view.View
import android.view.ViewGroup
import android.view.Window
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.test.platform.app.InstrumentationRegistry
import org.json.JSONArray
import org.json.JSONObject
import org.junit.Assert.assertTrue
import org.junit.Assume.assumeTrue
import org.junit.Rule
import org.junit.Test
import java.io.File
import java.util.Collections
import java.util.concurrent.CountDownLatch
import java.util.concurrent.TimeUnit

/** Opt-in diagnostic, not a device-independent FPS gate. Never uses a default host. */
class TerminalScrollProfileTest {
    @get:Rule val ui = createAndroidComposeRule<MainActivity>()
    private val instrumentation = InstrumentationRegistry.getInstrumentation()
    private val repository get() = (ui.activity.application as ZtermApplication).repository

    @Test fun profileCachedHistoryMotion() {
        val args = InstrumentationRegistry.getArguments()
        assumeTrue("explicit profiling run", args.getString("scrollProfile") == "1")
        assumeTrue("FrameMetrics deadlines", Build.VERSION.SDK_INT >= 31)
        val hostName = args.getString("hostName")
        require(hostName?.startsWith("presentation-") == true)
        ui.waitUntil(20_000) { repository.state.value.initialized }
        val host = repository.state.value.saved.hosts.first { it.name == hostName }
        val preferences = repository.state.value.saved.preferences
        val name = "android-scroll-profile-${System.nanoTime()}"
        var owned: String? = null
        var profile: Profile? = null
        try {
            ui.runOnIdle { repository.goHome(); repository.setPreferences(Preferences("en", "dark", 12)) }
            ui.waitUntil { !repository.state.value.busy && repository.frame.value == null }
            ui.runOnIdle { repository.connectHost(host.id) }
            ui.waitUntil(20_000) { repository.state.value.route == Route.Terminal && !repository.state.value.busy }
            ui.runOnIdle { repository.createSession(name, "") }
            ui.waitUntil(20_000) {
                repository.frame.value?.state == "active" &&
                    repository.state.value.sessions.any { it.name == name } && !repository.state.value.busy
            }
            owned = requireNotNull(repository.state.value.sessionId)
            ui.waitUntil { terminal().hasWindowFocus() && repository.frame.value!!.viewport.rows.toInt() == terminal().height / terminal().gridCellHeight }
            val fixture = "import os; n=os.get_terminal_size().columns-1; " +
                "print('\\n'.join((('ROW_%04d '%i)+'abcdefghijklmnopqrstuvwxyz'*12)[:n] for i in range(800)))"
            val encoded = android.util.Base64.encodeToString(fixture.toByteArray(), android.util.Base64.NO_WRAP)
            ui.runOnIdle { assertTrue(repository.text("python3 -c 'import base64; exec(base64.b64decode(\"$encoded\"))'\r")) }
            ui.waitUntil(20_000) {
                repository.frame.value?.let { frame ->
                    frame.historyMaximum > 700u && frame.rows.any { row -> row.cells.joinToString("") { it.text }.contains("ROW_0799") }
                } == true
            }
            val view = terminal()
            // Populate neighboring native pages and exercise JIT before measuring.
            repeat(7) { index ->
                drag(view, .2f, .75f, 400); SystemClock.sleep(120)
                val frame = repository.frame.value!!
                android.util.Log.i("ZtermScrollProfile", "warmup=$index offset=${frame.historyOffset} rows=${frame.rows.size} hits=${frame.stats.cacheHits} misses=${frame.stats.cacheMisses}")
            }
            ui.waitUntil(20_000) { repository.frame.value!!.historyOffset > 100u }
            SystemClock.sleep(800)
            profile = Profile(view, args.getString("rowWindowMetrics") == "1")
            instrumentation.runOnMainSync { profile.start() }
            for (round in 1..2) {
                measure(profile, "$round-redraw-only") { animate(1_200) { view.invalidate() } }
                // Archived APKs use the original Repository scroll signature.
                if (args.getString("gesturesOnly") != "1") measure(profile, "$round-same-row-requests") {
                    val offset = repository.frame.value!!.historyOffset.toLong()
                    animate(1_200) { repository.scroll(offset, false, 4) }
                }
                measure(profile, "$round-drag-down-slow") { drag(view, .25f, .7f, 1_200) }
                measure(profile, "$round-drag-up-slow") { drag(view, .7f, .25f, 1_200) }
                measure(profile, "$round-drag-down-fast") { drag(view, .2f, .8f, 300) }
                measure(profile, "$round-drag-up-fast") { drag(view, .8f, .2f, 300) }
                measure(profile, "$round-reversal") { drag(view, .25f, .75f, 1_600, reverse = true) }
                measure(profile, "$round-fling-down") {
                    drag(view, .2f, .75f, 160, fling = true)
                    instrumentation.runOnMainSync { profile.markRelease() }
                    SystemClock.sleep(1_800)
                }
                measure(profile, "$round-fling-up") {
                    drag(view, .8f, .25f, 160, fling = true)
                    instrumentation.runOnMainSync { profile.markRelease() }
                    SystemClock.sleep(1_800)
                }
            }
            instrumentation.runOnMainSync { profile.stop() }
            val label = args.getString("profileLabel") ?: "debug"
            require(label.matches(Regex("[a-zA-Z0-9_-]+")))
            val output = File(instrumentation.targetContext.getExternalFilesDir(null), "scroll-profile-$label.json")
            output.writeText(profile.json().toString())
            android.util.Log.i("ZtermScrollProfile", "Saved ${output.name}")
        } finally {
            instrumentation.runOnMainSync { profile?.stop() }
            owned?.let { id ->
                ui.runOnIdle { repository.deleteSession(id) }
                ui.waitUntil(20_000) { !repository.state.value.busy && repository.state.value.sessions.none { it.sessionId == id } }
            }
            ui.runOnIdle { repository.goHome(); repository.setPreferences(preferences) }
        }
    }

    private fun measure(profile: Profile, name: String, work: () -> Unit) {
        instrumentation.runOnMainSync { profile.begin(name) }
        try { work() } finally { instrumentation.runOnMainSync { profile.end() } }
        SystemClock.sleep(150)
    }

    private fun animate(durationMs: Long, step: (Float) -> Unit) {
        val done = CountDownLatch(1)
        var failure: Throwable? = null
        var start = 0L
        val callback = object : Choreographer.FrameCallback {
            override fun doFrame(time: Long) {
                try {
                    if (start == 0L) start = time
                    val fraction = ((time - start).toDouble() / (durationMs * 1_000_000)).coerceIn(0.0, 1.0).toFloat()
                    step(fraction)
                    if (fraction < 1f) Choreographer.getInstance().postFrameCallback(this) else done.countDown()
                } catch (error: Throwable) { failure = error; done.countDown() }
            }
        }
        instrumentation.runOnMainSync { Choreographer.getInstance().postFrameCallback(callback) }
        try {
            assertTrue("animation completed", done.await(durationMs + 10_000, TimeUnit.MILLISECONDS))
            failure?.let { throw it }
        } finally { instrumentation.runOnMainSync { Choreographer.getInstance().removeFrameCallback(callback) } }
    }

    private fun drag(view: TerminalView, from: Float, to: Float, duration: Long, reverse: Boolean = false, fling: Boolean = false) {
        val origin = IntArray(2)
        instrumentation.runOnMainSync { view.getLocationOnScreen(origin) }
        val down = SystemClock.uptimeMillis()
        var ended = false
        val period = 1_000.0 / view.display.refreshRate
        fun event(action: Int, y: Float, sync: Boolean = false) {
            val input = MotionEvent.obtain(down, SystemClock.uptimeMillis(), action,
                origin[0] + view.width * .5f, origin[1] + view.height * y, 0)
            input.source = InputDevice.SOURCE_TOUCHSCREEN
            try { assertTrue("touch injected", instrumentation.uiAutomation.injectInputEvent(input, sync)) }
            finally { input.recycle() }
        }
        event(MotionEvent.ACTION_DOWN, from)
        try {
            var tick = 0
            while (!ended) {
                val fraction = ((SystemClock.uptimeMillis() - down).toFloat() / duration).coerceIn(0f, 1f)
                val progress = if (reverse) 1f - kotlin.math.abs(2f * fraction - 1f) else fraction
                event(MotionEvent.ACTION_MOVE, from + (to - from) * progress)
                if (fraction == 1f) {
                    event(if (fling) MotionEvent.ACTION_UP else MotionEvent.ACTION_CANCEL, if (reverse) from else to, true)
                    ended = true
                } else {
                    // Independent input thread, near the display's cadence.
                    // Direct dispatch from an animation callback would defer
                    // postInvalidateOnAnimation to a later frame than real input.
                    ++tick
                    val wait = down + (tick * period).toLong() - SystemClock.uptimeMillis()
                    if (wait > 0) SystemClock.sleep(wait)
                }
            }
        } finally {
            if (!ended) event(MotionEvent.ACTION_CANCEL, from, true)
        }
    }

    private fun terminal(): TerminalView {
        fun find(view: View): TerminalView? {
            if (view is TerminalView) return view
            if (view is ViewGroup) for (index in 0 until view.childCount) find(view.getChildAt(index))?.let { return it }
            return null
        }
        return requireNotNull(find(ui.activity.window.decorView))
    }

    private inner class Profile(private val view: TerminalView, private val rowWindowMetrics: Boolean) {
        private val metricsThread = HandlerThread("scroll-frame-metrics").apply { start() }
        private val timings = Collections.synchronizedList(mutableListOf<LongArray>())
        private val deliveries = mutableListOf<JSONObject>()
        private val phases = mutableListOf<JSONObject>()
        private var phase: JSONObject? = null
        private var removeObserver: (() -> Unit)? = null
        private var running = false
        private val listener = Window.OnFrameMetricsAvailableListener { _, frame, dropped ->
            timings.add(longArrayOf(
                frame.getMetric(FrameMetrics.INTENDED_VSYNC_TIMESTAMP),
                frame.getMetric(FrameMetrics.TOTAL_DURATION), frame.getMetric(FrameMetrics.DEADLINE),
                frame.getMetric(FrameMetrics.DRAW_DURATION), frame.getMetric(FrameMetrics.LAYOUT_MEASURE_DURATION),
                frame.getMetric(FrameMetrics.GPU_DURATION), frame.getMetric(FrameMetrics.UNKNOWN_DELAY_DURATION),
                frame.getMetric(FrameMetrics.FIRST_DRAW_FRAME), dropped.toLong(),
            ))
        }

        fun start() {
            running = true
            ui.activity.window.addOnFrameMetricsAvailableListener(listener, Handler(metricsThread.looper))
            removeObserver = repository.observeTerminalFrames { frame ->
                if (phase != null && frame != null) {
                    val sample = JSONObject().put("timeNs", System.nanoTime()).put("generation", frame.generation.toLong())
                        .put("offset", frame.historyOffset.toLong())
                    if (rowWindowMetrics) sample.put("contentGeneration", frame.contentGeneration.toLong()).put("rowCount", frame.rows.size)
                    deliveries.add(sample)
                }
            }
        }

        fun begin(name: String) {
            phase = JSONObject().put("name", name).put("startNs", System.nanoTime()).put("before", stats())
        }

        fun end() {
            phase?.let { phases.add(it.put("endNs", System.nanoTime()).put("after", stats())) }
            phase = null
        }

        fun markRelease() { phase?.put("releaseNs", System.nanoTime()) }

        private fun stats(): JSONObject {
            val frame = requireNotNull(repository.frame.value)
            return JSONObject().put("offset", frame.historyOffset.toLong())
                .put("hits", frame.stats.cacheHits.toLong()).put("misses", frame.stats.cacheMisses.toLong())
                .put("requests", frame.stats.requests.toLong())
        }

        fun stop() {
            if (!running) return
            running = false
            removeObserver?.invoke(); removeObserver = null
            ui.activity.window.removeOnFrameMetricsAvailableListener(listener)
            metricsThread.quitSafely()
        }

        fun json(): JSONObject {
            metricsThread.join(2_000)
            val frame = requireNotNull(repository.frame.value)
            return JSONObject()
                .put("sdk", Build.VERSION.SDK_INT).put("refreshHz", view.display.refreshRate)
                .put("debuggable", instrumentation.targetContext.applicationInfo.flags and ApplicationInfo.FLAG_DEBUGGABLE != 0)
                .put("cellHeight", view.gridCellHeight).put("rows", frame.viewport.rows.toInt()).put("columns", frame.viewport.columns.toInt())
                .put("input", "UiAutomation touchscreen, independent display-rate injection")
                .put("deliveries", JSONArray(deliveries)).put("phases", JSONArray(phases))
                .put("metricsColumns", JSONArray(listOf("vsyncNs", "totalNs", "deadlineNs", "drawNs", "layoutNs", "gpuNs", "unknownNs", "firstDraw", "droppedReports")))
                .put("metrics", JSONArray(timings.map { JSONArray(it.toList()) }))
        }
    }
}
