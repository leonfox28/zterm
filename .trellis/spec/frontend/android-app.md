# Android application contract

## 1. Scope and trigger

Read before editing `apps/android`, Android build tools or mobile UI tests. Follow
the approved Android PRD/design and the shared client contract in
`../backend/shared-client.md`. Kotlin owns Android presentation/lifecycle and
platform APIs; Rust owns semantic content, synchronization and extraction.

## 2. Signatures and owners

`ZtermApplication` lazily owns one `NativeRuntime` and `AppRepository`.
Activity/Compose collectors observe state and never shut down connections.
`observeTerminalFrames` synchronously delivers each latest frame on Main so Views
can retain sources before the Repository closes its predecessor. `TerminalView`
separately owns pending/drawn source handles and releases each on replacement or
detach. Async selection commands retain their source before launching work.

`AppStore` uses AtomicFile in `noBackupFilesDir/zterm` and an AES-GCM Android
Keystore key to wrap the 32-byte identity seed. Store that identity before any
outbound pairing. Corruption or a missing key is an error, never a new identity.
Known hosts/recent IDs/preferences are one atomic no-backup state document.

Build through `sh tools/android/build.sh` / `just android-build`; generated Kotlin
is an output of the pinned Rust UniFFI bridge, never a manually edited source.
`just android-apk <versionCode>` signs a release with an external durable key and
verifies signature, checksum and 16 KiB native/zip alignment. Release applicationId
is `io.github.leonfox28.zterm`; debug adds `.dev` with label `zterm Dev`. Keep the
Kotlin namespace unchanged. Override the debug label in values and values-zh so
locale selection cannot hide the variant. Debug instrumentation lives under
`io.github.leonfox28.zterm.dev.test`; derive runtime paths from targetContext.
Variants have independent UID-scoped identity/host/settings stores and Keystore
keys. Existing release updates retain their applicationId and signing identity.

## 3. Contracts

Home has a Settings button, a conditional exact recent-Session card, saved hosts
and Add host. A host tap goes to Terminal. No Session route exists. Title expansion
overlays a bounded Session list without resizing the grid; row menus contain
Rename/Delete only. Named creation, destructive deletion and occupied takeover
remain explicit. Machine entry (host/recent/pairing) and Retry first list live
Sessions. Reuse the remembered ID if present; after authoritative absence, clear
matching saved references and attach one unoccupied Session, offer selection for
multiple/occupied Sessions, or call `connectDefaultTerminal` for an empty list.
Only default attachment may create reserved `main`; ordinary named creation
cannot. Store `NativeTerminal.sessionId()` as the authoritative result. Timeout,
authorization, occupancy and ambiguous outcomes never prove absence. SessionEnded
alone never triggers replacement; explicit row selection remains exact. Fence
all suspended selection/default-attach results by navigation epoch.
Terminal has no overflow, app-bar keyboard button or history/live/line
count banners. Two upload icons (photo, paperclip), eight shortcuts and a
rightmost keyboard show/hide button stay
above IME/insets during scroll and selection. Use the same TextButton, zero
content padding and equal-width 48 dp row slot for every control, including the
keyboard icon. Image, attachment, four direction arrows and keyboard use 18 dp
LineIcon vectors with the shared stroke and inherited button content color.
Keep Esc/Tab/Ctrl/Alt as 13 sp monospace text. Direction vectors have localized
content descriptions; do not render them as font-dependent arrow characters.
Enabled, disabled and pressed styling follows the neighboring shortcuts.

`AppState.sessionId` is the exact selected/retry target, not proof of controller
ownership. Mark a row Current only when its ID matches and the retained native
frame is active/synchronizing/reconnecting. A current-row tap only dismisses the
panel, including during recovery. Failed attach and ended/lease_lost frames do
not suppress occupied-row confirmation. Occupied and lost-lease error views
provide Take over using the existing confirmation, even before a Session list
has loaded. Freeze its exact ID; only confirmation may submit takeover=true.
Cancel must leave the competing controller Active. `OccupiedRecoveryTest` verifies
failed resume, row selection, cancellation and lost-lease recovery on one test-owned
Session; run it alone with `-e occupiedRecovery 1` to seed storage before the lazy
Repository starts.

Use CameraX plus bundled ML Kit. Lower-left image entry opens Android's photo
picker directly; lower-right manual entry opens a scanner-owned dialog with one
field, a wide Connect button and close control. Never read clipboard on opening
or store a bearer in saved-instance state. Pause camera analysis while a dialog,
picker, recognition or pairing attempt owns the scan. Filter multiple recognized
QRs using the shared decoder and let the user choose among valid host labels.

Language (system/zh/en), appearance (system/dark/light) and integer font size
(8–16, step 1, default 12) persist independently. `terminalFontSizes` is shared
by the Settings slider and AppStore loader. Compose `Slider.steps` counts the
seven interior positions, not the step size. Preview the local rounded value
while dragging and capture it before the asynchronous preference save on finish.
Accept all nine values on reload; retain the existing fallback to 12 for invalid
stored sizes. `SettingsUiTest` verifies end-to-end dragging, nine stored values
and Activity recreation; terminal UI tests check smaller/larger grid changes. Update one field using a transformation under the storage mutex,
not a stale snapshot of all preferences. A localized context must preserve its
Activity chain: use configuration-overridden `ContextThemeWrapper`; plain
`createConfigurationContext` breaks ActivityResultRegistry discovery. Preference
recomposition retains Session and selection unless measured geometry changes.

