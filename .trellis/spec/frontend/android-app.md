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
Rename/Delete only. Creation, destructive deletion and occupied takeover remain
explicit. Terminal has no overflow, app-bar keyboard button or history/live/line
count banners. Eight shortcuts and a rightmost keyboard show/hide button stay
above IME/insets during scroll and selection. Use the same TextButton, zero
content padding and equal-width 48 dp row slot for every control, including the
keyboard icon. Its 18 dp glyph inherits the button content color, so enabled,
disabled and pressed styling follows the neighboring shortcuts.

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

Repository resolves `NativeFrameSource.presentationRows()` off Main only when
attachment-local `contentGeneration` changes, then reuses the immutable row list.
Compose observes only `TerminalStatus`; the direct View observer owns content
frames. Kotlin row windows are bounded to three viewports plus one row, separately
from native cache budgets. API 29+ hardware Canvas reuses `TerminalRowRenderer`
RenderNodes for equal logical row/cell content, bounded to three viewports plus
one entry. Font/width/geometry changes, background and detach discard lists.
Check `hasDisplayList()` before reuse. API 26–28 and software Canvas use the same
cell painter directly. Do not build a full-history bitmap or cache cursor,
selection or preedit overlays into text nodes.

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

During IME animation, `imePadding` moves chrome and clips/pans the existing grid
locally. `terminalBottomRemainder` interpolates the endpoint remainders using
source/current/target consumed bottom insets; do not round every animation frame
to whole rows. Stable 17 px cells at available 1000/600 px give grids 986/595 px
and bottom padding 14/5 px; cursor row 39 pans to row 34 with no 9 px handoff. An Activity-owned `WindowInsetsAnimationCompat.Callback` on the decor
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

`TerminalView.onSingleTapUp` never requests IME visibility, including in a
plain shell, mouse mode and alternate-scroll mode. The bottom button calls
`TerminalView.setKeyboardVisible`; showing requires input readiness and native
View focus, hiding remains available while input is suspended. Observe
`WindowInsets.isImeVisible` for its action/accessible label and Back priority;
a floating keyboard can be visible with zero bottom inset. Do not guess an
input region from terminal glyphs/cursor coordinates or special-case programs.

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

InputConnection owns local preedit and commits Unicode once. Each connection
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
| Actual disconnection | No queued input replay; preserve last complete pixels |
| Theme/locale recreation | Keep application identity, Session and unchanged-geometry selection |
| Different APK signature | Android rejects update; do not silently uninstall user data |

## 5. Good, base and bad cases

Good: long-press a glyph, drag through three screens, use native Copy and preserve
exact Unicode; background/recreate the Activity and resume the same Session.
Base: initial settings follow system, and a host with no saved Session offers
explicit creation. Bad: app-owned album UI, tapping a host silently creating a
Session, rebuilding the native runtime on rotation or calling clipboard APIs
merely when the scanner opens.

## 6. Tests and assertion points

Run Android lint/JVM checks, bridge/identity instrumentation and focused real-host
tests. `TerminalUiTest` drives production IME/gestures, edge selection and system
Copy, verifies three measured screens, all nine locale/theme pairs, persistence,
overlay dimensions, cache resource bounds and Activity retention. IME geometry
acceptance requires a full software keyboard: Gboard's physical-keyboard side
strip can be shown with zero bottom inset. Verify both visibility and measured
row changes, waiting for terminal window focus after dialog dismissal.
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

## 7. Wrong versus correct

Wrong: release frame-source objects whenever Compose skips a frame, before the
native View has retained what it drew. Correct: Repository synchronous observers
and independent pending/drawn handles define explicit ownership.

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
