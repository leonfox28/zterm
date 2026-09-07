package io.github.leonfox28.zterm

import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import java.io.File
import java.security.KeyStore
import java.util.UUID
import org.junit.Assert.*
import org.junit.Test
import org.junit.runner.RunWith

@RunWith(AndroidJUnit4::class)
class IdentityStoreTest {
    private fun withStore(test: (AppStore, File, String) -> Unit) {
        val context = InstrumentationRegistry.getInstrumentation().targetContext
        val alias = "zterm.test.${UUID.randomUUID()}"
        val directory = File(context.cacheDir, alias)
        try { test(AppStore(context, directory, alias), directory, alias) }
        finally {
            directory.deleteRecursively()
            KeyStore.getInstance("AndroidKeyStore").apply { load(null); deleteEntry(alias) }
        }
    }
    @Test fun restartRetainsSeedAndAddressBookRemovalPreservesIdentity() = withStore { store, directory, alias ->
        val seed = store.loadOrCreateSeed()
        val host = SavedHost("a".repeat(64), "Mac", emptyList(), "b".repeat(32))
        store.save(SavedState(listOf(host), Preferences("zh", "dark", 16), RecentConnection(host.id, host.lastSession!!)))
        val context = InstrumentationRegistry.getInstrumentation().targetContext
        val reopened = AppStore(context, directory, alias)
        assertArrayEquals(seed, reopened.loadOrCreateSeed())
        assertEquals(host, reopened.load().hosts.single())
        reopened.save(reopened.load().copy(hosts = emptyList(), recent = null))
        assertTrue(reopened.load().hosts.isEmpty())
        assertArrayEquals(seed, reopened.loadOrCreateSeed())
        assertEquals(Preferences("zh", "dark", 16), reopened.load().preferences)
        seed.fill(0)
    }
    @Test fun corruptIdentityIsReportedAndNeverReplaced() = withStore { store, directory, _ ->
        store.loadOrCreateSeed().fill(0)
        val file = File(directory, "identity.json")
        file.writeText("broken identity")
        assertTrue(runCatching { store.loadOrCreateSeed() }.exceptionOrNull() is StoreFailure)
        assertEquals("broken identity", file.readText())
    }
    @Test fun ciphertextWithoutKeystoreKeyNeverCreatesANewIdentity() = withStore { store, directory, alias ->
        store.loadOrCreateSeed().fill(0)
        val before = File(directory, "identity.json").readBytes()
        KeyStore.getInstance("AndroidKeyStore").apply { load(null); deleteEntry(alias) }
        assertTrue(runCatching { store.loadOrCreateSeed() }.exceptionOrNull() is StoreFailure)
        assertArrayEquals(before, File(directory, "identity.json").readBytes())
    }
}