Native Canvas drawing explicitly clips to View bounds; Compose AndroidView
interop does not guarantee overscan cannot paint over title/shortcut chrome.
GestureDetector/OverScroller move local pixel offsets. Hit testing uses the drawn
source, not the next unpainted frame. ActionMode.TYPE_FLOATING supplies Copy;
Android theme handle drawables and nearest-handle hit testing support edge drag.
Only explicit Copy writes ClipData; no automatic success toast or remote Ctrl+C.
For a selection wholly belonging to one OSC 8 web link, the floating menu also
offers Open link. Native lookup reads the exact source-pinned selected rows and
refuses blocked/mixed targets; attachment and selection versions fence both menu
lookup and the click-time recheck. Reconnect/geometry/selection changes retire old
actions. Kotlin opens only HTTP(S) with ACTION_VIEW/BROWSABLE on explicit action,
with a local failure notice if no browser handles it. Ordinary tap/mouse routing
and Copy are unchanged; no URI is projected into every native cell.

`NativeFrame.firstRow` is the first row of a bounded presentation window,
not necessarily the visible top. `windowOffset` is the reading offset at that
window start; it can differ from `historyMaximum - firstRow` for a frozen epoch.
For `n` available rows and `h` viewport rows, local pixel bounds are
`[max(0, windowOffset - (n - h)), min(windowOffset, historyMaximum)] * cellHeight`.
Set `offset = ceil(scrollPixels / cellHeight)`, draw logical top
`firstRow + windowOffset - offset`, and add the fractional shift
`scrollPixels - offset * cellHeight` in `[-cellHeight, 0]` to the independent IME
pan. Cached pixels move immediately across multiple rows without actor delivery.

Submit only changed integer targets through the existing conflated scroll owner.
At an unavailable edge, stop inertia, discard excess distance and request the
adjacent row; its arrival expands available bounds without replaying movement.
Keep a still-valid drawn window when a late response after reversal does not
cover the actual position. Edge replies must overlap a complete waiting viewport;
see the multi-page query contract in `../backend/shared-client.md`. New output
rebases the reading offset while preserving logical rows, including an older
edge wait begun at zero. Stop the old-basis fling on that append.
`AppRepository.scrollIntent` distinguishes the displayed offset from an edge
probe; `scrollRequestGeneration` distinguishes explicit scroll/input return from
View metadata acknowledgements. A late live frame must not erase local motion.

Repository uses `TerminalFrameProjection` to resolve source-relative changed rows
off Main and preserve equal Kotlin row objects, including moved rows. Metadata-only
frames reuse the full list. `collectTerminalFrames` paces compatible content bursts
with a cancellable display clock; native installation/ACK stays independent. Read
[Frame preparation](./android-frame-preparation.md) for exact mapping,
lifetime and visibility contracts. Compose observes only `TerminalStatus`; the direct View observer owns content
frames. Include typed `connectionPath` and nullable `rttMs` in that metadata
projection. The fixed 48 dp header keeps title and host, reserves subtitle space
for connection state/latency and ellipsizes long host names. `Direct` and `Relay`
are non-translatable English resources in every app/system locale. Other states
are localized; unknown RTT is `— ms`. Initial synchronization without a known
path shows Connecting; healthy synchronization retains established route/RTT.

Kotlin row windows are bounded to three viewports plus one row, separately from
native cache budgets. API 29+ hardware Canvas reuses `TerminalRowRenderer`
RenderNodes by immutable resolved cell content and recording width/height,
independently of source row ordinals and soft-wrap metadata. A bounded ordinal
binding avoids repeated hashing/allocation for unchanged row objects; both maps
are limited to three viewports plus one entry. Hash equality alone is insufficient.
Each hardware node uses `setUseCompositingLayer(true, null)` to retain its text-row
pixels: a display list alone still replays glyph/background commands on
RenderThread. This is Android-managed per-row storage under the same cache bound,
not another history cache. Count its GPU allocations when profiling; display-list
recording counts do not measure replay cost. Fractional movement can resample
cached text; integer placement must match the direct painter and row joins must
retain their declared backgrounds.
Font/width changes, input-epoch changes, background visibility and detach discard
lists. Height-only layout and geometry-generation changes still retire coordinates
but preserve compatible text. Check `hasDisplayList()` before reuse: Android may
discard drawing that left the displayed scene. API 26–28 and software Canvas use
the same cell painter directly. Compose a complete frame from background, reused
or updated text and current overlays; do not depend on old Canvas pixels surviving.
Do not build a full-history bitmap or cache cursor, selection or preedit in nodes.

Concealed cells paint only their background: glyphs, underline and strike are
hidden without changing semantic text or explicit Copy. Visible strike draws
across the full glyph cell span. Selection and cursor overlays do not repaint
concealed glyphs.
Cursor shape and blinking come from native semantic metadata. Block/beam/underline
are dynamic overlays, outside row RenderNodes. The View schedules a local 500 ms
blink only when attached, shown, window-visible and displaying a visible live
blinking cursor; hidden/background/detached views cancel it. The phase does not
modify row data or create native/network updates.

