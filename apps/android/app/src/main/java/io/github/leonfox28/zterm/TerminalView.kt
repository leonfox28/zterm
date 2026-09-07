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
    private var drawnSource: NativeFrameSource? = null
    private var pendingSource: NativeFrameSource? = null
    private var disposing = false
    var pendingModifiers: () -> Int = { 0 }
    var consumedModifiers: () -> Unit = {}
    private var current: NativeFrame? = null
    private val paint = Paint(Paint.ANTI_ALIAS_FLAG).apply { typeface = Typeface.MONOSPACE }
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
    private var gridBottomGap = 0f
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
            gestureMode = drawnFrame?.pointerMode ?: NativePointerMode.NONE
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
        if (font != size) { font = size; updateMetrics(); measureGrid(); invalidate() }
        observeFrames()
        if (unobserve == null) acceptFrame(frame)
    }
    private fun observeFrames() {
        if (unobserve == null) unobserve = repository?.observeTerminalFrames(::acceptFrame)
    }
    private fun acceptFrame(frame: NativeFrame?) {
        if (current !== frame) {
            pendingSource?.close()
            pendingSource = frame?.source?.retained()
            if (frame == null) { drawnSource?.close(); drawnSource = null; drawnFrame = null }
            val previous = current
            current = frame
            if (previous?.inputEpoch != frame?.inputEpoch || gestureMode != frame?.pointerMode) releaseGestureSource()
            if (previous?.viewport != frame?.viewport && !imeAnimating()) gridBottomGap = height % cellHeight
            if (previous?.inputEpoch != frame?.inputEpoch) {
                composing.clear(); consumedModifiers(); manager.restartInput(this)
                scroller.forceFinished(true); scrollPixels = (frame?.historyOffset?.toLong() ?: 0) * cellHeight
            } else if (frame != null && previous != null) {
                val growth = frame.historyMaximum.toLong() - previous.historyMaximum.toLong()
                if (scrollPixels > 0f && growth > 0) scrollPixels += growth * cellHeight
                if (frame.historyOffset == 0uL && frame.cursorVisible && frame.selection == null) scrollPixels = 0f
            }
            if (frame?.selection != null && actionMode == null) showSelectionActions()
            if (frame?.selection == null) { val old = actionMode; actionMode = null; old?.finish() }
            actionMode?.invalidateContentRect()
            updateAnchor()
            postInvalidateOnAnimation()
        }
    }
    private fun updateMetrics() {
        paint.textSize = android.util.TypedValue.applyDimension(android.util.TypedValue.COMPLEX_UNIT_SP, font.toFloat(), resources.displayMetrics)
        cellWidth = paint.measureText("M").coerceAtLeast(1f)
        val metrics = paint.fontMetrics
        cellHeight = kotlin.math.ceil((metrics.descent - metrics.ascent).toDouble()).toFloat().coerceAtLeast(1f)
        baseline = -metrics.ascent
    }
    override fun onSizeChanged(w: Int, h: Int, oldw: Int, oldh: Int) { measureGrid(); updateAnchor() }
    override fun onLayout(changed: Boolean, left: Int, top: Int, right: Int, bottom: Int) { measureGrid() }
    private fun measureGrid() {
        if (width <= 0 || height <= 0 || animatedInsets || imeMeasurePending || imeAnimating()) return
        val size = max(1, floor(height / cellHeight).toInt()) to max(1, floor(width / cellWidth).toInt())
        if (size != lastSize) { lastSize = size; repository?.measure(size.first, size.second) }
    }
    override fun onDraw(canvas: Canvas) {
        val frame = current ?: return
        canvas.clipRect(0, 0, width, height)
        if (drawnFrame !== frame) {
            drawnSource?.close(); drawnSource = pendingSource; pendingSource = null
            drawnFrame = frame
            if (draggingHandle != null) post { extendHandle() }
        }
        canvas.drawColor(frame.background.toInt())
        val shift = drawShift()
        canvas.save(); canvas.translate(0f, shift)
        frame.rows.forEachIndexed { row, line ->
            val top = row * cellHeight
            if (top + shift >= height || top + cellHeight + shift <= 0) return@forEachIndexed
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
        drawSelection(canvas, frame)
        val cursorX = frame.cursorColumn.toInt() * cellWidth
        val cursorY = frame.cursorRow.toInt() * cellHeight
        if (frame.cursorVisible) {
            paint.color = frame.cursorColor.toInt(); paint.alpha = 130
            canvas.drawRect(cursorX,cursorY,cursorX+cellWidth,cursorY+cellHeight,paint); paint.alpha = 255
        }
        if (composing.isNotEmpty()) {
            paint.color = frame.background.toInt()
            val extent = paint.measureText(composing.toString())
            canvas.drawRect(cursorX,cursorY,cursorX+extent,cursorY+cellHeight,paint)
            paint.color = frame.cursorColor.toInt()
            canvas.drawText(composing.toString(),cursorX,cursorY+baseline,paint)
            canvas.drawLine(cursorX,cursorY+cellHeight-1,cursorX+extent,cursorY+cellHeight-1,paint)
        }
        canvas.restore()
    }
    private fun geometryShift(frame: NativeFrame): Float {
        if (frame.historyOffset != 0uL || frame.selection != null) return 0f
        val difference = height - (frame.viewport.rows.toInt() * cellHeight + gridBottomGap)
        // Consume blank rows before panning the cursor up. During expansion,
        // reserve room for the history the host can pull back above the grid.
        val minimum = minOf(0f, height - (frame.cursorRow.toInt()+1) * cellHeight - gridBottomGap)
        return difference.coerceIn(minimum, frame.historyMaximum.toLong() * cellHeight)
    }
    private fun drawShift(): Float {
        val frame = drawnFrame ?: current ?: return 0f
        val geometry = geometryShift(frame)
        val target = kotlin.math.ceil(scrollPixels / cellHeight).toLong()
        if (frame.historyOffset.toLong() != target) return geometry
        return geometry - (target * cellHeight - scrollPixels)
    }
    private fun hit(x: Float, y: Float): Pair<Int,Int> {
        val frame = drawnFrame
        return floor((y-drawShift()) / cellHeight).toInt().coerceIn(0,(frame?.viewport?.rows?.toInt() ?: 1)-1) to
            floor(x / cellWidth).toInt().coerceIn(0,(frame?.viewport?.columns?.toInt() ?: 1)-1)
    }
    private fun moveScroll(value: Float) {
        val frame = current ?: return
        if (frame.state != "active") return
        val previous = scrollPixels
        scrollPixels = value.coerceIn(0f,frame.historyMaximum.toLong() * cellHeight)
        val target = kotlin.math.ceil(scrollPixels / cellHeight).toLong()
        repository?.scroll(target, scrollPixels >= previous, (2 + kotlin.math.abs(scroller.currVelocity) / (cellHeight*12)).toInt().coerceIn(2,8))
        awakenScrollBars()
        postInvalidateOnAnimation()
    }
    override fun computeVerticalScrollRange(): Int = ((current?.historyMaximum?.toLong() ?: 0) * cellHeight + height).coerceAtMost(Int.MAX_VALUE.toFloat()).toInt()
    override fun computeVerticalScrollExtent(): Int = height
    override fun computeVerticalScrollOffset(): Int = (((current?.historyMaximum?.toLong() ?: 0) - (current?.historyOffset?.toLong() ?: 0)) * cellHeight).coerceIn(0f,Int.MAX_VALUE.toFloat()).toInt()
    override fun computeScroll() {
        if (scroller.computeScrollOffset()) { moveScroll(scroller.currY.toFloat()); postInvalidateOnAnimation() }
    }
    private fun handlePosition(anchor: Boolean): Pair<Float,Float>? {
        val frame = drawnFrame ?: return null
        val selection = frame.selection ?: return null
        val row = (if (anchor) selection.anchorRow else selection.focusRow) - frame.firstRow
        if (row !in 0 until frame.viewport.rows.toLong()) return null
        val column = if (anchor) selection.anchorColumn.toInt() else selection.focusColumn.toInt()+1
        return (column * cellWidth).coerceIn(handleRadius,width-handleRadius) to (row+1) * cellHeight + drawShift()
    }
    override fun onTouchEvent(event: MotionEvent): Boolean {
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
    private fun drawSelection(canvas: Canvas, frame: NativeFrame) {
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
                canvas.drawRect(left*cellWidth,index*cellHeight,right*cellWidth,(index+1)*cellHeight,paint)
            }
        }
        paint.alpha = 255
        for (anchor in listOf(true,false)) handlePosition(anchor)?.let { (x,y) ->
            handles[if (anchor) 0 else 1]?.let { handle ->
                val w = handle.intrinsicWidth.coerceAtLeast(1)
                val h = handle.intrinsicHeight.coerceAtLeast(1)
                val left = (x - if (anchor) w*.75f else w*.25f).toInt().coerceIn(0,(width-w).coerceAtLeast(0))
                val top = (y-drawShift()).toInt()
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
        if (visibility != VISIBLE) { scroller.forceFinished(true); draggingHandle = null; releaseGestureSource(); removeCallbacks(autoScroll) }
    }
    override fun onAttachedToWindow() {
        super.onAttachedToWindow(); disposing = false; observeFrames()
    }
    override fun onDetachedFromWindow() {
        disposing = true
        imeMeasurePending = false
        releaseGestureSource()
        unobserve?.invoke(); unobserve = null
        pendingSource?.close(); pendingSource = null
        drawnSource?.close(); drawnSource = null; drawnFrame = null; current = null
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
    private fun commit(value: String, paste: Boolean = false) {
        if (current?.inputReady != true) return
        repository?.text(value, pendingModifiers(), paste)
        if (value.isNotEmpty()) consumedModifiers()
        composing.clear(); manager.updateSelection(this,0,0,-1,-1); updateAnchor(); invalidate()
    }
    private fun updateAnchor() {
        val frame = current ?: return
        if (!frame.inputReady || frame.historyOffset != 0uL || frame.selection != null) return
        val location = IntArray(2); getLocationOnScreen(location)
        val matrix = Matrix().apply { setTranslate(location[0].toFloat(), location[1].toFloat()) }
        val x = frame.cursorColumn.toInt() * cellWidth
        val y = frame.cursorRow.toInt() * cellHeight + geometryShift(frame)
        val builder = CursorAnchorInfo.Builder().setMatrix(matrix).setSelectionRange(composing.length, composing.length)
            .setInsertionMarkerLocation(x,y,y+baseline,y+cellHeight,CursorAnchorInfo.FLAG_HAS_VISIBLE_REGION)
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
        override fun commitText(text: CharSequence?, newCursorPosition: Int): Boolean { if (!ready()) return false; commit(text?.toString().orEmpty()); return true }
        override fun finishComposingText(): Boolean { if (!ready()) return false; if (composing.isNotEmpty()) commit(composing.toString()); return true }
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
                repeat(before) { repository?.key(NativeKey.Backspace, pendingModifiers()) }
                repeat(after) { repository?.key(NativeKey.Delete, pendingModifiers()) }
                if (before + after > 0) consumedModifiers()
            }
            return true
        }
        override fun performEditorAction(actionCode: Int): Boolean { if (!ready()) return false; repository?.key(NativeKey.Enter, pendingModifiers()); consumedModifiers(); return true }
        override fun requestCursorUpdates(cursorUpdateMode: Int): Boolean { if (!ready()) return false; updateAnchor(); return true }
        override fun sendKeyEvent(event: KeyEvent): Boolean = ready() && handleKey(event)
        override fun performContextMenuAction(id: Int): Boolean {
            if (!ready() || id != android.R.id.paste) return false
            val clipboard = context.getSystemService(android.content.ClipboardManager::class.java)
            clipboard.primaryClip?.getItemAt(0)?.coerceToText(context)?.toString()?.let { commit(it,true) }
            return true
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
        repository?.key(key,modifiers,if (event.action == KeyEvent.ACTION_UP) 3 else if (event.repeatCount > 0) 2 else 1,text)
        if (event.action != KeyEvent.ACTION_UP) consumedModifiers()
        return true
    }
}
