package io.github.leonfox28.zterm

import android.content.Context
import android.graphics.Canvas
import android.graphics.Paint
import android.graphics.Typeface
import android.graphics.Matrix
import android.graphics.Rect
import android.graphics.Path
import android.view.ActionMode
import android.view.Menu
import android.view.MenuItem
import android.widget.OverScroller
import android.text.Editable
import android.text.SpannableStringBuilder
import android.text.InputType
import android.view.GestureDetector
import android.view.KeyEvent
import android.view.MotionEvent
import android.view.View
import android.view.inputmethod.BaseInputConnection
import android.view.inputmethod.CursorAnchorInfo
import android.view.inputmethod.EditorInfo
import android.view.inputmethod.InputConnection
import android.view.inputmethod.InputMethodManager
import androidx.core.view.OneShotPreDrawListener
import io.github.leonfox28.zterm.nativebridge.*
import kotlin.math.floor
import kotlin.math.max

/** Pure semantic Canvas renderer and native IME endpoint. There is no ANSI parser here. */
internal class TerminalView(context: Context) : View(context) {
    var repository: AppRepository? = null
        set(value) {
            if (field === value) return
            unobserve?.invoke(); unobserve = null; field = value
            observeFrames()
        }
    private var unobserve: (() -> Unit)? = null
    private var drawnFrame: NativeFrame? = null
    private var drawnGeometry: DrawnTerminalGeometry? = null
    private var geometryVersion = 0L
    var onCellHeightChanged: (Int) -> Unit = {}
    val gridCellHeight: Int get() = cellHeight.toInt()
    private var drawnSource: NativeFrameSource? = null
    private var pendingSource: NativeFrameSource? = null
    private var disposing = false
    var pendingModifiers: () -> Int = { 0 }
    var consumedModifiers: () -> Unit = {}
    private var current: NativeFrame? = null
    private val paint = Paint(Paint.ANTI_ALIAS_FLAG).apply { typeface = Typeface.MONOSPACE }
    private val rowRenderer = TerminalRowRenderer()
    private val underlinePath = Path()
    private val dotted = android.graphics.DashPathEffect(floatArrayOf(resources.displayMetrics.density,resources.displayMetrics.density*2),0f)
    private val dashed = android.graphics.DashPathEffect(floatArrayOf(resources.displayMetrics.density*3,resources.displayMetrics.density*2),0f)
    private var font = 12
    private var cellWidth = 1f
    private var cellHeight = 1f
    private var baseline = 1f
    private val composing = SpannableStringBuilder()
    private var ime: TerminalInputConnection? = null
    private val manager = context.getSystemService(InputMethodManager::class.java)
    private var lastSize: Pair<Int,Int>? = null
    var imeAnimating: () -> Boolean = { false }
    private var animatedInsets = false
    private var imeMeasurePending = false
    fun updateImeAnimation(running: Boolean) {
        if (animatedInsets == running) return
        animatedInsets = running
        // The last animated layout can already have the final height. Native
        // requestLayout alone need not yield another AndroidView layout in
        // Compose, so commit explicitly after the final layout, before drawing.
        if (!running) {
            imeMeasurePending = true
            OneShotPreDrawListener.add(this) {
                imeMeasurePending = false
                measureGrid()
                updateAnchor()
            }
            invalidate()
        }
    }
    private val scroller = OverScroller(context)
    private var scrollPixels = 0f
    private var lastScrollTarget: Long? = null
    private var scrollRequestGeneration = 0L
    private var waitingEdge = 0
    private var actionMode: ActionMode? = null
    private var draggingHandle: Boolean? = null
    private var touchX = 0f
    private var touchY = 0f
    private var childGesture = false
    private var gestureSource: NativeFrameSource? = null
    private var gestureMode = NativePointerMode.NONE
    private var gestureRow = 0
    private var gestureColumn = 0
    private var wheelPixels = 0f
    private fun releaseGestureSource() { gestureSource?.close(); gestureSource = null }
    private val handleRadius get() = 10f * resources.displayMetrics.density
    private val handles = context.obtainStyledAttributes(R.styleable.TerminalSelection).let { values ->
        try { listOf(values.getDrawable(R.styleable.TerminalSelection_android_textSelectHandleLeft), values.getDrawable(R.styleable.TerminalSelection_android_textSelectHandleRight)).map { it?.mutate()?.apply { setTint(0xFF7CA66A.toInt()) } } }
        finally { values.recycle() }
    }
    private val autoScroll = object : Runnable {
        override fun run() {
            if (draggingHandle == null || !isShown) return
            val edge = 32f * resources.displayMetrics.density
            val direction = when { touchY < edge -> 1f; touchY > height-edge -> -1f; else -> 0f }
            if (direction != 0f) { moveScroll(scrollPixels + direction * cellHeight * .45f); extendHandle() }
            postOnAnimation(this)
        }
    }
    private val gesture = GestureDetector(context, object : GestureDetector.SimpleOnGestureListener() {
        override fun onDown(event: MotionEvent): Boolean {
            scroller.forceFinished(true); releaseGestureSource(); wheelPixels = 0f
            gestureMode = if (scrollPixels == 0f) drawnFrame?.pointerMode ?: NativePointerMode.NONE else NativePointerMode.NONE
            childGesture = gestureMode != NativePointerMode.NONE && !imeAnimating()
            if (childGesture) {
                gestureSource = drawnSource?.retained()
                val point = hit(event.x,event.y); gestureRow = point.first; gestureColumn = point.second
            }
            return true
        }
        override fun onScroll(first: MotionEvent?, current: MotionEvent, distanceX: Float, distanceY: Float): Boolean {
            if (childGesture) {
                wheelPixels -= distanceY
                val steps = (wheelPixels / cellHeight).toInt().coerceIn(-32,32)
                if (steps != 0) {
                    gestureSource?.let { repository?.pointer(it,gestureRow,gestureColumn,steps) }
                    wheelPixels -= steps * cellHeight
                }
                return true
            }
            moveScroll(scrollPixels-distanceY); return true
        }
        override fun onFling(first: MotionEvent?, second: MotionEvent, velocityX: Float, velocityY: Float): Boolean {
            if (childGesture) return true
            val maximum = ((current?.historyMaximum?.toLong() ?: 0) * cellHeight).toInt()
            scroller.fling(0,scrollPixels.toInt(),0,velocityY.toInt(),0,0,0,maximum)
            postInvalidateOnAnimation(); return true
        }
        override fun onLongPress(event: MotionEvent) {
            releaseGestureSource(); childGesture = false
            val frame = drawnFrame ?: return
            val source = drawnSource ?: return
            if (frame.state != "active") return
            val (row,column) = hit(event.x,event.y)
            repository?.select(source,row,column)
            performHapticFeedback(android.view.HapticFeedbackConstants.LONG_PRESS)
        }
        override fun onSingleTapUp(event: MotionEvent): Boolean {
            performClick()
            requestFocus()
            if (current?.selection != null) { repository?.clearSelection(); return true }
            if (childGesture && gestureMode == NativePointerMode.MOUSE) {
                gestureSource?.let { repository?.pointer(it,gestureRow,gestureColumn) }
            }
            return true
        }
    }).apply {
        // Every completed tap belongs to the child, including rapid taps.
        // The default double-tap listener would swallow the second tap.
        setOnDoubleTapListener(null)
    }
    init {
        isFocusable = true; isFocusableInTouchMode = true
        isVerticalScrollBarEnabled = true; isScrollbarFadingEnabled = true
        scrollBarStyle = SCROLLBARS_INSIDE_OVERLAY
        importantForAutofill = IMPORTANT_FOR_AUTOFILL_NO_EXCLUDE_DESCENDANTS
        contentDescription = context.getString(R.string.app_name)
        updateMetrics()
    }
    fun update(frame: NativeFrame?, size: Int) {
        if (font != size) { font = size; updateMetrics(); requestLayout(); invalidate() }
        observeFrames()
        if (unobserve == null) acceptFrame(frame)
    }
    private fun observeFrames() {
        if (unobserve == null) unobserve = repository?.observeTerminalFrames(::acceptFrame)
    }
    private fun acceptFrame(frame: NativeFrame?) {
        if (current === frame) return
        val previous = current
        val epochChanged = previous?.inputEpoch != frame?.inputEpoch
        val geometryChanged = previous?.geometryGeneration != frame?.geometryGeneration
        val growth = if (previous != null && frame != null) frame.historyMaximum.toLong() - previous.historyMaximum.toLong() else 0L
        val returningLive = repository?.scrollIntent == 0L && frame?.historyOffset == 0uL && frame.cursorVisible && frame.selection == null &&
            (waitingEdge == 0 || repository?.scrollRequestGeneration != scrollRequestGeneration)
        if (epochChanged || geometryChanged) {
            scroller.forceFinished(true)
            scrollPixels = (frame?.historyOffset?.toLong() ?: 0) * cellHeight
            lastScrollTarget = null; waitingEdge = 0
            invalidateCoordinates()
            rowRenderer.clear()
        } else if ((scrollPixels > 0f || waitingEdge > 0) && growth > 0) {
            // Preserve the logical reading position as live output appends.
            scrollPixels += growth * cellHeight
            scroller.forceFinished(true)
            lastScrollTarget = null
        }
        if (returningLive) { scrollPixels = 0f; scroller.forceFinished(true) }
        if (epochChanged) {
            composing.clear(); consumedModifiers(); manager.restartInput(this)
        }
        repository?.let { owner ->
            val intent = owner.scrollIntent
            if (owner.scrollRequestGeneration != scrollRequestGeneration && intent != null &&
                frame?.historyOffset?.toLong() == intent) {
                scrollPixels = intent * cellHeight
                scrollRequestGeneration = owner.scrollRequestGeneration
                scroller.forceFinished(true); waitingEdge = 0; lastScrollTarget = null
            }
        }
        // A delayed response after reversal may no longer cover the View. Keep
        // its still-valid rows/source until a response covers the actual position.
        val retainedWindow = if (!epochChanged && !geometryChanged && !returningLive &&
            frame?.state == "active" && previous?.state == "active") {
            previous.copy(windowOffset = (previous.windowOffset.toLong() + growth.coerceAtLeast(0)).toULong())
                .takeIf { scrollPixels !in scrollBounds(frame) && scrollPixels in scrollBounds(it) }
        } else null
        if (retainedWindow != null) {
            if (pendingSource == null) pendingSource = drawnSource?.retained()
            current = frame!!.copy(rows = retainedWindow.rows, firstRow = retainedWindow.firstRow,
                windowOffset = retainedWindow.windowOffset, contentGeneration = retainedWindow.contentGeneration,
                background = retainedWindow.background, cursorVisible = false, source = null)
        } else {
            pendingSource?.close(); pendingSource = frame?.source?.retained()
            current = frame
        }
        if (frame == null) {
            drawnSource?.close(); drawnSource = null; drawnFrame = null; drawnGeometry = null
            rowRenderer.clear()
        }
        current?.let { shown ->
            scrollPixels = scrollPixels.coerceIn(scrollBounds(shown))
            if (waitingEdge != 0 && previous != null && frame != null) {
                val before = scrollBounds(previous)
                val after = scrollBounds(frame)
                if (waitingEdge > 0 && after.endInclusive > before.endInclusive + growth.coerceAtLeast(0) * cellHeight ||
                    waitingEdge < 0 && after.start < before.start + growth.coerceAtLeast(0) * cellHeight) {
                    val older = waitingEdge > 0
                    waitingEdge = 0
                    submitScroll(kotlin.math.ceil(scrollPixels / cellHeight).toLong(), older)
                }
            }
        }
        if (gestureMode != frame?.pointerMode && childGesture) invalidateCoordinates()
        if (frame?.selection != null && actionMode == null) showSelectionActions()
        if (frame?.selection == null) { val old = actionMode; actionMode = null; old?.finish() }
        actionMode?.invalidateContentRect()
        updateAnchor()
        postInvalidateOnAnimation()
    }
    private fun updateMetrics() {
        val rows = scrollPixels / cellHeight
        rowRenderer.clear()
        paint.textSize = android.util.TypedValue.applyDimension(android.util.TypedValue.COMPLEX_UNIT_SP, font.toFloat(), resources.displayMetrics)
        cellWidth = paint.measureText("M").coerceAtLeast(1f)
        val metrics = paint.fontMetrics
        cellHeight = kotlin.math.ceil((metrics.descent - metrics.ascent).toDouble()).toFloat().coerceAtLeast(1f)
        baseline = -metrics.ascent
        scrollPixels = rows * cellHeight
        invalidateCoordinates()
        onCellHeightChanged(gridCellHeight)
    }
    private fun invalidateCoordinates() {
        ++geometryVersion
        repository?.retireGeometry()
        releaseGestureSource(); childGesture = false; draggingHandle = null
        removeCallbacks(autoScroll)
    }
    override fun onSizeChanged(w: Int, h: Int, oldw: Int, oldh: Int) { rowRenderer.clear(); invalidateCoordinates(); measureGrid() }
    override fun onLayout(changed: Boolean, left: Int, top: Int, right: Int, bottom: Int) { measureGrid() }
    private fun measureGrid() {
        if (width <= 0 || height <= 0 || animatedInsets || imeMeasurePending || imeAnimating()) return
        val size = max(1, floor(height / cellHeight).toInt()) to max(1, floor(width / cellWidth).toInt())
        if (size != lastSize) { lastSize = size; repository?.measure(size.first, size.second) }
    }
    override fun onDraw(canvas: Canvas) {
        val frame = current ?: return
        canvas.clipRect(0, 0, width, height)
        val offset = kotlin.math.ceil(scrollPixels / cellHeight).toLong()
        val firstRow = frame.firstRow + frame.windowOffset.toLong() - offset
        val shift = geometryShift(frame) + scrollPixels - offset * cellHeight
        val location = IntArray(2); getLocationOnScreen(location)
        val geometry = DrawnTerminalGeometry(cellWidth, cellHeight, baseline, shift, width, height,
            location[0], location[1], geometryVersion, firstRow)
        canvas.drawColor(frame.background.toInt())
        canvas.save(); canvas.translate(0f, shift)
        frame.rows.forEachIndexed { row, line ->
            val logical = frame.firstRow + row
            val top = (logical - geometry.firstRow) * cellHeight
            if (top + shift >= height || top + cellHeight + shift <= 0) return@forEachIndexed
            canvas.save(); canvas.translate(0f, top)
            rowRenderer.draw(canvas, logical, line, width, cellHeight.toInt(), frame.viewport.rows.toInt() * 3 + 1) {
                drawRow(it, line)
            }
            canvas.restore()
        }
        drawSelection(canvas, frame, geometry)
        val cursorX = frame.cursorColumn.toInt() * cellWidth
        val cursorY = (frame.firstRow + frame.windowOffset.toLong() + frame.cursorRow.toInt() - firstRow) * cellHeight
        if (frame.cursorVisible && scrollPixels == 0f) {
            paint.color = frame.cursorColor.toInt(); paint.alpha = 130
            canvas.drawRect(cursorX,cursorY,cursorX+cellWidth,cursorY+cellHeight,paint); paint.alpha = 255
        }
        if (composing.isNotEmpty() && scrollPixels == 0f) {
            paint.color = frame.background.toInt()
            val extent = paint.measureText(composing.toString())
            canvas.drawRect(cursorX,cursorY,cursorX+extent,cursorY+cellHeight,paint)
            paint.color = frame.cursorColor.toInt()
            canvas.drawText(composing.toString(),cursorX,cursorY+baseline,paint)
            canvas.drawLine(cursorX,cursorY+cellHeight-1,cursorX+extent,cursorY+cellHeight-1,paint)
        }
        canvas.restore()
        if (drawnFrame !== frame || drawnGeometry?.firstRow != firstRow) {
            if (drawnGeometry?.firstRow != firstRow) repository?.retireGeometry()
            val base = pendingSource ?: drawnSource
            val source = try { base?.viewportSource(firstRow) } catch (_: NativeException.RequestFailed) { null }
            drawnSource?.close(); drawnSource = source
            pendingSource?.close(); pendingSource = null
        }
        drawnFrame = frame
        drawnGeometry = geometry
        updateAnchor()
        actionMode?.invalidateContentRect()
        if (draggingHandle != null) post { extendHandle() }
    }
    private fun drawRow(canvas: Canvas, line: NativeRow) {
        val top = 0f
        line.cells.forEachIndexed { column, cell ->
            if (cell.width == 0.toUByte()) return@forEachIndexed
            val x = column * cellWidth
            val right = x + cellWidth * cell.width.toInt()
            if (x >= width) return@forEachIndexed
            paint.color = cell.background.toInt(); paint.alpha = 255
            canvas.drawRect(x, top, right, top + cellHeight, paint)
            paint.color = cell.foreground.toInt()
            paint.alpha = if (cell.attributes.toInt() and 2 != 0) 150 else 255
            paint.isFakeBoldText = cell.attributes.toInt() and 1 != 0
            paint.textSkewX = if (cell.attributes.toInt() and 4 != 0) -.2f else 0f
            if (cell.text.isNotEmpty()) {
                canvas.save(); canvas.clipRect(x, top, right, top + cellHeight)
                canvas.drawText(cell.text, x, top + baseline, paint); canvas.restore()
            }
            paint.isFakeBoldText = false; paint.textSkewX = 0f; paint.alpha = 255
            if (cell.underline.toInt() != 0) {
                paint.color = cell.underlineColor.toInt(); paint.strokeWidth = resources.displayMetrics.density
                val y = top + cellHeight - paint.strokeWidth
                when (cell.underline.toInt()) {
                    3 -> {
                        val path = underlinePath.apply {
                            reset()
                            moveTo(x,y)
                            var position = x
                            while (position < right) {
                                val end = minOf(position+4*paint.strokeWidth,right)
                                quadTo((position+end)/2, y-3*paint.strokeWidth,end,y)
                                position = end
                            }
                        }
                        paint.style = Paint.Style.STROKE; canvas.drawPath(path,paint); paint.style = Paint.Style.FILL
                    }
                    4, 5 -> {
                        paint.pathEffect = if (cell.underline.toInt() == 4) dotted else dashed
                        canvas.drawLine(x,y,right,y,paint); paint.pathEffect = null
                    }
                    else -> {
                        canvas.drawLine(x,y,right,y,paint)
                        if (cell.underline.toInt() == 2) canvas.drawLine(x,y-2*paint.strokeWidth,right,y-2*paint.strokeWidth,paint)
                    }
                }
            }
        }
    }
    private fun geometryShift(frame: NativeFrame): Float {
        if (scrollPixels != 0f || frame.historyOffset != 0uL || frame.selection != null || !frame.cursorVisible) return 0f
        if (frame.viewport.columns.toInt() != floor(width / cellWidth).toInt().coerceIn(1, 240)) return 0f
        return terminalPan(minOf(height.toFloat(), 80 * cellHeight), cellHeight, frame.viewport.rows.toInt(), frame.cursorRow.toInt(), frame.historyMaximum.toLong())
    }
    private fun scrollBounds(frame: NativeFrame): ClosedFloatingPointRange<Float> {
        val maximum = minOf(frame.windowOffset.toLong(), frame.historyMaximum.toLong())
        val minimum = (frame.windowOffset.toLong() - (frame.rows.size - frame.viewport.rows.toInt()).coerceAtLeast(0))
            .coerceIn(0, maximum)
        return minimum * cellHeight..maximum * cellHeight
    }
    private fun hit(x: Float, y: Float): Pair<Int,Int> {
        val frame = drawnFrame
        val geometry = drawnGeometry ?: return 0 to 0
        return floor((y-geometry.shift) / geometry.cellHeight).toInt().coerceIn(0,(frame?.viewport?.rows?.toInt() ?: 1)-1) to
            floor(x / geometry.cellWidth).toInt().coerceIn(0,(frame?.viewport?.columns?.toInt() ?: 1)-1)
    }
    private fun moveScroll(value: Float) {
        val frame = current ?: return
        if (frame.state != "active") return
        val previous = scrollPixels
        val bounds = scrollBounds(frame)
        scrollPixels = value.coerceIn(bounds)
        val target = kotlin.math.ceil(scrollPixels / cellHeight).toLong()
        val edge = when { value > bounds.endInclusive -> 1; value < bounds.start -> -1; else -> 0 }
        if (edge != 0) {
            scroller.forceFinished(true)
            waitingEdge = edge
            // Ask for the next row to expose an adjacent cached/prefetched page.
            // Discard excess distance: delivery expands bounds, never replays it.
            submitScroll((target + edge).coerceIn(0, frame.historyMaximum.toLong()), edge > 0)
        } else {
            waitingEdge = 0
            submitScroll(target, scrollPixels >= previous)
        }
        awakenScrollBars()
        postInvalidateOnAnimation()
    }
    private fun submitScroll(target: Long, older: Boolean) {
        if (lastScrollTarget == target) return
        lastScrollTarget = target
        repository?.scroll(target, older,
            (2 + kotlin.math.abs(scroller.currVelocity) / (cellHeight * 12)).toInt().coerceIn(2, 8),
            kotlin.math.ceil(scrollPixels / cellHeight).toLong())
        scrollRequestGeneration = repository?.scrollRequestGeneration ?: scrollRequestGeneration
    }
    override fun computeVerticalScrollRange(): Int = ((current?.historyMaximum?.toLong() ?: 0) * cellHeight + height).coerceAtMost(Int.MAX_VALUE.toFloat()).toInt()
    override fun computeVerticalScrollExtent(): Int = height
    override fun computeVerticalScrollOffset(): Int = ((current?.historyMaximum?.toLong() ?: 0) * cellHeight - scrollPixels).coerceIn(0f,Int.MAX_VALUE.toFloat()).toInt()
    override fun computeScroll() {
        if (scroller.computeScrollOffset()) { moveScroll(scroller.currY.toFloat()); postInvalidateOnAnimation() }
    }
    private fun handlePosition(anchor: Boolean, frame: NativeFrame? = drawnFrame, geometry: DrawnTerminalGeometry? = drawnGeometry): Pair<Float,Float>? {
        frame ?: return null
        geometry ?: return null
        val selection = frame.selection ?: return null
        val row = (if (anchor) selection.anchorRow else selection.focusRow) - geometry.firstRow
        if (row !in 0 until frame.viewport.rows.toLong()) return null
        val column = if (anchor) selection.anchorColumn.toInt() else selection.focusColumn.toInt()+1
        return (column * geometry.cellWidth).coerceIn(handleRadius, maxOf(handleRadius,geometry.width-handleRadius)) to (row+1) * geometry.cellHeight + geometry.shift
    }
    override fun onTouchEvent(event: MotionEvent): Boolean {
        if (drawnGeometry?.version != geometryVersion || drawnFrame?.geometryGeneration != current?.geometryGeneration || imeAnimating()) return true
        val geometry = drawnGeometry ?: return true
        val frame = drawnFrame ?: return true
        if (event.actionMasked == MotionEvent.ACTION_DOWN &&
            (event.x >= frame.viewport.columns.toInt() * geometry.cellWidth || event.y < geometry.shift ||
             event.y - geometry.shift >= frame.viewport.rows.toInt() * geometry.cellHeight)) return true
        touchX = event.x; touchY = event.y
        if (event.actionMasked == MotionEvent.ACTION_DOWN) {
            draggingHandle = listOf(true,false).mapNotNull { anchor ->
                handlePosition(anchor)?.let { (x,y) ->
                    val drawable = handles[if (anchor) 0 else 1]
                    val centerX = x + (drawable?.intrinsicWidth ?: 0) * if (anchor) -.25f else .25f
                    val dx = centerX-event.x; val dy = y+handleRadius-event.y
                    if (kotlin.math.abs(dx) < handleRadius*2 && kotlin.math.abs(dy) < handleRadius*2) anchor to dx*dx+dy*dy else null
                }
            }.minByOrNull { it.second }?.first
            if (draggingHandle != null) { scroller.forceFinished(true); postOnAnimation(autoScroll); parent?.requestDisallowInterceptTouchEvent(true) }
        }
        if (draggingHandle != null) {
            if (event.actionMasked == MotionEvent.ACTION_MOVE) { actionMode?.hide(1000); extendHandle() }
            if (event.actionMasked == MotionEvent.ACTION_UP || event.actionMasked == MotionEvent.ACTION_CANCEL) {
                draggingHandle = null; removeCallbacks(autoScroll); actionMode?.invalidateContentRect()
            }
            return true
        }
        val handled = gesture.onTouchEvent(event)
        if (event.actionMasked == MotionEvent.ACTION_UP || event.actionMasked == MotionEvent.ACTION_CANCEL) releaseGestureSource()
        return handled
    }
    private fun extendHandle() {
        val source = drawnSource ?: return
        val anchor = draggingHandle ?: return
        val (row,column) = hit(touchX,touchY-cellHeight*.5f)
        repository?.extendSelection(source,row,column,anchor)
    }
    private fun drawSelection(canvas: Canvas, frame: NativeFrame, geometry: DrawnTerminalGeometry) {
        val selection = frame.selection ?: return
        val a = selection.anchorRow to selection.anchorColumn.toInt()
        val b = selection.focusRow to selection.focusColumn.toInt()
        val forward = a.first < b.first || a.first == b.first && a.second <= b.second
        val first = if (forward) a else b; val last = if (forward) b else a
        paint.color = 0xFF7CA66A.toInt(); paint.alpha = 90
        frame.rows.forEachIndexed { index, _ ->
            val logical = frame.firstRow + index
            if (logical in first.first..last.first) {
                val left = if (logical == first.first) first.second else 0
                val right = if (logical == last.first) last.second+1 else frame.viewport.columns.toInt()
                val row = logical - geometry.firstRow
                canvas.drawRect(left*cellWidth,row*cellHeight,right*cellWidth,(row+1)*cellHeight,paint)
            }
        }
        paint.alpha = 255
        for (anchor in listOf(true,false)) handlePosition(anchor, frame, geometry)?.let { (x,y) ->
            handles[if (anchor) 0 else 1]?.let { handle ->
                val w = handle.intrinsicWidth.coerceAtLeast(1)
                val h = handle.intrinsicHeight.coerceAtLeast(1)
                val left = (x - if (anchor) w*.75f else w*.25f).toInt().coerceIn(0,(width-w).coerceAtLeast(0))
                val top = (y-geometry.shift).toInt()
                handle.setBounds(left,top,left+w,top+h); handle.draw(canvas)
            }
        }
    }
    private fun showSelectionActions() {
        actionMode = startActionMode(object : ActionMode.Callback2() {
            override fun onCreateActionMode(mode: ActionMode, menu: Menu): Boolean {
                menu.add(0,android.R.id.copy,0,android.R.string.copy).setShowAsAction(MenuItem.SHOW_AS_ACTION_ALWAYS)
                return true
            }
            override fun onPrepareActionMode(mode: ActionMode, menu: Menu): Boolean = false
            override fun onActionItemClicked(mode: ActionMode, item: MenuItem): Boolean {
                if (item.itemId != android.R.id.copy) return false
                repository?.copySelection { text ->
                    context.getSystemService(android.content.ClipboardManager::class.java)
                        .setPrimaryClip(android.content.ClipData.newPlainText("",text))
                    repository?.clearSelection(); mode.finish()
                }
                return true
            }
            override fun onDestroyActionMode(mode: ActionMode) { if (actionMode === mode) { actionMode = null; if (!disposing) repository?.clearSelection() } }
            override fun onGetContentRect(mode: ActionMode, view: View, outRect: Rect) {
                val point = handlePosition(false) ?: handlePosition(true)
                val y = point?.second?.toInt()?.coerceIn(0,height) ?: height/2
                outRect.set(0,(y-cellHeight).toInt().coerceAtLeast(0),width,y)
            }
        },ActionMode.TYPE_FLOATING)
    }
    override fun onWindowVisibilityChanged(visibility: Int) {
        super.onWindowVisibilityChanged(visibility)
        repository?.terminalVisible(visibility == VISIBLE)
        if (visibility != VISIBLE) { rowRenderer.clear(); scroller.forceFinished(true); draggingHandle = null; releaseGestureSource(); removeCallbacks(autoScroll) }
    }
    override fun onAttachedToWindow() {
        super.onAttachedToWindow(); disposing = false; observeFrames()
    }
    override fun onDetachedFromWindow() {
        disposing = true
        rowRenderer.clear()
        imeMeasurePending = false
        releaseGestureSource()
        unobserve?.invoke(); unobserve = null
        pendingSource?.close(); pendingSource = null
        drawnSource?.close(); drawnSource = null; drawnFrame = null; drawnGeometry = null; current = null
        scroller.forceFinished(true); draggingHandle = null; removeCallbacks(autoScroll)
        repository?.terminalVisible(false)
        super.onDetachedFromWindow()
    }
    override fun performClick(): Boolean { super.performClick(); return true }
    fun setKeyboardVisible(visible: Boolean) {
        if (!visible) manager.hideSoftInputFromWindow(windowToken, 0)
        else if (current?.inputReady == true) {
            requestFocus()
            manager.showSoftInput(this, InputMethodManager.SHOW_IMPLICIT)
        }
    }
    override fun onCheckIsTextEditor(): Boolean = true
    override fun onCreateInputConnection(outAttrs: EditorInfo): InputConnection {
        outAttrs.inputType = InputType.TYPE_CLASS_TEXT or InputType.TYPE_TEXT_FLAG_MULTI_LINE or InputType.TYPE_TEXT_FLAG_NO_SUGGESTIONS
        outAttrs.imeOptions = EditorInfo.IME_FLAG_NO_EXTRACT_UI or EditorInfo.IME_FLAG_NO_FULLSCREEN or EditorInfo.IME_ACTION_NONE
        outAttrs.initialSelStart = composing.length; outAttrs.initialSelEnd = composing.length
        outAttrs.privateImeOptions = "com.google.android.inputmethod.latin.noMicrophoneKey"
        return TerminalInputConnection().also { ime = it }
    }
    private fun commit(value: String, paste: Boolean = false): Boolean {
        if (current?.inputReady != true || repository?.text(value, pendingModifiers(), paste) != true) return false
        if (value.isNotEmpty()) consumedModifiers()
        composing.clear(); manager.updateSelection(this,0,0,-1,-1); updateAnchor(); invalidate()
        return true
    }
    private fun updateAnchor() {
        val frame = drawnFrame ?: return
        val geometry = drawnGeometry ?: return
        if (geometry.version != geometryVersion || frame.inputEpoch != current?.inputEpoch) return
        if (!frame.inputReady || scrollPixels != 0f || frame.historyOffset != 0uL || frame.selection != null) return
        val matrix = Matrix().apply { setTranslate(geometry.screenX.toFloat(), geometry.screenY.toFloat()) }
        val x = frame.cursorColumn.toInt() * geometry.cellWidth
        val y = (frame.firstRow + frame.windowOffset.toLong() + frame.cursorRow.toInt() - geometry.firstRow) * geometry.cellHeight + geometry.shift
        val visible = x in 0f..geometry.width.toFloat() && y >= 0f && y + geometry.cellHeight <= geometry.height
        val flags = if (visible) CursorAnchorInfo.FLAG_HAS_VISIBLE_REGION else CursorAnchorInfo.FLAG_HAS_INVISIBLE_REGION
        val builder = CursorAnchorInfo.Builder().setMatrix(matrix).setSelectionRange(composing.length, composing.length)
            .setInsertionMarkerLocation(x,y,y+geometry.baseline,y+geometry.cellHeight,flags)
        if (composing.isNotEmpty()) builder.setComposingText(0,composing)
        manager.updateCursorAnchorInfo(this,builder.build())
    }
    private inner class TerminalInputConnection : BaseInputConnection(this@TerminalView, true) {
        private val inputEpoch = current?.inputEpoch
        private val retiredEditable = SpannableStringBuilder()
        private fun ready() = current?.inputReady == true && current?.inputEpoch == inputEpoch
        override fun getEditable(): Editable = if (current?.inputEpoch == inputEpoch) composing else retiredEditable
        override fun setComposingText(text: CharSequence?, newCursorPosition: Int): Boolean {
            if (!ready() || (text?.length ?: 0) > 524288) return false
            composing.replace(0,composing.length,text ?: ""); updateAnchor(); invalidate(); return true
        }
        override fun commitText(text: CharSequence?, newCursorPosition: Int): Boolean = ready() && commit(text?.toString().orEmpty())
        override fun finishComposingText(): Boolean = ready() && (composing.isEmpty() || commit(composing.toString()))
        override fun deleteSurroundingText(beforeLength: Int, afterLength: Int): Boolean = delete(beforeLength,afterLength,false)
        override fun deleteSurroundingTextInCodePoints(beforeLength: Int, afterLength: Int): Boolean = delete(beforeLength,afterLength,true)
        private fun delete(before: Int, after: Int, codePoints: Boolean): Boolean {
            if (!ready()) return false
            if (before < 0 || after < 0 || before > 128 || after > 128) return false
            if (composing.isNotEmpty()) {
                val count = if (codePoints) composing.toString().codePointCount(0,composing.length) else composing.length
                var start = if (codePoints) Character.offsetByCodePoints(composing,composing.length,-minOf(before,count)) else (composing.length-before).coerceAtLeast(0)
                if (start > 0 && start < composing.length && Character.isLowSurrogate(composing[start])) start--
                composing.delete(start,composing.length); updateAnchor(); invalidate()
            } else {
                if (repository?.deleteKeys(before, after, pendingModifiers()) != true) return false
                if (before + after > 0) consumedModifiers()
            }
            return true
        }
        override fun performEditorAction(actionCode: Int): Boolean { if (!ready() || repository?.key(NativeKey.Enter, pendingModifiers()) != true) return false; consumedModifiers(); return true }
        override fun requestCursorUpdates(cursorUpdateMode: Int): Boolean { if (!ready()) return false; updateAnchor(); return true }
        override fun sendKeyEvent(event: KeyEvent): Boolean = ready() && handleKey(event)
        override fun performContextMenuAction(id: Int): Boolean {
            if (!ready() || id != android.R.id.paste) return false
            val clipboard = context.getSystemService(android.content.ClipboardManager::class.java)
            return clipboard.primaryClip?.getItemAt(0)?.coerceToText(context)?.toString()?.let { commit(it,true) } ?: false
        }
    }
    override fun onKeyDown(keyCode: Int, event: KeyEvent): Boolean = handleKey(event) || super.onKeyDown(keyCode,event)
    override fun onKeyUp(keyCode: Int, event: KeyEvent): Boolean = handleKey(event) || super.onKeyUp(keyCode,event)
    private fun handleKey(event: KeyEvent): Boolean {
        if (event.keyCode == KeyEvent.KEYCODE_BACK || KeyEvent.isModifierKey(event.keyCode)) return false
        val key: NativeKey = when(event.keyCode) {
            KeyEvent.KEYCODE_ESCAPE -> NativeKey.Escape; KeyEvent.KEYCODE_ENTER, KeyEvent.KEYCODE_NUMPAD_ENTER -> NativeKey.Enter
            KeyEvent.KEYCODE_TAB -> NativeKey.Tab; KeyEvent.KEYCODE_DEL -> NativeKey.Backspace; KeyEvent.KEYCODE_FORWARD_DEL -> NativeKey.Delete
            KeyEvent.KEYCODE_DPAD_LEFT -> NativeKey.Left; KeyEvent.KEYCODE_DPAD_RIGHT -> NativeKey.Right
            KeyEvent.KEYCODE_DPAD_UP -> NativeKey.Up; KeyEvent.KEYCODE_DPAD_DOWN -> NativeKey.Down
            KeyEvent.KEYCODE_MOVE_HOME -> NativeKey.Home; KeyEvent.KEYCODE_MOVE_END -> NativeKey.End
            KeyEvent.KEYCODE_PAGE_UP -> NativeKey.PageUp; KeyEvent.KEYCODE_PAGE_DOWN -> NativeKey.PageDown
            in KeyEvent.KEYCODE_F1..KeyEvent.KEYCODE_F12 -> NativeKey.Function((event.keyCode-KeyEvent.KEYCODE_F1+1).toUByte())
            else -> { val scalar = event.getUnicodeChar(0); if (scalar <= 0 || scalar and android.view.KeyCharacterMap.COMBINING_ACCENT != 0) return false; NativeKey.Unicode(scalar.toUInt()) }
        }
        val modifiers = (if (event.isShiftPressed) 1 else 0) or (if (event.isAltPressed) 2 else 0) or
            (if (event.isCtrlPressed) 4 else 0) or (if (event.isMetaPressed) 8 else 0) or pendingModifiers()
        val scalar = event.unicodeChar
        val text = if (scalar in 32..0x10ffff && scalar !in 0x7f..0x9f && scalar !in 0xd800..0xdfff) String(Character.toChars(scalar)) else ""
        if (repository?.key(key,modifiers,if (event.action == KeyEvent.ACTION_UP) 3 else if (event.repeatCount > 0) 2 else 1,text) != true) return false
        if (event.action != KeyEvent.ACTION_UP) consumedModifiers()
        return true
    }
}
