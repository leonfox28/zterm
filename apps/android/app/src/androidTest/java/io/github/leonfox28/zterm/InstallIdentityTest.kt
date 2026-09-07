package io.github.leonfox28.zterm

import androidx.test.platform.app.InstrumentationRegistry
import io.github.leonfox28.zterm.nativebridge.NativeRuntime
import kotlinx.coroutines.runBlocking
import org.json.JSONObject
import org.junit.Assert.*
import org.junit.Assume.assumeTrue
import org.junit.Test
import java.io.File

class InstallIdentityTest {
    @Test fun identityAndHostsSurviveAnExplicitSignedUpdate() = runBlocking {
        val arguments = InstrumentationRegistry.getArguments()
        assumeTrue("explicit install/update acceptance", arguments.getString("installEvidence") == "1")
        val context = InstrumentationRegistry.getInstrumentation().targetContext
        val store = AppStore(context)
        val seed = store.loadOrCreateSeed()
        val saved = store.load()
        try {
            NativeRuntime().use { runtime ->
                val publicId = runtime.initialize(seed,PlatformNetwork(context) {}.current(),saved.hosts.map { it.native() })
                arguments.getString("expectedIdentity")?.let { assertEquals("same controller identity after update",it,publicId) }
                arguments.getString("expectedHosts")?.let { assertEquals("known hosts survive update",it.toInt(),saved.hosts.size) }
                File(context.cacheDir,"install-evidence.json").writeText(JSONObject()
                    .put("publicIdentity",publicId).put("hosts",saved.hosts.size)
                    .put("language",saved.preferences.language).put("theme",saved.preferences.theme).toString())
            }
        } finally { seed.fill(0) }
    }
}