For other cells, the painter omits only glyph submission/foreground setup for an empty string
or exactly one ASCII space. Always paint the cell background and declared underline;
do not skip the whole blank cell, use generic whitespace detection, or trim its
semantic text. A space followed by a combining mark still needs glyph drawing.
Retain clipping and bold/dim/italic state restoration for actual glyphs. This
reduces cold row-recording work, including when Android discarded offscreen lists,
without introducing another cache or changing source/Copy authority.

After drawing, bind `viewportSource(actualFirstRow)` to the committed geometry.
A moved logical top retires pending Copy; hit/selection coordinates use the bound
source even while the native navigation offset lags. Never use the window's
first row as the visible top. `TerminalRenderingTest` verifies actual Canvas
pixels across multi-row motion, late/reversed deliveries, cache edges and output
append; hardware PixelCopy verifies that changed cells update and other pixels
remain identical, including A-B-A content restoration.

`TerminalGridLayout` measures fixed header/divider/toolbar first, then derives
one terminal height from the parent's remaining physical pixels. Stable height
is `floor(available / cellHeight) * cellHeight`; place the subrow remainder
below the toolbar, inside already consumed navigation/IME insets. Paint supplies
the integer ceil font height through `gridCellHeight`. Never derive remainder
from an already rounded child height or add padding above the toolbar.
The 80-row cap retains neutral excess area; exclude that area from cell hits and
pan calculations. Subcell windows keep one clipped row, not a zero-height view.
Use an ordinary `Box` below the terminal screen's inset padding. Only the visible
session panel needs `BoxWithConstraints` to cap its height at 65% / 420 dp; putting
the whole terminal in that scope repeats subcomposition as IME constraints change.
Keep animated inset reads in `TerminalGridLayout` measurement. Recomposition
counts, row recording and Android FrameMetrics measure different work; fewer
compositions alone do not establish a device FPS improvement.

During IME animation, `imePadding` moves chrome and clips/pans the existing grid
locally. `terminalPan(height, cellHeight, rows, cursorRow: Int?)` applies the same
signed `height - rows * cellHeight` movement on both live screens, in both
directions. A visible caret supplies only a top-edge constraint: clamp upward
movement at `-cursorRow * cellHeight`. It does not pull the caret to the toolbar
or discard footer/pane space below it. Hidden cursors supply null and do not
limit movement. History extent does not limit growth; newly exposed unknown
area stays background. Main/alternate is not a shell/TUI classifier. Keep the
authoritative screen for source compatibility and actual screen-switch fences.
Frozen history/selection retain their reading owner, incompatible widths bypass pan,
and the 80-row cap still limits its effective height. Incoming content compares
against that moved presentation; different source ordinals do not force recording
equal rows. Source/geometry commit and input fences advance even when pixels match.
`terminalBottomRemainder` interpolates the endpoint remainders using
source/current/target consumed bottom insets; do not round every animation frame
to whole rows. Stable 17 px cells at available 1000/600 px give grids 986/595 px
and bottom padding 14/5 px. The old 58-row grid moves by -23 rows (cursor row 39
lands on row 16); there is no extra 9 px remainder correction at handoff. A
different remote layout still replaces those row positions. An Activity-owned `WindowInsetsAnimationCompat.Callback` on the decor
View tracks every prepared IME animation until its matching end. Register above
Compose, whose inset consumer stops descendant animation dispatch. Unrelated
bar animations cannot release the IME fence. Neither a conflated size channel
nor equality of Compose animation source/target insets is a lifetime fence.
Deliver prepare/end edges directly through `ImeAnimationState.observe`, registered
for the Compose-owned TerminalView lifetime. Recomposition can conflate both edges
and is not an event delivery mechanism. Read the live animation owner inside the
layout measurement so intermediate frames interpolate instead of stepping by rows.
`imeCompletionSurvivesCoalescedCompositionAndOverlappingAnimations` covers synchronous
edges, overlap and observer disposal. On completion use `OneShotPreDrawListener` to commit the settled View once.
Keep measurement fenced through that final layout and clear the pending flag
on detach. Native `requestLayout()` alone may not cause a same-size layout
callback through Compose AndroidView, leaving the host at the old size. Commit `DrawnTerminalGeometry` with `drawnFrame`/`drawnSource` after drawing.
Hit tests, selection handles and IME anchors consume those exact cell metrics,
pan, logical first row, bounds and screen origin. Pending frames/layout never silently change the
coordinate source; local layout/font changes retire active gestures. Keep
complete Canvas draws and explicit bounded pending/drawn handle ownership. Do not
send a network resize per animation frame or substitute a debounce delay.

For an input-ready live screen, `TerminalGridLayout` also calls
`terminalImeTargetHeight(available, currentBottom, startBottom, endBottom, animating)`.
A known differing endpoint yields `available + currentBottom - endBottom`; all
bottom values include the already consumed navigation inset. An unknown endpoint
or an inactive animation yields no early target. `TerminalView.prepareImeViewport`
submits that final grid through the same `submitGrid`/repository owner as ordinary
measurement, capped at 80 rows / 240 columns. Equal targets send nothing; final
pre-draw verifies/corrects the early target, including cancelled/reversed motion.
Frozen-reading scheduling retains final measurement; screen type does not
determine early-target eligibility.
The signed endpoint calculation applies to both opening (smaller grid) and
closing (larger grid). `productionLayoutReportsTheTargetBeforeTheSystemImeFinishes`
drives the real system IME in both directions on both screens with the production grid layout:
assert one early final target before the View reaches its new height, and equality
with settled layout. A pure endpoint-math test alone cannot prove callback timing.

