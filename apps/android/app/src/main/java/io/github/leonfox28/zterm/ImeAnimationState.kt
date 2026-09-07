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
    var running by mutableStateOf(false)
        private set

    override fun onPrepare(animation: WindowInsetsAnimationCompat) {
        if (animation.typeMask and WindowInsetsCompat.Type.ime() != 0) {
            active.add(animation)
            running = true
        }
    }
    override fun onProgress(insets: WindowInsetsCompat, runningAnimations: MutableList<WindowInsetsAnimationCompat>): WindowInsetsCompat = insets
    override fun onEnd(animation: WindowInsetsAnimationCompat) {
        active.remove(animation)
        running = active.isNotEmpty()
    }
}

internal val LocalImeAnimation = staticCompositionLocalOf<ImeAnimationState> { error("Activity IME animation owner missing") }
