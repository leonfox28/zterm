# Android architecture, implementation and rendering review

Reviewed on 2026-09-09 against `a2b518e61544642bb4537935e8fd6c192dff607b`
(main, v0.1.31). Product sources were initially clean and remain unchanged.
This is a source-backed review with focused native tests, not new phone visual
or performance acceptance. See [architecture and alternatives](../design.md)
and [validation plan](../implement.md).

## Assessment

The main architecture is appropriate: the host owns terminal semantics, the
shared Rust client owns synchronization, native Android owns bounded semantic
navigation, and Kotlin owns platform presentation/input. Explicit drawn sources,
separate input/geometry epochs and latest-frame delivery address real lifetime
problems. There is no evidence supporting replacement of Compose, Canvas,
Alacritty or Iroh as the first action.

The most promising diff optimization is preserving unchanged row content through
the Rust-to-Kotlin path. Current network row patches and RenderNode reuse are
real, but substantial copying/conversion sits between those two optimizations.
Three local implementation issues also deserve focused follow-up. They are not
proof that the whole architecture is wrong.

The user's later clarification narrows the current priority: the TUI pauses after
keyboard arrival. The [focused resize investigation](tui-ime-resize.md) establishes
late resize submission and broad row-cache invalidation from current source, and
records their subsequent local-movement-first requirement. The core proposal is
to reconcile against the actually moved display with row reuse across changed
ordinals. Early remote resize complements that behavior; broader shared row
ownership remains measurement-driven. Current device timing remains unmeasured.

## Findings index

P2 means a normal-priority correction or bounded optimization; it does not imply
a crash or data-loss finding. Evidence categories below deliberately distinguish
executed reproduction, source-confirmed schedules, platform incompatibility and
historical observations. No P0/P1 correctness defect was established in this pass.

| ID | Priority / category | Finding | Evidence |
| --- | --- | --- | --- |
| IMPL-1 | P2 / local defect | Appearance changed during pending attach can be lost | Source-confirmed async schedule; device reproduction pending |
| IMPL-2 | P2 / local defect | A cancelled child gesture can continue as local history scrolling | Source-confirmed event path; instrumentation reproduction pending |
| IMPL-3 | P2 / compatibility defect | API 26–27 hardware underline drawing uses unsupported line PathEffect | Current code plus official support table; pixels pending |
| PERF-1 | P2 / optimization | Cursor-only deltas still deep-copy rows; native live capture and FFI add further full-grid work | Public-API allocation probe plus source trace; device cost unmeasured |
| VIS-1 | Open acceptance gap | Keyboard-close history refill continuity remains unproven on current main | Historical observation; no current reproduction |
| RESIZE-SCHEDULE | Priority from user follow-up | Final remote resize is submitted after IME animation; View height changes clear text-row caches | Source-confirmed mechanisms matching the reported timing; contribution to actual latency unmeasured |

## IMPL-1 — latest appearance is not reconciled after attach

**Trigger and impact:** use system appearance, begin a slow initial/default or
exact-Session connection, then change light/dark mode while connect is suspended.
Compose can adopt the new appearance while the eventual native terminal and its
base color observations retain the old one until another appearance change or
new attachment.

**Causal schedule:**

1. `attach` retires the old handle, leaving `terminal == null`, then evaluates
   `dark` for the suspended native connect call.
2. `setDark(newValue)` stores the new repository value, but its optional call to
   `terminal?.setDark(value)` has no target.
3. Connect returns the native actor created with the old value. Adoption reapplies
   a changed viewport only; it does not reconcile appearance.
4. Another `setDark(newValue)` returns early because the repository already holds
   that value. Native Active events reapply the actor's own old `dark`.

**Evidence:**

- `apps/android/app/src/main/java/io/github/leonfox28/zterm/AppUi.kt:50` drives
  appearance independently of connection completion.
- `apps/android/app/src/main/java/io/github/leonfox28/zterm/AppRepository.kt:148`
  stores the desired value and optionally submits it.
