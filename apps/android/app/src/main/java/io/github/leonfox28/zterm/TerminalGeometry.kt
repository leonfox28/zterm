package io.github.leonfox28.zterm

import kotlin.math.roundToInt

/** One-way allocation from the parent's available pixels, never the child's rounded height. */
internal fun terminalBottomRemainder(
    available: Int, cellHeight: Int, currentBottom: Int, startBottom: Int, endBottom: Int,
    animating: Boolean,
): Int {
    val height = available.coerceAtLeast(0)
    val cell = cellHeight.coerceAtLeast(1)
    if (height < cell) return 0 // Preserve the existing one-row clipped minimum.
    if (!animating || startBottom == endBottom) return height % cell
    val startHeight = (height + currentBottom - startBottom).coerceAtLeast(0)
    val endHeight = (height + currentBottom - endBottom).coerceAtLeast(0)
    val fraction = ((currentBottom - startBottom).toFloat() / (endBottom - startBottom)).coerceIn(0f, 1f)
    val startRemainder = if (startHeight < cell) 0 else startHeight % cell
    val endRemainder = if (endHeight < cell) 0 else endHeight % cell
    return (startRemainder * (1f - fraction) + endRemainder * fraction)
        .roundToInt().coerceIn(0, height)
}

/** Cursor-preserving clip/pan; integral grid endpoints make the host handoff exact. */
internal fun terminalPan(height: Float, cellHeight: Float, rows: Int, cursorRow: Int, history: Long): Float {
    val difference = height - rows * cellHeight
    val minimum = minOf(0f, height - (cursorRow + 1) * cellHeight)
    return difference.coerceIn(minimum, history * cellHeight)
}

/** Committed together with the drawn semantic source. Pending layout cannot change hit tests. */
internal data class DrawnTerminalGeometry(
    val cellWidth: Float, val cellHeight: Float, val baseline: Float, val shift: Float,
    val width: Int, val height: Int, val screenX: Int, val screenY: Int, val version: Long,
)
