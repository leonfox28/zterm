# Attribute the final TUI drawing burst

The user defers the unreproduced tiny keyboard overshoot and asks to continue
other work. Continue the independently measured 14.58–26.92 ms final app frames
from `ime-phone-trace.md`. All device work remains on the owned API 37 emulator;
the phone and the user's existing host/session are unavailable for this work.

The stage owner is **undetermined** below Main's `Record View#draw()`. Source
inspection puts missing row display-list recording, dynamic overlays, drawn-source
viewport binding/release and input-method anchor publication inside
`TerminalView.onDraw`. Native `viewport_source` clones an immutable page handle;
that does not measure FFI/destructor cost or prove it negligible. A fixture with
null source handles cannot attribute source work.

Before a product correction, add temporary content-free Android Trace sections
to the existing `TerminalView` draw stages and the row cache's actual recording
branch. Use the existing `presentation_fixture` CLI to create a separate marked
host root and only owned sessions, then exercise real native sources through
the existing repository/View path during real IME opening/closing. Reuse current
pairing/Android test helpers and remove the diagnostic hooks afterward. The
measurement distinguishes row recording from source handoff and anchor updates;
it is not a phone FPS benchmark or a new profiling framework.

Expected temporary product files are `TerminalView.kt` and
`TerminalRowRenderer.kt`, plus one bounded opt-in Android diagnostic and this
task's research. The eventual fix remains unchosen until the dominant substage
is measured. Preserve prior early-target scheduling, actual drawn-source
authority, geometry/input epochs, compatible row reuse and the bounded GPU layers.
Do not expand to native row sharing, prewarming queues or another animation
owner without evidence that the existing boundary requires it.

## Owned emulator result

The diagnostic used `Zterm_ImeEnd37_0909` only, with real repository/native source
handles, an independent marked host at `/tmp/zterm-ime-stage-host`, and an owned
Session running the existing continuity fixture's SIGWINCH repaint. It uses an
alternate screen, stable bottom-relative row labels, synchronized output and a
visible caret with two footer rows. No user host or phone was operated.

Fixture preparation found a stale cached `presentation_fixture` binary (0.1.30);
it was rebuilt from the current 0.1.31 source. A preceding pairing attempt returned
`pair_outcome_unknown`; the explicit host device list confirmed the emulator's
exact persistent peer had been authorized. The diagnostic verified that same
client identity and seeded only this disposable host's known public address-book
record, then used normal authenticated requests. It did not replay the ticket or
change product pairing behavior. Both runs deleted their exact Session afterward.
The existing Espresso dependency failed to initialize on API 37 because it
reflects `InputManager.getInstance`; the temporary diagnostic used public
ActivityScenario and Instrumentation APIs without Espresso. No dependency or
platform API workaround was added to the app.

The first, normal-speed run passed four open/close pairs. Its eight IME animations
lasted 330–351 ms. Across 127 terminal draws, mean/p95 duration was 0.533/2.666 ms;
rows were 0.280/1.227 ms, source binding/release 0.095/0.530 ms, and anchor
publication 0.131/0.249 ms. The maximum source stage was 1.192 ms. Several new
remote frames arrived well after the IME endpoint, so this is not a reproduction
of the phone's end-coincident drawing burst. The four draws that recorded 18
rows cost 3.596–4.811 ms total, including 3.072–4.091 ms in the row stage.

The second run changed only the owned emulator's animator scale to 10 to control
ordering, then restored it to 1. Two open/close pairs passed; actual animations
lasted 2.884–2.891 seconds. This is an ordering probe, **not a performance comparison
or proposed product animation speed**. Candidate/drawn row counters show:

| Transition | Target-size candidate observed after start | Drawing switches after IME end | Handoff draw / row stage | Row recordings |
| --- | --- | --- | --- | --- |
| Open 1, 44 → 26 rows | 1,345.300 ms | 2.751 ms | 1.969 / 1.441 ms | 1 |
| Close 1, 26 → 44 rows | Candidate arrived after IME end | After late arrival | 2.860 / 2.644 ms | 18 |
| Open 2, 44 → 26 rows | 1,284.041 ms | 2.889 ms | 1.609 / 0.915 ms | 1 |
| Close 2, 26 → 44 rows | 1,705.014 ms | 1.801 ms | 3.098 / 2.847 ms | 18 |

For the three early-candidate handoffs, source work was 0.224–0.581 ms and anchor
publication 0.011–0.080 ms. These samples support prompt local adoption after
animation when the target is already ready. The row recordings are observed work;
they do not by themselves establish whether entries were new, evicted, changed
or discarded by the platform, nor justify calling them a diff correctness defect.
That distinction is the next narrow row-cache question if further work is needed.

Some anchor scopes outside the handoff were longer. The largest was 7.395 ms wall
time but only 0.058 ms of scheduled CPU time; other samples did consume CPU.
Therefore neither the largest wall time nor the aggregate mean justifies a
blanket claim that all anchor publication is expensive or negligible. The query
preserves wall/CPU attribution separately.

No new product correction is selected from this emulator result. It narrows the
known fixture's work to row preparation, without reproducing the phone's
6.41–12.43 ms Main `Record View#draw()` scopes or proving that a source-release,
anchor or scheduling change would resolve them. Existing early-target ownership,
source authority and row layers remain the comparison implementation. The tiny
keyboard overshoot remains separately deferred in `ime-end-motion.md`.

## Reproduction and cleanup

- Archived, removed diagnostic: `ime-draw-probe.patch`, containing the literal
  content-free Trace stages, optional row counters and opt-in owned-host test.
- Query: `ime-draw-stages.sql`; output tables: `ime-draw-normal-results.txt` and
  `ime-draw-early-results.txt`. The normal run predates the row counters; null
  counter results there are missing instrumentation, not failed geometry.
- Local traces: `/tmp/zterm-ime-stage.pftrace` (19,550,376 bytes) and
  `/tmp/zterm-ime-stage-early.pftrace` (36,286,672 bytes). Host/network timing,
  API 37 emulator, SwiftShader and deliberately slowed second run limit transfer
  of timings to the physical phone.
- Tests: `/tmp/zterm-ime-stage-test.txt` and
  `/tmp/zterm-ime-stage-early-test.txt`, each `OK (1 test)`.
- Temporary product hooks and diagnostic test were archived and removed, and
  the prior production files restored byte for byte before the clean build.
- Clean wrapper build (`assembleDebug`, `assembleDebugAndroidTest`, `lintDebug`)
  and `git diff --check` pass. The 49,999,725-byte development APK's SHA-256 is
  `b4fcabc2cb0c84da177c47db6fc20ade26818a77b93ded25ed7e359fd183c5cc`,
  identical to the already-checked row-layer comparison artifact. No repeated
  native/package matrix or phone installation is needed for an identical APK.
- Confirmed zero Sessions on the owned fixture, stopped that daemon, then
  removed its marked root (including identity and ticket). Both owned AVDs were
  stopped/deleted through the SDK tools. Existing user AVDs and daemon were left
  untouched. Research/trace artifacts remain; the broader task stays in progress.
