package io.github.leonfox28.zterm

import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.activity.enableEdgeToEdge
import androidx.compose.runtime.CompositionLocalProvider
import androidx.core.view.ViewCompat

class MainActivity : ComponentActivity() {
    override fun onStart() {
        super.onStart()
        (application as ZtermApplication).repository.activityStarted()
        (application as ZtermApplication).diagnostics.record(io.github.leonfox28.zterm.nativebridge.AppDiagnostic.FOREGROUND)
    }
    override fun onStop() {
        (application as ZtermApplication).diagnostics.record(io.github.leonfox28.zterm.nativebridge.AppDiagnostic.BACKGROUND)
        (application as ZtermApplication).repository.activityStopped()
        super.onStop()
    }
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        enableEdgeToEdge()
        val imeAnimation = ImeAnimationState()
        ViewCompat.setWindowInsetsAnimationCallback(window.decorView, imeAnimation)
        val repository = (application as ZtermApplication).repository
        val updates = (application as ZtermApplication).updates
        setContent { CompositionLocalProvider(LocalImeAnimation provides imeAnimation) { ZtermApp(repository, updates) } }
    }
}
