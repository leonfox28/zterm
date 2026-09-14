package io.github.leonfox28.zterm

import android.os.Handler
import android.os.HandlerThread
import android.view.FrameMetrics
import android.view.View
import android.view.ViewGroup
import android.view.Window
import android.view.inputmethod.InputMethodManager
import androidx.activity.compose.setContent
import androidx.compose.material3.MaterialTheme
import androidx.compose.runtime.CompositionLocalProvider
import androidx.core.view.ViewCompat
import androidx.test.core.app.ActivityScenario
import androidx.test.platform.app.InstrumentationRegistry
import org.junit.Assert.assertTrue
import org.junit.Test
import java.util.Collections
import java.util.concurrent.CountDownLatch
import java.util.concurrent.TimeUnit

/** Archived layout-only diagnostic. See ime-motion.md for the temporary counter hooks. */
class TerminalMotionProbeTest {
    @Test fun profileProductionImeLayout() {
        val instrumentation = InstrumentationRegistry.getInstrumentation()
        val samples = Collections.synchronizedList(mutableListOf<LongArray>())
        val thread = HandlerThread("ime-layout-metrics").apply { start() }
        val listener = Window.OnFrameMetricsAvailableListener { _, metrics, _ ->
            samples.add(longArrayOf(
                metrics.getMetric(FrameMetrics.INTENDED_VSYNC_TIMESTAMP),
                metrics.getMetric(FrameMetrics.TOTAL_DURATION),
                metrics.getMetric(FrameMetrics.LAYOUT_MEASURE_DURATION),
                metrics.getMetric(FrameMetrics.DRAW_DURATION),
            ))
        }
        fun terminal(root: View): TerminalView? {
            if (root is TerminalView) return root
            if (root is ViewGroup) for (i in 0 until root.childCount) terminal(root.getChildAt(i))?.let { return it }
            return null
        }
        ActivityScenario.launch(MainActivity::class.java).use { scenario ->
            lateinit var activity: MainActivity
            lateinit var view: TerminalView
            var completed = CountDownLatch(1)
            var started = false
            var stop: (() -> Unit)? = null
            scenario.onActivity {
                activity = it
                val owner = ImeAnimationState()
                ViewCompat.setWindowInsetsAnimationCallback(it.window.decorView, owner)
                stop = owner.observe { running ->
                    if (running) started = true else if (started) completed.countDown()
                }
                val repository = (it.application as ZtermApplication).repository
                it.setContent {
                    CompositionLocalProvider(LocalImeAnimation provides owner) {
                        MaterialTheme { TerminalScreen(AppState(initialized = true), repository) }
                    }
                }
                it.window.addOnFrameMetricsAvailableListener(listener, Handler(thread.looper))
            }
            instrumentation.waitForIdleSync()
            scenario.onActivity { view = requireNotNull(terminal(it.window.decorView)) }
            try {
                repeat(4) { round ->
                    for (show in listOf(true, false)) {
                        var begin = 0L
                        scenario.onActivity {
                            completed = CountDownLatch(1); started = false
                            TerminalMotionProbe.compositions = 0; TerminalMotionProbe.updates = 0
                            begin = System.nanoTime()
                            val manager = it.getSystemService(InputMethodManager::class.java)
                            if (show) { view.requestFocus(); manager.showSoftInput(view, InputMethodManager.SHOW_IMPLICIT) }
                            else manager.hideSoftInputFromWindow(view.windowToken, 0)
                        }
                        assertTrue("system IME completed show=$show", completed.await(10, TimeUnit.SECONDS))
                        val drawn = CountDownLatch(1)
                        view.postOnAnimation { view.postOnAnimation { drawn.countDown() } }
                        assertTrue(drawn.await(5, TimeUnit.SECONDS))
                        var end = 0L
                        var compositions = 0
                        var updates = 0
                        scenario.onActivity {
                            end = System.nanoTime()
                            compositions = TerminalMotionProbe.compositions
                            updates = TerminalMotionProbe.updates
                        }
                        val rows = synchronized(samples) { samples.filter { it[0] in begin..end } }
                        fun percentile(column: Int, fraction: Double): Double {
                            val values = rows.map { it[column] }.sorted()
                            return if (values.isEmpty()) 0.0 else values[((values.size - 1) * fraction).toInt()] / 1_000_000.0
                        }
                        android.util.Log.i("ZtermMotion", "round=$round show=$show frames=${rows.size} compositions=$compositions updates=$updates " +
                            "layoutP50=${percentile(2, .5)} layoutP95=${percentile(2, .95)} drawP50=${percentile(3, .5)} drawP95=${percentile(3, .95)} totalP95=${percentile(1, .95)}")
                    }
                }
            } finally {
                scenario.onActivity {
                    it.getSystemService(InputMethodManager::class.java).hideSoftInputFromWindow(view.windowToken, 0)
                    it.window.removeOnFrameMetricsAvailableListener(listener)
                    stop?.invoke()
                }
                thread.quitSafely()
            }
        }
    }
}
