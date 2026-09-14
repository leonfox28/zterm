package io.github.leonfox28.zterm

import android.graphics.Canvas
import android.graphics.RenderNode
import android.os.Build
import androidx.annotation.RequiresApi
import io.github.leonfox28.zterm.nativebridge.NativeCell
import io.github.leonfox28.zterm.nativebridge.NativeRow

/** Bounded display lists for text only. Selection, cursor and preedit stay dynamic. */
internal class TerminalRowRenderer {
    private val hardware = if (Build.VERSION.SDK_INT >= 29) HardwareRows() else null
    /** Cumulative text display-list recordings, used by rendering/profile checks. */
    var recordingCount = 0L
        private set

    fun draw(canvas: Canvas, ordinal: Long, row: NativeRow, width: Int, height: Int,
             limit: Int, record: (Canvas) -> Unit) {
        if (Build.VERSION.SDK_INT >= 29 && canvas.isHardwareAccelerated) {
            if (hardware!!.draw(canvas, ordinal, row, width, height, limit, record)) recordingCount++
        } else record(canvas)
    }

    fun clear() {
        if (Build.VERSION.SDK_INT >= 29) hardware?.clear()
    }

    @RequiresApi(29)
    private class HardwareRows {
        // Ordinals locate a row; they do not identify its drawing. A resized TUI
        // can put identical text at a different ordinal after the local pan.
        private class Content(val cells: List<NativeCell>, val width: Int, val height: Int) {
            private val hash = (cells.hashCode() * 31 + width) * 31 + height
            override fun hashCode() = hash
            override fun equals(other: Any?) = this === other || other is Content &&
                width == other.width && height == other.height && cells == other.cells
        }
        private data class Entry(val content: Content, val node: RenderNode)
        private data class Binding(val row: NativeRow, val content: Content)
        private val entries = LinkedHashMap<Content, Entry>(16, .75f, true)
        // Avoid hashing every cell on warmed scrolling/animation frames. Both
        // this identity shortcut and the content cache have the same row bound.
        private val bindings = LinkedHashMap<Long, Binding>(16, .75f, true)

        fun draw(canvas: Canvas, ordinal: Long, row: NativeRow, width: Int, height: Int,
                 limit: Int, record: (Canvas) -> Unit): Boolean {
            val binding = bindings[ordinal]
            val content = if (binding?.row === row && binding.content.width == width && binding.content.height == height)
                binding.content else Content(row.cells, width, height)
            val entry = entries[content] ?: Entry(content, RenderNode("terminal-row")).also {
                it.node.setPosition(0, 0, width, height)
                // A display list alone still replays every glyph/background on
                // RenderThread. Retain this bounded, immutable row's pixels too.
                it.node.setUseCompositingLayer(true, null)
                entries[content] = it
            }
            val recorded = !entry.node.hasDisplayList()
            if (recorded) {
                val recording = entry.node.beginRecording(width, height)
                try { record(recording) } finally { entry.node.endRecording() }
            }
            if (binding?.row !== row || binding.content !== entry.content)
                bindings[ordinal] = Binding(row, entry.content)
            canvas.drawRenderNode(entry.node)
            while (entries.size > limit) {
                val iterator = entries.entries.iterator()
                iterator.next().value.node.discardDisplayList()
                iterator.remove()
            }
            while (bindings.size > limit) {
                val iterator = bindings.entries.iterator()
                iterator.next()
                iterator.remove()
            }
            return recorded
        }

        fun clear() {
            entries.values.forEach { it.node.discardDisplayList() }
            entries.clear()
            bindings.clear()
        }
    }
}
