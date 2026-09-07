package io.github.leonfox28.zterm

import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import io.github.leonfox28.zterm.nativebridge.NativeException
import io.github.leonfox28.zterm.nativebridge.NativeRuntime
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.withTimeout
import org.junit.Assert.*
import org.junit.Assume.assumeTrue
import org.junit.Test
import org.junit.runner.RunWith
import java.io.File

@RunWith(AndroidJUnit4::class)
class NativeNetworkTest {
    @Test fun explicitShutdownAllowsTheSameControllerToReconnectImmediately() = runBlocking {
        val context = InstrumentationRegistry.getInstrumentation().targetContext
        val store = AppStore(context)
        val host = store.load().hosts.firstOrNull { it.name == "my-mac" }
        assumeTrue("explicitly paired acceptance host",host != null)
        val acceptedHost = requireNotNull(host)
        val seed = store.loadOrCreateSeed()
        val dns = PlatformNetwork(context) {}.current()
        try {
            repeat(4) {
                NativeRuntime().use { runtime ->
                    try {
                        withTimeout(15_000) { runtime.initialize(seed,dns,listOf(acceptedHost.native())) }
                        withTimeout(15_000) { runtime.listSessions(acceptedHost.id) }
                    } finally { withTimeout(5_000) { runtime.shutdown() } }
                }
            }
        } finally { seed.fill(0) }
    }

    @Test fun realHostWelcomeAndExplicitDenialRemainDistinct() = runBlocking {
        val context = InstrumentationRegistry.getInstrumentation().targetContext
        val store = AppStore(context)
        val saved = store.load()
        val host = saved.hosts.firstOrNull { it.name == "my-mac" }
        assumeTrue("explicitly paired acceptance host", host != null)
        val acceptedHost = requireNotNull(host)
        val dns = PlatformNetwork(context) {}.current()
        val seed = store.loadOrCreateSeed()
        val unknownSeed = ByteArray(32).also { java.security.SecureRandom().nextBytes(it) }
        try {
            NativeRuntime().use { known ->
                try {
                    withTimeout(15_000) { known.initialize(seed,dns,listOf(acceptedHost.native())) }
                    val sessions = withTimeout(15_000) { known.listSessions(acceptedHost.id) }
                    assertTrue(sessions.all { it.sessionId.length == 32 })
                    assertNotNull("Welcome establishes an authenticated connection",known.connectionInfo(acceptedHost.id))
                } finally { withTimeout(5_000) { known.shutdown() } }
            }
            // A disposable controller identity is deliberately not paired.
            // Never revoke or mutate the real App identity/authorization.
            NativeRuntime().use { unknown ->
                try {
                    withTimeout(15_000) { unknown.initialize(unknownSeed,dns,listOf(acceptedHost.native())) }
                    val failure = runCatching { withTimeout(15_000) { unknown.listSessions(acceptedHost.id) } }.exceptionOrNull()
                    assertTrue("host explicitly rejects the unknown controller",failure is NativeException.RequestFailed)
                    assertEquals("unauthorized",(failure as NativeException.RequestFailed).code)
                    assertNull("rejected handshake cannot be cached as authorized",unknown.connectionInfo(acceptedHost.id))
                } finally { withTimeout(5_000) { unknown.shutdown() } }
            }
        } finally { seed.fill(0); unknownSeed.fill(0) }
        assertEquals("handshake tests preserve saved hosts",saved,store.load())
    }

    @Test fun endpointLoadsWithSystemDnsAndRejectsUnknownHosts() = runBlocking {
        val context = InstrumentationRegistry.getInstrumentation().targetContext
        val dns = PlatformNetwork(context) {}.current()
        assertTrue("emulator has system DNS", dns.isNotEmpty())
        NativeRuntime().use { runtime ->
            val seed = ByteArray(32) { (it + 1).toByte() }
            val identity = withTimeout(15_000) { runtime.initialize(seed, dns, emptyList()) }
            assertEquals(64, identity.length)
            assertEquals(identity, runtime.initialize(seed, dns, emptyList()))
            val other = ByteArray(32) { (it + 2).toByte() }
            val mismatch = runCatching { runtime.initialize(other, dns, emptyList()) }.exceptionOrNull()
            assertTrue(mismatch is NativeException.RequestFailed)
            assertEquals("identity_state_mismatch", (mismatch as NativeException.RequestFailed).code)
            val unknown = runCatching { runtime.listSessions("11".repeat(32)) }.exceptionOrNull()
            assertTrue(unknown is NativeException.RequestFailed)
            seed.fill(0)
            other.fill(0)
        }
    }

    /** Explicit real-host acceptance fixture, never generated or logged by this test. */
    @Test fun pairAndListOnRealHost() = runBlocking {
        val context = InstrumentationRegistry.getInstrumentation().targetContext
        val fixture = File(context.filesDir, "pairing-fixture.txt")
        assumeTrue("real-host ticket fixture was explicitly installed", fixture.isFile)
        val store = AppStore(context)
        val seed = store.loadOrCreateSeed()
        val saved = store.load()
        try {
            NativeRuntime().use { runtime ->
                val identity = withTimeout(15_000) { runtime.initialize(seed, PlatformNetwork(context) {}.current(), saved.hosts.map { it.native() }) }
                assertEquals(64, identity.length)
                val ticket = fixture.readText().trim()
                fixture.delete()
                withTimeout(25_000) { runtime.pairTicket(ticket) }.use { pairing ->
                    val result = pairing.host()
                    assertNotEquals(identity, result.deviceId)
                    val host = SavedHost(result.deviceId, result.name, result.relayUrls)
                    store.save(saved.copy(hosts = saved.hosts.filterNot { it.id == host.id } + host))
                    runtime.commitPairing(pairing)
                    val sessions = withTimeout(10_000) { runtime.listSessions(host.id) }
                    assertTrue(sessions.all { it.sessionId.length == 32 && it.name.isNotBlank() })
                    assertEquals(host, store.load().hosts.first { it.id == host.id })
                }
            }
        } finally { seed.fill(0); fixture.delete() }
    }
}
