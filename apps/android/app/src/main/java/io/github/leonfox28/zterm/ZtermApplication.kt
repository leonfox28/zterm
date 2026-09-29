package io.github.leonfox28.zterm

import android.app.Application
import io.github.leonfox28.zterm.nativebridge.NativeRuntime

class ZtermApplication : Application() {
    internal val diagnostics by lazy { AppDiagnostics(this) }
    override fun onCreate() {
        super.onCreate()
        diagnostics // Install before any identity/network initialization.
    }
    // Activities and their collectors do not own or close the native runtime.
    internal val repository by lazy { AppRepository(this, runtime, diagnostics) }
    internal val updates by lazy { AppUpdates(AndroidUpdateSource(this), java.io.File(cacheDir, "updates"),
        { repository.state.value.saved.updateReminder }, repository::saveUpdateReminder, diagnostics = { event, code -> diagnostics.record(event, code) }) }
    val runtime: NativeRuntime by lazy { diagnostics; NativeRuntime() }
}
