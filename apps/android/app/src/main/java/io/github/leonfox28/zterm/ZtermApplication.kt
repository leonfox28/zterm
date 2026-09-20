package io.github.leonfox28.zterm

import android.app.Application
import io.github.leonfox28.zterm.nativebridge.NativeRuntime

class ZtermApplication : Application() {
    // Activities and their collectors do not own or close the native runtime.
    internal val repository by lazy { AppRepository(this, runtime) }
    internal val updates by lazy { AppUpdates(AndroidUpdateSource(this), java.io.File(cacheDir, "updates"),
        { repository.state.value.saved.updateReminder }, repository::saveUpdateReminder) }
    val runtime: NativeRuntime by lazy { NativeRuntime() }
}