`current`/`pendingSource` are the latest prepared candidate; `drawnFrame`/
`drawnSource` remain the actual presentation. During IME/final-layout fencing,
`frameForDraw` retains the drawn compatible live geometry on either screen when
a differently sized candidate arrives. It also retains that baseline when a late
candidate does not match the latest requested grid. Same-size updates can continue.
Epoch loss, input suspension, a change of active screen, width/font incompatibility and selection /
history bypass retention. Do not bind or release the candidate source as though
it supplied the retained drawing; adopt it only when that candidate is drawn.
Use only the existing drawn and newest pending handles, with complete Canvas draws.

Host resize can publish caret-anchored rows before the TUI repaints: a 12-to-9
row shrink with the cursor on row 10 first produces old rows 02–10, whereas the
local bottom anchor displays 03–11. Exact diff still shows the one-row jump if
that intermediate image is admitted. Early scheduling plus retained animation
presentation overlaps this work with IME. Neither dimensions nor Active nor the
first output proves child-layout completion; do not add blank-content guesses or
an output debounce, and do not claim late/unmarked redraws are always atomic.
The same applies to a main-screen shell that performs no child redraw: 24 to 12
rows with cursor row 3 locally clamps at -3 rows, while the host retains rows
0..11. Final handoff must adopt the host rows/cursor even though their positions
differ. On growth the host restores available main history and otherwise adds
blank rows below; local bottom anchoring cannot redefine those semantics.

`TerminalView.onSingleTapUp` never requests IME visibility, including in a
plain shell, mouse mode and alternate-scroll mode. The bottom button calls
`TerminalView.setKeyboardVisible`; showing requires input readiness and native
View focus, hiding remains available while input is suspended. Observe
`WindowInsets.isImeVisible` for its action/accessible label and Back priority;
a floating keyboard can be visible with zero bottom inset. Do not guess an
input region from terminal glyphs/cursor coordinates or special-case programs.

`TerminalScreen` scopes `SOFT_INPUT_STATE_UNCHANGED` to its Activity window with
`DisposableEffect(LocalActivity.current?.window)`. Replace only
`SOFT_INPUT_MASK_STATE`, preserve resize/other bits, and restore the prior state
bits on disposal. A focused terminal with the default `STATE_UNSPECIFIED` and
`ADJUST_RESIZE` can trigger Android's `SHOW_AUTO_EDITOR_FORWARD_NAV` when its task
returns, even after the button or Back successfully hid the IME. Keep native
editor focus for hardware input and the existing explicit button calls; do not
clear focus or hide IME unconditionally on resume. `keyboardDismissalSurvivesTaskResume`
backgrounds the real task, returns to the same Activity, and observes a settled
visibility interval. It covers both dismissal paths, repeated returns, retained
input focus/Session/epoch, visible-state resume, input, and policy restoration
after leaving Terminal. Virtual keyboard event injection can make Gboard request
its own IME display (`SHOW_SOFT_INPUT_FROM_IME`); distinguish that from a window's
automatic show request when diagnosing a failure.

Disable local double-tap recognition: every completed tap, including rapid
consecutive taps, follows the same ownership rule. At the live synchronized
grid, declared mouse reporting routes completed taps
and vertical drag wheel steps to the child. Alternate-scroll uses cursor steps
only when declared on the alternate screen without mouse capture. Capture one
gesture source/owner/cell at DOWN; long press releases child ownership and starts
local selection without an earlier remote press. Selection and pinned history
own local scrolling. UP/CANCEL/background/detach retire held sources; mode/epoch
changes cancel child input for the rest of the gesture. Kotlin sends typed intent
and keeps pointer events in the bounded input queue, closing held sources even
when events are discarded. Never encode mouse escape sequences in Kotlin.

After Copy or cancellation at the bottom, restore the current live frame and
declared child pointer mode without keyboard input, a new sync or an epoch change.
The tap that dismisses selection remains local; the following tap/wheel reaches
the child. Cover this ordering explicitly in mouse, alternate-scroll and real
TUI regression tests: an intervening text/key command masks a broken exit by
triggering its own history-to-live synchronization. Clearing a selection in
older content preserves the reading position.

InputConnection owns local preedit and commits Unicode once. Its Editable is the
single source for text, selection and composing spans: initialize/reset selection
to `(0,0)`, use BaseInputConnection's cursor/replacement/deletion semantics, and
derive EditorInfo, context queries and cursor anchors from those same spans.
For example, `setComposingText("n", 1)` must expose selection `(1,1)`, composing
range `[0,1)`, before-cursor text `n`, and no after-cursor text or host input.
Replacing with `ni` must replace the composing range, not append or commit `n`.
Publish `updateSelection` with the actual composing range at outermost batch
completion; do not publish intermediate cursor anchors inside a batch.
Doubao's display-in-editor Pinyin path exposed this contract: missing selection
spans returned empty before-cursor context and triggered premature finalization.
Do not special-case an IME or suppress legitimate `finishComposingText` to hide it.

