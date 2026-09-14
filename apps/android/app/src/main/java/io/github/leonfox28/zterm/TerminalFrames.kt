package io.github.leonfox28.zterm

import android.view.Choreographer
import io.github.leonfox28.zterm.nativebridge.NativeFrame
import io.github.leonfox28.zterm.nativebridge.NativeFrameSource
import io.github.leonfox28.zterm.nativebridge.NativeRow
import io.github.leonfox28.zterm.nativebridge.NativeRowUpdate
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.channels.Channel
import kotlinx.coroutines.currentCoroutineContext
import kotlinx.coroutines.ensureActive
import kotlinx.coroutines.withContext

/** Main-owned display gate. Hiding never leaves final native state waiting for vsync. */
internal class TerminalFrameClock {
    private val ticks = Channel<Unit>(Channel.CONFLATED)
    private val callback = Choreographer.FrameCallback { ticks.trySend(Unit) }
    var visible = false
        set(value) {
            field = value
            if (!value) ticks.trySend(Unit)
        }

    suspend fun awaitFrame() {
        if (!visible) return
        ticks.tryReceive()
        val clock = Choreographer.getInstance()
        clock.postFrameCallback(callback)
        try { ticks.receive() } finally { clock.removeFrameCallback(callback) }
    }
}

/** One paired source/list baseline, independent of Repository and View source ownership. */
internal class TerminalFrameProjection : AutoCloseable {
    private var source: NativeFrameSource? = null
    private var generation = 0uL
    private var rows: List<NativeRow> = emptyList()

    suspend fun resolve(next: NativeFrame): NativeFrame {
        val candidate = next.source ?: return next.also { close() }
        try {
            if (source == null || generation != next.contentGeneration) {
                val resolved = withContext(Dispatchers.Default) {
                    candidate.presentationRowsFrom(source).map { update ->
                        when (update) {
                            is NativeRowUpdate.Reuse -> rows[update.index.toInt()]
                            is NativeRowUpdate.Replace -> update.row
                        }
                    }
                }
                val held = candidate.retained()
                source?.close()
                source = held
                generation = next.contentGeneration
                rows = resolved
            }
            return next.copy(rows = rows)
        } catch (error: Throwable) { candidate.close(); throw error }
    }

    override fun close() {
        source?.close(); source = null
        rows = emptyList(); generation = 0uL
    }
}

/** Consume already installed complete frames; the native actor alone applies/ACKs wire updates. */
internal suspend fun collectTerminalFrames(
    current: () -> NativeFrame,
    wait: suspend (ULong) -> NativeFrame,
    clock: TerminalFrameClock,
    deliver: (NativeFrame) -> Unit,
) {
    TerminalFrameProjection().use { projection ->
        var previous: NativeFrame? = null
        var pending: NativeFrame? = current()
        try {
            while (true) {
                currentCoroutineContext().ensureActive()
                val candidate = checkNotNull(pending)
                if (previous != null && candidate.canPaceAfter(previous)) {
                    // Release the skipped source before waiting. Re-read at the
                    // display boundary, including a final frame after actor closure.
                    candidate.source?.close(); pending = null
                    clock.awaitFrame()
                    pending = current()
                }
                val next = checkNotNull(pending)
                pending = null // resolve owns cleanup on failure; delivery takes ownership on success.
                val resolved = projection.resolve(next)
                deliver(resolved)
                previous = resolved
                pending = wait(resolved.generation)
            }
        } finally { pending?.source?.close() }
    }
}

private fun NativeFrame.canPaceAfter(previous: NativeFrame): Boolean =
    contentGeneration != previous.contentGeneration &&
        state == "active" && previous.state == "active" &&
        inputReady == previous.inputReady && inputEpoch == previous.inputEpoch &&
        geometryGeneration == previous.geometryGeneration && activeScreen == previous.activeScreen &&
        pointerMode == previous.pointerMode && selection == previous.selection &&
        historyOffset == previous.historyOffset && notice == previous.notice && error == previous.error
