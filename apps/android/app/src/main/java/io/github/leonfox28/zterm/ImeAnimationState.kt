package io.github.leonfox28.zterm

import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.compose.runtime.staticCompositionLocalOf
import androidx.core.view.WindowInsetsAnimationCompat
import androidx.core.view.WindowInsetsCompat

/** Window-level lifetime: Compose consumes animation dispatch below this owner. */
internal class ImeAnimationState : WindowInsetsAnimationCompat.Callback(DISPATCH_MODE_CONTINUE_ON_SUBTREE) {
    private val active = mutableSetOf<WindowInsetsAnimationCompat>()
    private val observers = linkedSetOf<(Boolean) -> Unit>()
    var running by mutableStateOf(false)
        private set

    fun observe(observer: (Boolean) -> Unit): () -> Unit {
        observers.add(observer)
        observer(running)
        return { observers.remove(observer) }
    }
    private fun publishRunning() {
        val next = active.isNotEmpty()
        if (running == next) return
        running = next
        // Geometry completion is an event. Compose may coalesce true/false
        // before recomposition, so the native View must observe it directly.
        observers.toList().forEach { it(next) }
    }

    override fun onPrepare(animation: WindowInsetsAnimationCompat) {
        if (animation.typeMask and WindowInsetsCompat.Type.ime() != 0) {
            active.add(animation)
            publishRunning()
        }
    }
    override fun onProgress(insets: WindowInsetsCompat, runningAnimations: MutableList<WindowInsetsAnimationCompat>): WindowInsetsCompat = insets
    override fun onEnd(animation: WindowInsetsAnimationCompat) {
        active.remove(animation)
        publishRunning()
    }
}

internal val LocalImeAnimation = staticCompositionLocalOf<ImeAnimationState> { error("Activity IME animation owner missing") }