Candidate commit applies the platform replacement locally before queue admission.
Rejected admission preserves text, selection and composing spans exactly. A
`SpannableStringBuilder(existing)` snapshot drops Android's `NoCopySpan` markers,
including selection/composition; copy with an empty builder's `append(existing)`
when rollback must retain them. Successful admission clears text/spans and resets
selection; repeated explicit finish must not submit twice. Local surrounding
deletion respects selection/composing boundaries and code-point requests; only
an empty local buffer routes deletion to terminal keys.

Each connection
captures its creation input epoch; old callbacks and editable buffers cannot
commit into a newer synchronized attachment. Copy captures the Repository
selection version and attachment epoch, rechecking after FFI extraction before
any clipboard effect. Cancel/geometry/navigation changes retire pending Copy.
Healthy resize from Active Live preserves the native input epoch, existing
InputConnection, preedit and shortcut readiness; it shows no reconnect spinner.
Its independent geometry generation retires stale pointer/selection/Copy work,
including A-B-A. Initial attach, reconnect, lease loss/end and history-return
retain their real fences. Repository text/key/delete queue admission returns
Boolean: InputConnection reports false and retains preedit on queue rejection;
admitted same-attachment stale-epoch work reports `input_not_ready`. Never clear
composition or claim success merely because a fire-and-forget enqueue ran. App background visibility only stops
gesture animation/speculation, not the application connection. Navigate Home by
explicit detach; fence asynchronous attach/create/list results with their owning
navigation/attachment epoch so Back cannot be undone by a late response.

Store the latest desired viewport synchronously in `AppRepository.measure`.
The conflated size channel is only a wakeup for one resize submitter. A measurement
received while `connectTerminal` awaits must be reapplied when that exact handle
is adopted; a consumed signal with no terminal must not erase desired geometry.
Never put an older captured viewport back into a channel over a newer measurement.
Fence resize failure reporting by attachment epoch/handle. `AttachGeometryTest`
changes geometry while attach is suspended and checks the actual host's final
rows/columns; run alone with `-e attachGeometry 1` before UI startup.

Development builds log native terminal state transitions and typed operation
error codes under `ZtermState` for connected-device diagnosis. Log only state,
code, exception type and grid dimensions; never payloads, text, credentials,
identity seeds, host addresses or exception message trees. No per-frame log or
release diagnostic UI is added.

## 4. Validation and error matrix

| Case | Required behavior |
| --- | --- |
| Camera denied permanently | Settings entry; gallery/manual still available |
| Picker cancel / unreadable image / invalid ticket | Scanner retained, concise error/retry |
| Manual invalid credentials | Keep editable field; close/Back/outside dismiss |
| Storage failure | No successful saved-host claim or regenerated seed |
| More than 32 hosts / state above 256 KiB | Reject before replacing valid stored document |
| Actual disconnection | No queued input replay; preserve last complete pixels; clear route/RTT |
| Missing saved Session, successful empty list | Default attach main, persist returned ID |
| Failed Session list or unknown mutation outcome | Show failure; no blind default creation |
| Theme/locale recreation | Keep application identity, Session and unchanged-geometry selection |
| Different APK signature | Android rejects update; do not silently uninstall user data |
| Known live IME endpoint on either screen | Submit changed final grid early; verify it at settled layout |
| Differently sized candidate during IME / obsolete resize target | Retain compatible drawn frame/source; native installation and ACK still progress |
| Visible caret would move above the top | Clamp the common upward pan at the caret's top edge |
| Candidate changes active screen or input authority | Bypass retention; never carry old-screen presentation into the new authority |
| Unknown IME endpoint / incompatible retained source | Use final measurement / ordinary current-frame drawing |

## 5. Good, base and bad cases

Good: long-press a glyph, drag through three screens, use native Copy and preserve
exact Unicode; background/recreate the Activity and resume the same Session.
Base: initial settings follow system, and entering a host with an empty live
list opens default main. Bad: creating on a failed list request, taking over an
occupied Session implicitly, rebuilding the native runtime on rotation or calling clipboard APIs
merely when the scanner opens.

## 6. Tests and assertion points

Run Android lint/JVM checks, bridge/identity instrumentation and focused real-host
tests. `ReconnectRecoveryTest` uses a fresh ticket/disposable host with
`reconnectFixture=1`; optional `daemonRestart=1` coordinates `reconnect-ready` /
`reconnect-restarted` cache markers. It checks stale recent/default creation,
actual restart/Retry, saved identity, live reuse, one/multiple/occupied candidates,
input and Back fencing. `ConnectionStatusUiTest` checks fixed English route labels
in all language settings, narrow layout and connecting/unknown/reconnect/end/sync
projection. Do not treat a compiled APK as runtime evidence. `TerminalUiTest` drives production IME/gestures, edge selection and system
Copy, verifies three measured screens, all nine locale/theme pairs, persistence,
overlay dimensions, cache resource bounds and Activity retention. IME geometry
acceptance requires a full software keyboard: Gboard's physical-keyboard side
strip can be shown with zero bottom inset. Verify both visibility and measured
row changes, waiting for terminal window focus after dialog dismissal.
`TerminalInputConnectionTest` checks cursor queries, composing replacement,
Unicode deletion, nested batches, rejected admission and retired-buffer isolation
without a host. Keep the real-host explicit-finish/healthy-resize tests as well.
IME compatibility acceptance uses actual software-key taps and exact child bytes:
for Doubao 26-key Pinyin, test both candidate-bar and display-in-editor modes.
Typing `nihao` sends zero bytes; choosing `你好` sends exactly its UTF-8 bytes once,
without a Pinyin prefix or trailing space. A direct `setComposingText` test alone
does not establish compatibility with the IME's own query/callback sequence.
Assert content taps (ordinary, mouse and alternate screen) keep IME hidden and
the host viewport unchanged, while the explicit bottom button shows/hides IME.
Check full-grid IME show/hide produces only two resize epochs in total, and
zero-offset scroll never enters Synchronizing. A raw child fixture verifies
exact tap coordinates, one press/release, wheel directions, alternate-scroll,
cancellation, stale source rejection and long-press ownership. Isolated Herdr
acceptance additionally exercises actual pane history and tab clicking.
`ScannerUiTest` uses explicit host-generated ticket fixtures, Android MediaStore
and the real OS picker. Camera acceptance uses an explicitly positioned emulator
virtual-scene poster; it is distinct from Xiaomi camera acceptance. Tests close
only their own Sessions and delete their own ticket/image fixtures.

