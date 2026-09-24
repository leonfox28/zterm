package io.github.leonfox28.zterm

import org.junit.Assert.assertEquals
import org.junit.Test

class TerminalTitleTest {
    @Test fun titleCombinesWithoutChangingManualName() {
        val manual = "开发会话"
        assertEquals("开发会话 · 编辑器", terminalDisplayTitle(manual, "编辑器"))
        assertEquals(manual, terminalDisplayTitle(manual, ""))
        assertEquals(manual, terminalDisplayTitle(manual, manual))
        assertEquals("重命名 · 编辑器", terminalDisplayTitle("重命名", "编辑器"))
    }
}
