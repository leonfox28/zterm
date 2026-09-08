package io.github.leonfox28.zterm

import android.graphics.Canvas
import android.graphics.RenderNode
import android.os.Build
import androidx.annotation.RequiresApi
import io.github.leonfox28.zterm.nativebridge.NativeRow

/** Bounded display lists for text only. Selection, cursor and preedit stay dynamic. */
internal class TerminalRowRenderer {
    private val hardware = if (Build.VERSION.SDK_INT >= 29) HardwareRows() else null

    fun draw(canvas: Canvas, ordinal: Long, row: NativeRow, width: Int, height: Int,
             limit: Int, record: (Canvas) -> Unit) {
        if (Build.VERSION.SDK_INT >= 29 && canvas.isHardwareAccelerated) {
            hardware!!.draw(canvas, ordinal, row, width, height, limit, record)
        } else record(canvas)
    }

    fun clear() {
        if (Build.VERSION.SDK_INT >= 29) hardware?.clear()
    }

    @RequiresApi(29)
    private class HardwareRows {
        private data class Entry(var row: NativeRow, val node: RenderNode)
        private val entries = LinkedHashMap<Long, Entry>(16, .75f, true)

        fun draw(canvas: Canvas, ordinal: Long, row: NativeRow, width: Int, height: Int,
                 limit: Int, record: (Canvas) -> Unit) {
            val entry = entries[ordinal] ?: Entry(row, RenderNode("terminal-row")).also {
                it.node.setPosition(0, 0, width, height)
                entries[ordinal] = it
            }
            if (!entry.node.hasDisplayList() || (entry.row !== row && entry.row != row)) {
                val recording = entry.node.beginRecording(width, height)
                try { record(recording) } finally { entry.node.endRecording() }
            }
            entry.row = row
            canvas.drawRenderNode(entry.node)
            while (entries.size > limit) {
                val iterator = entries.entries.iterator()
                iterator.next().value.node.discardDisplayList()
                iterator.remove()
            }
        }

        fun clear() {
            entries.values.forEach { it.node.discardDisplayList() }
            entries.clear()
        }
    }
}