The local signing key stays outside Git. Record actual APK SHA-256/versionCode,
signature and installation/update evidence. Build-only CI does not establish
runtime or phone results; keep pending physical-phone rows explicit.

`TerminalScrollProfileTest` is opt-in with `scrollProfile=1` and an explicit
disposable `presentation-` host. It profiles real touchscreen injection, warmed
history and system FrameMetrics; it is not a universal FPS gate. Inject touch
through Android's input dispatcher from a separate thread. Direct dispatch from
a Choreographer animation callback changes `postInvalidateOnAnimation` timing.
Use actual Canvas pixels for delayed-row displacement correctness; a posted
OnDraw observer can mix a new desired position with older committed geometry.
Do not infer phone frame rates or natural content-stall counts from that sampling.

`TerminalRenderingTest.hiddenCursorGridMovesLocallyAndMatchesDelayedResizeInBothDirections`
holds remote frames while changing View height, checks actual moved pixels and
delayed A-B-A handoffs. `liveScreensKeepContentBelowVisibleCursorAboveToolbar`
checks a visible caret with two footer rows at intermediate/final local heights
and pixel-identical delayed resize on both screens.
`visibleCaretNeverLeavesTheTopEdgeOnEitherScreen` includes high carets, reversal
and a subcell clipped View. Its hardware row-content and both-screen height tests verify
zero new recordings for equal moved rows, one for a changed row, complete-frame
pixel equivalence and dimension/cache-loss invalidation. `rowRecordingCount` is a
cumulative local display-list count for these checks, not a physical refresh/FPS
metric. Newly exposed rows or content actually changed since its last display
may need recording again; a previously cached offscreen node is not guaranteed
to survive. Keep whole-IME/real-TUI visual acceptance separate from these fixtures.

`blankCellsKeepBackgroundsDecorationsAndAdjacentGlyphs` compares styled ASCII
spaces with empty glyphs at 8/12/16 sp, including all underline modes and adjacent
wide/combining text. It also requires a combining mark attached to a space to
remain visible. Painter microbenchmarks need matching warmup and a dense-text
control; a faster fresh-process sample alone does not establish a device speedup.

`hardwareRowLayersMatchDirectPaintingAndKeepSharedRowsSeamless` compares direct
hardware painting and the production row renderer in the same frame. Check styled,
wide/combining and blank cells at integer shifts, repeated equal rows sharing a
single entry, one changed row, explicit clear/recovery and solid fractional row
joins. Do not use antialiased direct rectangle seams as the fractional background
oracle. Pair animation-scoped RenderThread timings with foreground GPU memory;
stopped-window cache dumps and whole-process cumulative jank counters are not
comparable live animation measurements.

`resizeSnapshotCannotInterruptEitherMovingScreen` includes the actual
caret-anchored resize-only row sequence before the final repaint on both screens. The hardware
height-change test checks unchanged pixels/recordings across that intermediate
candidate and final adoption. `knownImeTargetSubmitsEarlyOnceAndFinalLayoutCorrectsIt`
checks deduplication, intermediate-measurement fencing, correction and reversal.
`reversedImeKeepsTheDrawnGridUntilTheLatestTargetArrives` holds an obsolete-size
candidate after a settled reversal, then verifies current changed content resumes
when the requested size arrives.
`authoritativeResizeStillWinsWhenTheFinalLayoutDiffers` separately checks actual
host high/bottom-caret resize sequences and growth with/without history. Hold the
early intermediate while animating, then require equality with a freshly drawn
authoritative frame; do not assert equal local/final pixels when layouts differ.
`imeRetentionNeverCrossesScreenOrInputAuthority` checks both screen-switch
directions, input-epoch replacement and suspended input during a pending resize.
`productionLayoutReportsTheTargetBeforeTheSystemImeFinishes` uses the production
Compose grid, native View, decor animation owner and actual system IME; assert
the target is submitted while the visible grid is still larger and equals the
final measured rows. It does not claim remote TUI completion within that interval.

## 7. Wrong versus correct

Wrong: infer that unchanged display lists avoid all glyph work or that low
`rowRecordingCount` proves smooth movement. Correct: measure RenderThread replay,
retain bounded row pixels where justified, and verify actual pixels and GPU cost.