- `apps/android/app/src/main/java/io/github/leonfox28/zterm/AppRepository.kt:230`
  captures the connect arguments; `:241`–`:246` adopt and reconcile geometry only.
- `crates/android/src/terminal.rs:698`, `:727`, `:861` retain/reapply actor appearance.

**Owner and recommendation:** local attachment-adoption contract in Repository.
Apply the newest desired appearance to the adopted exact handle, following the
existing latest-viewport rule. No wire change is needed. Validate a held attach
with appearance changes, including Activity recreation; assert native and host
observations, not only Compose colors. This schedule has not been executed on a
device in this review.

## IMPL-2 — gesture cancellation loses the original owner

**Trigger and impact:** on a main-screen mouse TUI with warmed history, start a
child-owned gesture, receive a frame disabling mouse reporting, let that frame
draw, then move the same finger without a new DOWN. Remaining movement can start
local history browsing instead of staying cancelled.

**Evidence:** `TerminalView.kt` in
`apps/android/app/src/main/java/io/github/leonfox28/zterm/`:

- `:112`–`:121`: DOWN captures child mode/source.
- `:123`–`:134`: `onScroll` uses local scrolling whenever `childGesture` is false.
- `:247`: a pointer-mode change invalidates coordinates.
- `:266`–`:270`: invalidation clears `childGesture`, without a cancelled-owner
  state or cancellation of GestureDetector's event stream.
- `:323`–`:324`, `:437`–`:465`: once the new geometry draws, the temporary geometry
  guard no longer consumes subsequent MOVE events; GestureDetector can continue
  its existing gesture.

The native source checks still reject stale child input. The defect here is
local ownership changing inside one gesture, not bypassing the native lease.
The declared contract locks an owner until UP/CANCEL and cancels stale child
interaction for the remainder (`.trellis/spec/frontend/android-app.md`).

**Owner and recommendation:** local View gesture lifecycle. Distinguish Local,
Child and Cancelled ownership, and retire pending gesture timers when coordinates
become invalid. Do not infer local ownership merely from a released child source.
Validate the exact DOWN → mode update → draw → MOVE → UP schedule, then a new
DOWN. Include the same invalidation around pending long-press as a sibling case;
no observed long-press failure is claimed here.

## IMPL-3 — dotted/dashed underlines at the supported API floor

The app declares minSdk 26 (`apps/android/app/build.gradle.kts:27`). API 26–28 use
the direct cell painter, which can still receive a hardware Canvas:
`apps/android/app/src/main/java/io/github/leonfox28/zterm/TerminalRowRenderer.kt:14`.
The dotted/dashed branches always set `Paint.pathEffect` and draw a line:
`apps/android/app/src/main/java/io/github/leonfox28/zterm/TerminalView.kt:364`.

