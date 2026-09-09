package io.github.leonfox28.zterm

import android.content.Context
import io.github.leonfox28.zterm.nativebridge.*
import kotlinx.coroutines.*
import kotlinx.coroutines.channels.Channel
import kotlinx.coroutines.flow.*
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock

internal enum class Route { Home, Scanner, Terminal, Settings }
internal data class TerminalStatus(val inputEpoch: ULong, val state: String, val error: String?, val notice: String?, val inputReady: Boolean, val connectionPath: NativeConnectionPath, val rttMs: UInt?)
internal data class AppState(
    val saved: SavedState = SavedState(),
    val initialized: Boolean = false,
    val route: Route = Route.Home,
    val hostId: String? = null,
    val sessionId: String? = null,
    val sessions: List<NativeSession> = emptyList(),
    val panel: Boolean = false,
    val busy: Boolean = false,
    val error: String? = null,
)

/** The Application owns mutations, streams and frames. Activity collectors own none of them. */
internal class AppRepository(context: Context, val runtime: NativeRuntime) {
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.Main.immediate)
    private val store = AppStore(context)
    private val storage = Mutex()
    private val mutable = MutableStateFlow(AppState())
    val state = mutable.asStateFlow()
    private val mutableFrame = MutableStateFlow<NativeFrame?>(null)
    val frame = mutableFrame.asStateFlow()
    val terminalStatus = frame.map { it?.let { frame ->
        TerminalStatus(frame.inputEpoch, frame.state, frame.error, frame.notice, frame.inputReady, frame.connectionPath, frame.rttMs)
    } }.distinctUntilChanged().stateIn(scope, SharingStarted.Eagerly, null)
    private val frameObservers = linkedSetOf<(NativeFrame?) -> Unit>()
    private var selectionVersion = 0L
    fun observeTerminalFrames(observer: (NativeFrame?) -> Unit): () -> Unit {
        frameObservers.add(observer)
        observer(mutableFrame.value)
        return { frameObservers.remove(observer) }
    }
    private fun publishFrame(next: NativeFrame?) {
        val previous = mutableFrame.value
        if (BuildConfig.DEBUG && (previous?.state != next?.state || previous?.error != next?.error)) {
            android.util.Log.d("ZtermState", "state=${next?.state ?: "detached"} code=${next?.error ?: "none"} rows=${next?.viewport?.rows} columns=${next?.viewport?.columns}")
        }
        if (previous?.inputEpoch != next?.inputEpoch || previous?.geometryGeneration != next?.geometryGeneration || previous?.selection != next?.selection) ++selectionVersion
        // View subscribers retain an independent source before the repository
        // releases its predecessor. Unobserved frames never wait for JVM GC.
        frameObservers.toList().forEach { it(next) }
        mutableFrame.value = next
        uploads.authorityChanged()
        if (previous !== next) previous?.source?.close()
    }
    private var terminal: NativeTerminal? = null
    private var observation: Job? = null
    private var operation: Job? = null
    private var epoch = 0L
    val uploads = TerminalUploads(context.applicationContext, scope) {
        val target = terminal
        val snapshot = frame.value
        if (target != null && snapshot?.inputReady == true && state.value.route == Route.Terminal) target to snapshot.inputEpoch else null
    }
    private var viewport = NativeViewport(24u, 80u)
    private var dark = true
    private data class Input(val attachment: Long, val nativeEpoch: ULong, val action: suspend (NativeTerminal, ULong) -> Unit, val source: NativeFrameSource? = null, val uploadVersion: Long)
    private val keys = Channel<Input>(32)
    private data class Scroll(val attachment: Long, val offset: ULong, val older: Boolean, val horizon: UByte)
    private val scrolls = Channel<Scroll>(Channel.CONFLATED)
    private val sizes = Channel<Unit>(Channel.CONFLATED)
    private val network = PlatformNetwork(context) { servers ->
        scope.launch { if (state.value.initialized) runCatching { runtime.updateDns(servers) } }
    }

    init {
        scope.launch {
            for (input in keys) {
                try {
                    val target = terminal ?: continue
                    if (input.attachment != epoch) continue
                    if (uploads.paused || input.uploadVersion != uploads.inputVersion) continue
                    if (input.nativeEpoch != frame.value?.inputEpoch) {
                        mutable.update { it.copy(error = "input_not_ready") }; continue
                    }
                    input.action(target, input.nativeEpoch)
                } catch (cancel: CancellationException) { throw cancel }
                catch (error: Exception) { report(error) }
                finally { input.source?.close() }
            }
        }
        scope.launch {
            for (scroll in scrolls) {
                val target = terminal ?: continue
                if (scroll.attachment != epoch) continue
                try { target.scrollTo(scroll.offset,scroll.older,scroll.horizon) }
                catch (cancel: CancellationException) { throw cancel }
                catch (error: Exception) { report(error) }
            }
        }
        scope.launch {
            for (signal in sizes) {
                val target = terminal ?: continue
                val mine = epoch
                try { target.resize(viewport) } catch (cancel: CancellationException) { throw cancel }
                catch (error: Exception) { if (mine == epoch && terminal === target) report(error) }
            }
        }
        scope.launch {
            try {
                val seed = withContext(Dispatchers.IO) { store.loadOrCreateSeed() }
                try {
                    val saved = withContext(Dispatchers.IO) { store.load() }
                    runtime.initialize(seed, network.current(), saved.hosts.map { it.native() })
                    mutable.update { it.copy(saved = saved, initialized = true) }
                    network.start()
                } finally { seed.fill(0) }
            } catch (error: Exception) { report(error) }
        }
    }
    private suspend fun save(transform: (SavedState) -> SavedState) = storage.withLock {
        val next = transform(state.value.saved)
        withContext(Dispatchers.IO) { store.save(next) }
        mutable.update { it.copy(saved = next) }
    }
    private fun act(block: suspend () -> Unit) {
        if (operation?.isActive == true || state.value.busy || !state.value.initialized) return
        mutable.update { it.copy(busy = true, error = null) }
        operation = scope.launch {
            try { block() } catch (cancel: CancellationException) { throw cancel }
            catch (error: Exception) { report(error) }
            finally { mutable.update { it.copy(busy = false) } }
        }
    }
    fun clearError() { mutable.update { it.copy(error = null) }; localTerminal { it.clearNotice() } }
    fun show(route: Route) {
        if (route == Route.Home && state.value.route == Route.Terminal) { goHome(); return }
        mutable.update { it.copy(route = route, error = null) }
    }
    fun setPreferences(preferences: Preferences) {
        updatePreferences { preferences }
    }
    fun updatePreferences(transform: (Preferences) -> Preferences) {
        if (!state.value.initialized) return
        scope.launch { try { save { it.copy(preferences = transform(it.preferences)) } } catch (error: Exception) { report(error) } }
    }
    fun setDark(value: Boolean) {
        if (dark == value) return
        dark = value
        scope.launch { try { terminal?.setDark(value) } catch (error: Exception) { report(error) } }
    }
    // Latest displayed intent is distinct from a one-row cache-edge probe.
    var scrollIntent: Long? = null
        private set
    var scrollRequestGeneration = 0L
        private set
    fun retireGeometry() { ++selectionVersion }
    fun measure(rows: Int, columns: Int) {
        val size = NativeViewport(rows.coerceIn(1, 80).toUShort(), columns.coerceIn(1, 240).toUShort())
        if (size != viewport) { viewport = size; ++selectionVersion; sizes.trySend(Unit) }
    }
    fun pair(ticket: String) {
        if (ticket.isBlank() || ticket.toByteArray().size > runtime.ticketTextLimit().toInt()) { mutable.update { it.copy(error = "invalid_ticket") }; return }
        act {
            runtime.pairTicket(ticket.trim()).use { pairing ->
                val value = pairing.host()
                val existing = state.value.saved.hosts.firstOrNull { it.id == value.deviceId }
                val host = SavedHost(value.deviceId, value.name, value.relayUrls, existing?.lastSession)
                save { saved -> saved.copy(hosts = saved.hosts.filterNot { it.id == host.id } + host) }
                runtime.commitPairing(pairing)
                if (state.value.route == Route.Scanner) openHost(host.id)
            }
        }
    }
    fun connectHost(host: String, recent: Boolean = false) = act {
        openHost(host, if (recent) state.value.saved.recent?.takeIf { it.host == host }?.session else null)
    }
    private suspend fun openHost(host: String, exact: String? = null) {
        val mine = retire()
        if (mine != epoch) return
        val last = exact ?: state.value.saved.hosts.first { it.id == host }.lastSession
        mutable.update { it.copy(route = Route.Terminal, hostId = host, sessionId = last, sessions = emptyList(), panel = false) }
        // Only a successful live list proves a remembered ID no longer exists.
        // Transport/auth/occupancy errors never authorize default creation.
        val sessions = refresh(host)
        if (mine != epoch || state.value.route != Route.Terminal) return
        if (last != null && sessions.any { it.sessionId == last }) {
            attach(host, last, false)
            return
        }
        if (last != null) {
            save { saved -> saved.copy(
                hosts = saved.hosts.map { if (it.id == host && it.lastSession == last) it.copy(lastSession = null) else it },
                recent = saved.recent?.takeUnless { it.host == host && it.session == last },
            ) }
            if (mine != epoch || state.value.route != Route.Terminal) return
        }
        mutable.update { it.copy(sessionId = null) }
        when (sessions.size) {
            0 -> { attach(host, null, false); if (state.value.route == Route.Terminal && state.value.hostId == host) refresh(host) }
            1 -> { val session = sessions.single()
                if (session.occupied) mutable.update { it.copy(panel = true) }
                else attach(host, session.sessionId, false)
            }
            else -> mutable.update { it.copy(panel = true) }
        }
    }
    private suspend fun refresh(host: String): List<NativeSession> {
        val mine = epoch
        val sessions = runtime.listSessions(host)
        if (mine == epoch && state.value.hostId == host && state.value.route == Route.Terminal) mutable.update { it.copy(sessions = sessions) }
        return sessions
    }
    fun togglePanel() {
        val next = !state.value.panel
        mutable.update { it.copy(panel = next) }
        val host = state.value.hostId ?: return
        if (next) scope.launch { try { refresh(host) } catch (error: Exception) { report(error) } }
    }
    fun closePanel() { mutable.update { it.copy(panel = false) } }
    fun selectSession(session: String, takeover: Boolean = false) {
        if (session == state.value.sessionId && frame.value?.state == "active") { closePanel(); return }
        act { val host = state.value.hostId ?: return@act; attach(host, session, takeover) }
    }
    fun retry() = act {
        val host = state.value.hostId ?: return@act
        openHost(host, state.value.sessionId)
    }
    private suspend fun attach(host: String, session: String?, takeover: Boolean) {
        val mine = retire()
        if (mine != epoch || state.value.route != Route.Terminal) return
        mutable.update { it.copy(sessionId = session, panel = false) }
        val requestedViewport = viewport
        val attached = if (session == null) runtime.connectDefaultTerminal(host, requestedViewport, dark)
            else runtime.connectTerminal(host, session, requestedViewport, dark, takeover)
        if (mine != epoch || state.value.route != Route.Terminal) {
            try { attached.detach() } finally { attached.close() }; return
        }
        val attachedSession = attached.sessionId()
        terminal = attached
        mutable.update { it.copy(sessionId = attachedSession) }
        // Measurement can finish while connectTerminal is suspended and there
        // is no terminal for the size consumer yet. Reapply its latest intent
        // through that same consumer without overwriting newer measurements.
        if (viewport != requestedViewport) sizes.trySend(Unit)
        // Metadata frames share an immutable row window. Resolve a replacement
        // off Main, once, while retaining its exact native source until delivery.
        var contentGeneration = 0uL
        var contentRows: List<NativeRow> = emptyList()
        suspend fun resolve(next: NativeFrame): NativeFrame {
            val source = next.source ?: return next
            try {
                if (next.contentGeneration != contentGeneration) {
                    contentRows = withContext(Dispatchers.Default) { source.presentationRows() }
                    contentGeneration = next.contentGeneration
                }
                return next.copy(rows = contentRows)
            } catch (error: Throwable) { source.close(); throw error }
        }
        val initial = resolve(attached.currentFrame())
        if (mine != epoch) { initial.source?.close(); return }
        publishFrame(initial)
        observation = scope.launch {
            var generation = initial.generation
            try {
                while (isActive && mine == epoch) {
                    val next = resolve(attached.waitForFrame(generation))
                    generation = next.generation
                    if (mine == epoch) publishFrame(next) else next.source?.close()
                }
            } catch (_: NativeException.Closed) { /* Final complete frame is retained. */ }
            catch (cancel: CancellationException) { throw cancel }
            catch (error: Exception) { if (mine == epoch) report(error) }
        }
        save { saved -> saved.copy(hosts = saved.hosts.map { if (it.id == host) it.copy(lastSession = attachedSession) else it }, recent = RecentConnection(host, attachedSession)) }
    }
    fun createSession(name: String, directory: String) = act {
        val host = state.value.hostId ?: return@act
        val mine = epoch
        val session = runtime.createSession(host, name, directory.ifBlank { null }, viewport, dark)
        // Save the returned identity before attach: retries never create again.
        if (mine == epoch) mutable.update { it.copy(sessionId = session.sessionId) }
        save { saved -> saved.copy(hosts = saved.hosts.map { if (it.id == host) it.copy(lastSession = session.sessionId) else it }) }
        if (mine == epoch && state.value.route == Route.Terminal && state.value.hostId == host) {
            attach(host, session.sessionId, false)
            refresh(host)
        }
    }
    fun renameSession(session: String, name: String) = act {
        val host = state.value.hostId ?: return@act
        runtime.renameSession(host, session, name)
        refresh(host)
    }
    fun deleteSession(session: String) = act {
        val host = state.value.hostId ?: return@act
        val mine = epoch
        runtime.closeSession(host, session)
        if (mine != epoch) return@act
        if (state.value.sessionId == session) retire()
        refresh(host)
        mutable.update { it.copy(panel = true) }
    }
    fun removeHost(host: String) = act {
        if (state.value.hostId == host) retire()
        save { saved -> saved.copy(hosts = saved.hosts.filterNot { it.id == host }, recent = saved.recent?.takeUnless { it.host == host }) }
        runtime.forgetHost(host)
        mutable.update { it.copy(route = Route.Home, hostId = null, sessionId = null, sessions = emptyList()) }
    }
    private suspend fun retire(): Long {
        uploads.retire()
        val mine = ++epoch
        scrollIntent = null
        val observer = observation
        observation = null
        while (true) { val input = keys.tryReceive().getOrNull() ?: break; input.source?.close() }
        val previous = terminal
        terminal = null
        publishFrame(null)
        observer?.cancelAndJoin()
        if (previous != null) {
            try { previous.detach() } catch (_: Exception) { /* Session is already closed or transport lost. */ }
            finally { previous.close() }
        }
        return mine
    }
    fun goHome() {
        uploads.retire()
        ++epoch
        mutable.update { it.copy(route = Route.Home) }
        scope.launch {
            if (retire() != epoch) return@launch
            mutable.update { it.copy(route = Route.Home, hostId = null, sessionId = null, panel = false, error = null, busy = operation?.isActive == true) }
        }
    }
    fun text(text: String, modifiers: Int = 0, paste: Boolean = false): Boolean {
        if (text.isEmpty()) return frame.value?.inputReady == true && !uploads.paused
        return enqueue { target, admitted -> target.commitText(admitted, text, modifiers.toUByte(), paste) }
    }
    fun key(key: NativeKey, modifiers: Int = 0, kind: Int = 1, text: String = ""): Boolean {
        return enqueue { target, admitted -> target.sendKey(admitted, key, modifiers.toUByte(), kind.toUByte(), text) }
    }
    fun deleteKeys(before: Int, after: Int, modifiers: Int): Boolean = enqueue { target, admitted ->
        repeat(before) { target.sendKey(admitted, NativeKey.Backspace, modifiers.toUByte(), 1u, "") }
        repeat(after) { target.sendKey(admitted, NativeKey.Delete, modifiers.toUByte(), 1u, "") }
    }
    fun pointer(source: NativeFrameSource, row: Int, column: Int, wheelLines: Int = 0) {
        if (uploads.paused) return
        val snapshot = frame.value ?: return
        if (snapshot.state != "active" || snapshot.pointerMode == NativePointerMode.NONE) return
        val held = source.retained()
        val input = Input(epoch, snapshot.inputEpoch, { target, _ -> target.sendPointer(held, row.toUShort(), column.toUShort(), wheelLines.toShort()) }, held, uploads.inputVersion)
        if (!keys.trySend(input).isSuccess) { held.close(); mutable.update { it.copy(error = "resource_limit") } }
    }
    private fun enqueue(action: suspend (NativeTerminal, ULong) -> Unit): Boolean {
        if (uploads.paused) return false
        val snapshot = frame.value ?: return false
        if (!snapshot.inputReady || terminal == null) return false
        val accepted = keys.trySend(Input(epoch,snapshot.inputEpoch,action, uploadVersion = uploads.inputVersion)).isSuccess
        if (!accepted) mutable.update { it.copy(error = "resource_limit") }
        else { scrollIntent = 0; ++scrollRequestGeneration }
        return accepted
    }
    fun scroll(offset: Long, older: Boolean, horizon: Int = 4, displayedOffset: Long? = null) {
        if (frame.value?.state == "active") {
            scrollIntent = (displayedOffset ?: offset).coerceAtLeast(0)
            if (displayedOffset == null) ++scrollRequestGeneration
            scrolls.trySend(Scroll(epoch,offset.coerceAtLeast(0).toULong(),older,horizon.coerceIn(2,8).toUByte()))
        }
    }
    fun select(source: NativeFrameSource, row: Int, column: Int) { ++selectionVersion; localSource(source) { target, held -> target.beginSelection(held,row.toUShort(),column.toUShort()) } }
    fun extendSelection(source: NativeFrameSource, row: Int, column: Int, anchor: Boolean) { ++selectionVersion; localSource(source) { target, held -> target.extendSelection(held,row.toUShort(),column.toUShort(),anchor) } }
    fun clearSelection() { ++selectionVersion; localTerminal { it.clearSelection() } }
    fun copySelection(copy: (String) -> Unit) {
        val selected = selectionVersion
        val attachment = epoch
        localTerminal { target ->
            val text = target.copySelection()
            if (selected == selectionVersion && attachment == epoch) copy(text)
        }
    }
    fun terminalVisible(visible: Boolean) = localTerminal { it.setVisible(visible) }
    private fun localSource(source: NativeFrameSource, action: suspend (NativeTerminal, NativeFrameSource) -> Unit) {
        val target = terminal ?: return
        val mine = epoch
        val held = source.retained()
        scope.launch {
            held.use {
                if (mine != epoch) return@launch
                try { action(target,held) } catch (cancel: CancellationException) { throw cancel }
                catch (error: Exception) { if (mine == epoch) report(error) }
            }
        }
    }
    private fun localTerminal(action: suspend (NativeTerminal) -> Unit) {
        val target = terminal ?: return
        val mine = epoch
        scope.launch {
            if (mine != epoch) return@launch
            try { action(target) } catch (cancel: CancellationException) { throw cancel }
            catch (error: Exception) { if (mine == epoch) report(error) }
        }
    }
    private fun report(error: Exception) {
        val code = when (error) { is NativeException.RequestFailed -> error.code; is StoreFailure -> error.code; else -> "transport_unavailable" }
        if (BuildConfig.DEBUG) android.util.Log.d("ZtermState", "operation_error=$code type=${error.javaClass.simpleName}")
        mutable.update { it.copy(error = code) }
    }
}