Wrong: count reused text nodes and infer that resize cannot visibly jump.
Correct: include the resize-only semantic snapshot in the complete presentation
sequence, preserve the moved baseline during IME and adopt the latest prepared
source only with its actual drawing.

Wrong: use alternate screen as a proxy for TUI when selecting keyboard movement
or early resize. Correct: apply one live-screen policy, preserve explicit
screen-change fences, and distinguish local movement from authoritative layout.

Wrong: skip local movement when a TUI hides its cursor, or clear text nodes whenever
geometry changes. Correct: move the retained live grid using its explicit anchor,
reuse equal content across row positions, and retire only the appropriate coordinate
authority. Source/Copy correctness never comes from cached visual equality.

Wrong: release frame-source objects whenever Compose skips a frame, before the
native View has retained what it drew. Correct: Repository synchronous observers
and independent pending/drawn handles define explicit ownership.

Wrong: retry a remembered missing ID forever or treat a network error as an empty
host. Correct: resolve existence from a successful live list and use the shared
default-main attachment only for an empty result.

Wrong: copy every `.so` left in Cargo's target directory. Correct: copy only the
current bridge artifact, then verify every packaged library's ELF/zip alignment.


## Formal APK publication

Android is now part of the exact-main CI/release inventory. Build/lint/JVM and
16 KB checks remain distinct from arm64 emulator/device runtime evidence.
`tools/android/release.py` owns deterministic SemVer-to-versionCode allocation,
APK package/embedded-source/floor inspection and protected platform signing.
Gradle tracks source authority as an input to avoid stale generated metadata or
native libraries. Public package/certificate match prior acceptance builds;
development `.dev` uses its separate debug signer. See `docs/releasing.md` and
backend distribution-lifecycle.md for the complete authenticated asset inventory.

Presentation evidence owners: `AttachGeometryTest.integralEndpointsPreserveCursorAndInterpolateBelowToolbarRemainder`
asserts endpoint arithmetic, smooth interpolation and the clipped minimum;
`NativeTerminalTest` checks retained input plus stale A-B-A coordinates;
`TerminalUiTest` checks real keyboard/IME/selection behavior. Select a disposable
host with `hostName` and an explicit emulator serial; build success is not visual
or Chinese-composition acceptance. `presentation_fixture` creates independent
host state and refuses to discover/autostart the user's daemon.

## Retained file uploads

`TerminalUploads` is Application/Repository-owned, reserves native input pause
before Image/File selection, stages one ContentResolver stream into private cache,
and observes metadata-only `NativeUploadState`. `TerminalUploadUi` owns only the
launcher/dialog presentation, with first photo and second paperclip buttons
opening the respective picker directly. Saved launcher IDs prevent duplicate relaunch after
Activity recreation; observer startup includes already-published completion.
Pause uses a separate Repository input version and native odd/even generation,
not a NativeFrame input epoch change. Preserve IME preedit and healthy resize.
See [Single-file Upload](../backend/file-upload.md) for limits, errors, cancellation,
API signatures and `UploadUiTest`/`NativeUploadTest` assertion points.

`UploadPickerUiTest` takes `uploadPickerHost=upload-...` for a test-owned saved
host. It drives both actual OS pickers, Back cancellation, repeated launch and
same-Session/IME-epoch retention. Advance Compose's test clock with `ui.waitUntil`
when waiting for the launcher LaunchedEffect; coroutine sleep alone can leave
the effect unexecuted in an otherwise healthy app. Preview restarts must not
leave external picker Activities pointing at a destroyed parent Activity.

Distinguish Android `KEYCODE_BACK` injection from the emulator sidebar's hardware
Back route. Ask which input was used when a user reports Back failure and preserve
the current task stack before restarting. With VirtioInput, an AVD configured
with `hw.keyboard=no` has no virtual keyboard to receive sidebar key events.
Enable the AVD keyboard device and cold boot for a hardware-key preview, keeping
`show_ime_with_hard_keyboard=1` for software IME checks. Verify the sidebar itself
and guest input events; a passing instrumentation test cannot prove this path.

## Ordinary system notifications

[Terminal Notifications](../backend/terminal-notifications.md) owns the shared
OSC 9/777 and kind-325 contracts. AppRepository owns one transient native-event
consumer alongside, but separate from, frame collection. Fence repository and
native connection generations before invoking TerminalNotifications; recreation
and visibility changes never add consumers. Retirement cancels/joins it before
closing the handle. Kotlin owns the stable channel, ordinary notification fields
and Settings permission flow; Android OS state is the permission authority.
Disabled/denied notifications are dropped without ending the connection or
retaining content for a later grant. This adds no foreground service or push.

## Settings and application updates (2026-09-20)

### 1. Scope / trigger

Settings owns the fixed GitHub link, manual update action and terminal notification
switch. The user approved one silent cold-launch check, optional installation and
a 24-hour same-version reminder interval. This extends the original publication-only
Android scope. Terminal work never waits for a check.

### 2. Signatures / owners

- `ZtermApplication.updates: AppUpdates` owns jobs and `StateFlow<UpdateState>`;
  Activity/Compose never owns the network operation.
- `UpdateSource.check(): UpdateCandidate?`, `download(candidate, file, progress)`
  return verified candidates/downloads or stable `UpdateFailure` codes.
