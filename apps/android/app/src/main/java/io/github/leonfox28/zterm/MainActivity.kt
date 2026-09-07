package io.github.leonfox28.zterm

import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.activity.enableEdgeToEdge
import androidx.compose.runtime.CompositionLocalProvider
import androidx.core.view.ViewCompat

class MainActivity : ComponentActivity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        enableEdgeToEdge()
        val imeAnimation = ImeAnimationState()
        ViewCompat.setWindowInsetsAnimationCallback(window.decorView, imeAnimation)
        val repository = (application as ZtermApplication).repository
        setContent { CompositionLocalProvider(LocalImeAnimation provides imeAnimation) { ZtermApp(repository) } }
    }
}
