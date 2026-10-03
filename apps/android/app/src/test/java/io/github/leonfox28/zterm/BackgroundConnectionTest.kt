package io.github.leonfox28.zterm

import org.junit.Assert.*
import org.junit.Test

class BackgroundConnectionPolicyTest {
    @Test fun backgroundServiceTracksConnectionLifetimeWithoutNotificationPermissionGate() {
        for (phase in listOf("active", "connecting", "synchronizing", "reconnecting")) {
            assertTrue(backgroundConnectionWanted(true, true, false, phase))
            assertFalse(backgroundConnectionWanted(false, true, false, phase))
            assertFalse(backgroundConnectionWanted(true, false, false, phase))
        }
        for (phase in listOf(null, "closed", "ended", "lease_lost")) {
            assertFalse(backgroundConnectionWanted(true, true, false, phase))
        }
        assertTrue(backgroundConnectionWanted(true, true, true, null))
        assertFalse(Preferences().keepBackgroundConnection)
        assertNull(SavedState().activeTerminal)
    }
}