- UniFFI exports `validateAndroidUpdateBuild`, `validateAndroidUpdateTag`,
  `verifyAndroidUpdate`; opaque `NativeUpdate` owns authentication authority.
  Its presentation fields cannot reconstruct an authorized candidate.
- `AppRepository.saveUpdateReminder(UpdateReminder)` and
  `setNotificationsEnabled(Boolean)` use the existing serialized atomic state save.

### 3. Contracts

- `AppUpdateHost` starts after an initialized, resumed UI frame. The process gate
  is satisfied by startup or a prior manual check. Manual intent promotes an
  in-flight check without a second fetch. Automatic no-update/failure is silent;
  manual results use native `Toast.makeText`, consumed once.
- Automatic offers wait for Home/Settings, resumed lifecycle, no busy/error state,
  and no modal/permission blocker. They remain pending during Terminal/Scanner.
  Downloads start only from the offer's confirmation. No periodic work exists.
- Optional `state.json.updateReminder = {version, dismissedAt}` stores one bounded
  version/time pair. Matching automatic offers are suppressed for `0 <= age < 24h`;
  future timestamps are ignored. Newer versions and manual checks bypass it.
  Obsolete version records are harmless and replaced on the next dismissal.
- `preferences.notificationsEnabled` defaults to true for old files;
  `notificationPermissionRequested` defaults to false. Missing/malformed reminder
  data does not discard hosts/preferences. Identity encryption remains unchanged.
- Notification Off gates posting immediately; persistence commits through the
  existing mutex. In-flight saves retain this intent until completion. Save failure
  restores the committed gate and reports the existing storage error. Effective
  switch state is app preference AND OS app/channel permission; refresh on resume.
  Permission grants enable only a pending explicit request, preserving saved Off.
- HTTP discovery is fixed to the official repository, bounded to 256 KiB; immutable
  signed metadata is bounded to 64 KiB each, detached signatures to 64 bytes.
  HTTPS-only redirects, 60s check/10m download budgets, and cancellation-driven
  socket disconnects bound work. APKs are capped at 128 MiB, streamed to private
  cache, hashed by Rust, then inspected for package/version/code/minSdk/certificate.
- Installer content URIs expose only `cache/updates/`, with transient read grants.
  Unknown-source permission has explicit guidance and a return check. Consume Ready
  into HandedOff before launch; recreation cannot relaunch. Installer return is not
  reported as successful installation. Handed-off files survive for Android reads;
  older-than-24h owned files are cleaned on a subsequent download.
- Canceled partials and native handles are cleaned inside the NonCancellable block;
  code after a dispatcher-changing cleanup can be skipped by prompt cancellation.
- `WideStatusCard` fixes failure/retry width at available width minus 48dp (max560dp),
  minimum284dp, radius28dp, padding24dp; update dialogs use minimum324dp. Content
  scrolls for compact height/large fonts. Keep Retry/Sessions/takeover semantics.

### 4. Validation / error matrix

| Condition | Outcome |
| --- | --- |
| Development package, signer or source identity | `update_unsupported` before network |
| Untrusted metadata/APK or contradictory version ordering | `update_invalid`, never installer |
| Current/older authenticated release | manual `update_latest`; automatic silent |
| Network/timeout or rate limit | `update_network` / `update_rate_limit`; automatic silent |
| Disk failure / missing installer | `update_storage` / `update_install_failed` |
| Source permission denied | Remain at explicit guidance; Cancel deletes unhanded file |
| Notification OS block | Effective Off; request permission or explain system settings |

### 5. Good / base / bad cases

Good: manual tap joins startup, displays one verified offer, then confirmation starts
one transfer. Base: current version on startup leaves UI untouched. Bad: failed
signature presented as latest, automatic downloading, or app-drawn Toast.

### 6. Required tests

`AppUpdatesTest` covers one process gate, silent/manual outcomes, promotion, reminders,
clock skew, cancellation cleanup, consent and single-use handoff. `IdentityStoreTest`
covers migration and persistence. `SettingsUiTest` covers switch, GitHub intent,
locale/theme/footer and real permission denial/grant. `UpdateUiTest` covers modal
deferral, consent, responsive card geometry and FileProvider confinement.
`UpdateInstallTest` is opt-in (`installerFixture=1`) on a disposable install with
source permission initially denied; it exercises real settings return and a matching
test APK in the system confirmation. Reset install appops from the host afterward:
revocation kills the instrumented process. Official upgrade acceptance still needs
a genuinely newer protected release; fixture handoff is not production upgrade proof.

### 7. Wrong versus correct

Wrong: an Activity launches its own startup job or trusts GitHub's version JSON as
installation authority. Correct: observe the application owner; Rust authenticates
both release signatures and checksum bindings before offering a candidate, then
Android independently checks the downloaded APK and delegates final installation.

## Application title and link actions

TerminalStatus carries applicationTitle independently of saved Session names.
The fixed-height header uses session · application title, omits an empty or
identical suffix, and truncates to one ellipsized line. Terminal title changes
never invoke rename/storage/list mutation.

Open link shares the existing source-pinned selection: native lookup must find
one validated HTTP(S) target throughout the selected span, and AppRepository
fences delivery with both selection version and attachment epoch. Menu availability
stores only a boolean; clicking rechecks the current native target. Copy is
unchanged. ACTION_VIEW is a deliberate system handoff; absent handlers produce
a localized message without recording the URI.
