package io.github.leonfox28.zterm

import androidx.test.ext.junit.runners.AndroidJUnit4
import io.github.leonfox28.zterm.nativebridge.NativeException
import io.github.leonfox28.zterm.nativebridge.NativeRuntime
import kotlinx.coroutines.CoroutineStart
import kotlinx.coroutines.async
import kotlinx.coroutines.cancelAndJoin
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.withTimeout
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith

@RunWith(AndroidJUnit4::class)
class NativeBridgeTest {
    @Test fun nativeLibraryLoadsAndAsyncStateCrossesJni() = runBlocking {
        NativeRuntime().use { runtime ->
            runtime.newOperation().use { operation ->
                val current = runtime.currentState()
                val received = withTimeout(5_000) { runtime.waitForState(0uL, operation) }
                assertEquals(BuildConfig.VERSION_NAME, received.version)
                assertEquals(current, received)
                assertTrue(received.generation > 0uL)
            }
        }
    }

    @Test fun explicitCancellationIsTypedAndLeavesRuntimeUsable() = runBlocking {
        NativeRuntime().use { runtime ->
            val before = runtime.currentState()
            runtime.newOperation().use { operation ->
                val waiting = async(start = CoroutineStart.UNDISPATCHED) {
                    try {
                        runtime.waitForState(before.generation, operation)
                        false
                    } catch (_: NativeException.Cancelled) { true }
                }
                assertFalse(waiting.isCompleted)
                operation.cancel()
                assertTrue(withTimeout(5_000) { waiting.await() })
            }
            runtime.newOperation().use { another ->
                assertEquals(before, withTimeout(5_000) { runtime.waitForState(0uL, another) })
            }
        }
    }

    @Test fun CoroutineDisposalDoesNotDestroyApplicationRuntime() = runBlocking {
        NativeRuntime().use { runtime ->
            val before = runtime.currentState()
            runtime.newOperation().use { operation ->
                val collector = async(start = CoroutineStart.UNDISPATCHED) {
                    runtime.waitForState(before.generation, operation)
                }
                withTimeout(5_000) { collector.cancelAndJoin() }
                assertEquals(before, runtime.currentState())
            }
        }
    }

    @Test fun shutdownUnblocksPendingObservationAndDisposalIsIdempotent() = runBlocking {
        val runtime = NativeRuntime()
        runtime.newOperation().use { operation ->
            val waiting = async(start = CoroutineStart.UNDISPATCHED) {
                try {
                    runtime.waitForState(runtime.currentState().generation, operation)
                    false
                } catch (_: NativeException.Closed) { true }
            }
            withTimeout(5_000) { runtime.shutdown(); runtime.shutdown() }
            assertTrue(withTimeout(5_000) { waiting.await() })
            assertTrue(runCatching { runtime.currentState() }.exceptionOrNull() is NativeException.Closed)
        }
        runtime.close()
        runtime.close()
        assertTrue(runtime.uniffiIsDestroyed)
    }
}