Android documents hardware support for line `setPathEffect()` starting at API 28.
Consequently the present fallback does not establish correct dotted/dashed
underlines on API 26–27. A software Bitmap test exercises a different path.
[Android supported drawing operations](https://developer.android.com/develop/ui/views/graphics/hardware-accel#support).

**Owner and recommendation:** the cell painter's API compatibility boundary.
Use explicit bounded dash/dot geometry or an appropriate local fallback on the
affected APIs, then compare hardware pixels with API 28+ and software rendering.
Do not change the declared API floor or disable all terminal acceleration as an
unreviewed shortcut. Actual device pixels were not collected this turn.

## PERF-1 — diff information stops short of the expensive projection path

**Executed probe:** a synthetic 53×56 grid containing nonempty cells is created
through the actual public `TerminalModel` API. A cursor move changes no rows.
The actual core `TerminalSurfaceDelta::candidate` produces the correct final
surface, but allocates independent buffers for every row and nonempty cell text.
The probe uses distinct live allocations, not elapsed-time estimates.

```text
cursor-only: row_patches=0, equal_rows=53, copied_row_buffers=53, copied_nonempty_text_buffers=2968
one-cell change: row_patches=1, replacement_cells=56
```

Reproduce with [run-delta-copy-probe.py](run-delta-copy-probe.py), which compiles
[delta-copy-probe.rs](delta-copy-probe.rs) against the current repository libraries.
It creates no host process, PTY, connection or device state.

**End-to-end cost trace:**

1. Host `model.rs:365` projects a complete screen, and `:390` compares rows to the
   attachment checkpoint. A changed cell currently replaces its complete row.
2. `crates/core/src/terminal.rs:584` validates the delta, validates the baseline,
   clones it at `:595`, installs row replacements and validates the candidate.
   Even a cursor-only delta takes this whole-surface path. It is transactional
   and correct; the observation concerns work and allocation.
3. `crates/android/src/terminal/navigation.rs:221` recaptures live rows whenever
   the source revision changes, including cursor-only revisions, cloning them
   at `:227`. The normal live path therefore loses immutable row identity again.
4. `crates/android/src/terminal.rs:1287` reuses `content_generation` only when the
   page pointer/colors/theme and window match. A new live capture produces a new
   generation; `:214` and `:1213` project the exported rows into cell DTOs.
5. `apps/android/app/src/main/java/io/github/leonfox28/zterm/AppRepository.kt:251`
   resolves that new generation off Main. This protects Main from conversion but
   does not eliminate conversion or allocations.
6. `apps/android/app/src/main/java/io/github/leonfox28/zterm/TerminalRowRenderer.kt:35`
   can then discover unchanged row content and reuse its display list. Reused
   scrolling rows benefit; rebuilt but equal live row DTOs still require equality
   work before reaching this reuse.

**Recommendation:** first measure and preserve unchanged row content identities
through candidate application, native capture and projection, keeping authority
epochs separate. Cursor/style/selection overlays must still update. Avoid per-cell
FFI calls; deliver bounded changed rows or share a bounded immutable row window.
Keep transactional validation, exact frozen Copy and color-wide invalidation.
Do not simply keep an old source revision or skip required deltas.

The probe proves copying, not that this is the dominant device bottleneck. Native
capture/FFI counts above are source-derived, not probe measurements. No percentage
performance gain is predicted. `design.md` compares alternatives and their costs.

## VIS-1 — preserve the previous acceptance gap accurately

The earlier presentation task observed newly exposed top rows initially empty
and then filled by history during keyboard close. It explicitly did not establish
a whole-screen clear, universal flicker removal or a controlled before/after
measurement. The later scrolling task accepted phone scrolling but preserved this
separate gap.

Evidence:
`.trellis/tasks/09-07-terminal-presentation-continuity/research/runtime-acceptance.md:106`
and `.trellis/tasks/09-07-terminal-presentation-continuity/prd.md`.
This is historical evidence, not a reproduced v0.1.31 defect. Recheck full
transitions on current code before choosing a fix. Frame endpoints, snapshot ACK
and the presence of a complete new-size grid do not prove that an unmarked child
has finished its layout. Marked output already has bounded host publication.

## TUI coverage and fidelity

| Scenario | Current mechanism / evidence | Remaining limit |
| --- | --- | --- |
| Same-size sparse text and styled blanks | Complete-row deltas; fresh-snapshot equivalence tests passed this turn | Sparse live projection cost is unmeasured |
| Cursor-only update | Cursor metadata preserved; zero row patches verified by probe | Full row copies remain; Android cursor is a fixed translucent block |
| Wide glyphs, combining text, rightmost wide fragments | Host semantic projection and orphan normalization; snapshot/delta tests passed | Font fallback, emoji clusters and actual glyph clipping require pixel evidence |
| Underline variants, inverse, indexed/RGB colors | Native resolution and Canvas painter exist | API 26–27 dotted/dashed compatibility finding; palette/style-only pixel matrix pending |
| Alternate-screen resize | Full snapshot on screen/size change; orphan-wide regression passed | Whole-transition application redraw evidence pending |
| DEC 2026 output; A complete then B partial | Held read/query/boundary tests passed this turn | No added new device TUI run; unmarked output can expose intermediate states |
| Interrupted marked batch | Existing 150 ms bounded recovery, reset/resize handling | Quiet-PTY/EOF driver tests were not rerun in this pass |
| Cached history and selection | Native exact-source, bounds, return-to-live tests passed | Current Android pixel suite not rerun; cancelled-gesture schedule is missing |
| IME resize and healthy input | Separate geometry/input epochs; prior native/UI evidence | Current full-transition and vendor IME acceptance not established |
| Reconnect / background / Activity recreation | App-owned runtime, final frames, no disconnected input replay | Native teardown/disconnect tests passed; current device/network transitions unperformed |

Strike, conceal/hidden text, hyperlinks and graphics are explicitly outside the
advertised semantic subset (`.trellis/spec/backend/terminal-model.md:199`). The
current DTO also has no cursor shape/blink field (`crates/core/src/terminal.rs:199`),
and Android draws a fixed block (`TerminalView.kt:303`). These are fidelity/product
scope limits, not evidence that Android accidentally dropped fields present on
the wire. Extending them would require reviewing the shared contract and both
clients; changing the Android painter alone cannot recover absent semantics.

## What the current diff implementation gets right

- Deltas carry explicit baseline/replacement revisions, complete replacement rows,
  modes/cursor/colors and validated geometry; gaps trigger synchronization.
- Screen/size changes resynchronize instead of applying incompatible row indices.
- Host parsing and eligible publication are separate under synchronized output.
- Android applies required updates before latest-frame display coalescing, and
  ACKs installed snapshots independently of drawing.
- Repository conversion is off Main; row windows are bounded and reused for
  same-content metadata; sources are retained explicitly before old handles close.
- API 29+ row display lists compare actual row content, check `hasDisplayList()`,
  and clear for font/geometry/lifecycle changes. Cursor/selection/preedit remain
  overlays. Android permits the system to discard an unused display list, so
  that check is necessary.
  [RenderNode reference](https://developer.android.com/reference/android/graphics/RenderNode#hasDisplayList()).

## Evidence collected this turn

| Command / observation | Result | Practical limit |
| --- | --- | --- |
| `cargo test -p zterm-android -p zterm-terminal --lib` | 17 + 13 passed | Native host tests, not Android graphics |
| `cargo test -p zterm-terminal --test synchronized_output --test terminal_snapshot_delta` | 3 + 6 passed | Semantic/publication tests, not physical display |
| Task-local delta-copy probe | Passed; output above | Allocation identity, not timing |
| Build wrapper `:app:lintDebug :app:testDebugUnitTest` | Build/Kotlin/lint succeeded; JVM task `NO-SOURCE` | No JVM behavioral tests executed |
| `adb devices -l` | No connected devices | No new emulator/phone instrumentation or screenshots |

Build used the pinned NDK 28.2.13676358 successfully; the tool printed warnings
about ambient NDK_ROOT/NDK_HOME pointing to 30.0.15729638. This review made no
machine-configuration changes.

The historical scrolling measurement reported gesture draw p50/p95 changing from
15.01/20.11 ms to 1.79/5.30 ms in one controlled emulator comparison. It explicitly
does not prove stable FPS, uniform deadline improvements or live TUI performance.
See `.trellis/tasks/09-07-terminal-presentation-continuity/research/android-scroll-optimization.md:80`.
Android's drawing and total frame durations measure different pipeline work;
measure both, with deadlines where supported, for follow-up comparisons.
[FrameMetrics reference](https://developer.android.com/reference/android/view/FrameMetrics).

## Proposed order of follow-up

1. Confirm and fix the three bounded implementation issues with their exact
   regressions; they do not require a renderer migration.
2. Capture current full-transition IME/TUI evidence and a live-update allocation/
   frame profile. Resolve VIS-1 from observed coverage and geometry.
3. Optimize immutable row sharing and projection scheduling if the profile
   confirms PERF-1 matters. Use semantic equivalence and exact Copy as hard gates.
4. Consider host damage-assisted projection or finer wire patches only if host
   CPU or network bytes become the measured limiting stage.

No product edits, spec-contract changes, child-task creation, commit, installation,
daemon restart or release were performed. Review notes should not be promoted to
normative specs as if the proposed corrections already existed.
