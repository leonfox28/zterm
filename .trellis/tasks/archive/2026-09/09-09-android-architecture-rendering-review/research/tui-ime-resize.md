# TUI pause after keyboard arrival

Baseline: `a2b518e61544642bb4537935e8fd6c192dff607b`, 2026-09-09.
This extends the [architecture review](review.md). It records the original
investigation and design. The subsequently approved first implementation and
focused emulator evidence are in [first-build.md](first-build.md); end-to-end
real-TUI performance remains unmeasured.

## User observation and conclusion

The user describes Herdr-like TUIs pausing **after the keyboard has reached its
open/closed position**, then refreshing. This narrows the primary investigation
from animation-frame cost to the end-to-end resize schedule.

The subsequent clarification establishes the main desired behavior: first move
the available local rows, then update only visual differences between incoming
remote content and the moved display. This must work independently of remote
response speed. Early resize remains complementary; it is not a condition for
local movement or visual reuse. The proposal recommends row granularity first.

The current code deliberately submits the final grid after IME animation ends.
Consequently, the remote resize request, PTY/model work, and the child's response
to that request cannot overlap the preceding keyboard animation. This is a
source-confirmed scheduling cost and a strong match to the report. The reported
app/host build, actual duration and dominant remote/local stage remain unmeasured.

There is also avoidable-looking local work: every View height change clears all
recorded text rows. Reusing those rows can reduce recording work, but cannot by
itself remove the remote work scheduled after animation completion.

## Actual stages and evidence

| Stage | Current owner and source | Implication |
| --- | --- | --- |
| Observe IME lifetime | `apps/android/app/src/main/java/io/github/leonfox28/zterm/ImeAnimationState.kt:11` | The Activity owner publishes running/completed events; it does not currently publish an early target grid. |
| Animate local available height | `apps/android/app/src/main/java/io/github/leonfox28/zterm/TerminalScreen.kt:65`, `:244` | Compose consumes system/IME insets and measures fixed chrome, terminal height and the interpolated row remainder. It already reads IME source/target insets. |
| Submit remote dimensions | `apps/android/app/src/main/java/io/github/leonfox28/zterm/TerminalView.kt:67`, `:274` | Measurement returns while IME animation or its final pre-draw is pending. Completion schedules a pre-draw that measures the final grid. |
| Deliver latest desired size | `apps/android/app/src/main/java/io/github/leonfox28/zterm/AppRepository.kt:72`, `:103`, `:159` | The repository uses a conflated size signal and current desired viewport. Preserve its existing bounds and attachment ownership. |
| Resize synchronization | `crates/android/src/terminal.rs:790`, `:852` | Native records latest desired size, advances coordinate authority, sends when active and reconciles another desired size on return to Active. |
| Resize the PTY/model | `crates/daemon/src/terminal_driver.rs:515`; `crates/terminal/src/model.rs:308` | Host resizes the PTY and model, then publishes a revision. It does not await the TUI's subsequent redraw. |
| Choose snapshot versus delta | `crates/terminal/src/model.rs:370` | Geometry changes require a snapshot for both shell and alternate-screen TUI. A different program name does not select another resize protocol. |
| Install and ACK | `crates/android/src/terminal.rs:866` | Valid snapshot installation and exact ACK are independent of Kotlin drawing. Keep this ordering so subsequent output can progress. |
| Project and display | `apps/android/app/src/main/java/io/github/leonfox28/zterm/AppRepository.kt:251`; `TerminalView.kt:279` in the same directory | Off-Main projection precedes frame delivery and Canvas submission. A model-size snapshot is not proof of completed child layout. |
| Reuse text drawing | `apps/android/app/src/main/java/io/github/leonfox28/zterm/TerminalRowRenderer.kt:35`; `TerminalView.kt:189`, `:272` in the same directory | The renderer compares actual row content, but geometry generation and every View size change clear its entries first. |

Conceptual timing, without invented durations:

```text
Current:  keyboard motion -> final measure -> remote resize/TUI output -> projection -> display
Proposed: keyboard motion -------------------------------> final geometry/handoff
             reliable target -> remote resize/TUI output -> prepared display candidate
```

Only remote work that actually finishes during the animation can be hidden by
overlap. Slow child output or transport can still extend beyond keyboard arrival.
Some TUIs substantially change their layout on resize; those changes still need
to be displayed. The shell/TUI visual difference does not establish that one
receives network deltas while the other receives snapshots.

## Main contract: compare against the moved display

Preserve the authoritative remote surface used for protocol application. Separately
retain the actually drawn rows and their current pixel placement, clip and source.
The keyboard changes that presentation geometry immediately using available local
content. It does not mutate the remote baseline or predict the TUI's semantics.

On a valid candidate, compare content after mapping it into the same viewport
geometry. For example:

| Visible slot | After local movement | Incoming target | Text recording |
| --- | --- | --- | --- |
| 0 | B (previously old row 1) | B (new row 0) | Reuse |
| 1 | C (previously old row 2) | C (new row 1) | Reuse |
| 2 | D (previously old row 3) | D* (new row 2) | Update |

These are illustrative row contents, not measured output. Comparing only old and
new semantic row ordinals would miss the reuse. Maintain a bounded displayed-slot
to render-content mapping and the known local movement correspondence. No general
text-editor diff/LCS or screenshot readback is required. Compare actual resolved
row content/render configuration; a hash can accelerate comparison but needs
equality confirmation. Update placements/clips and current source metadata even
when no text content changes.

RenderNode supports moving recorded drawing through properties without issuing
each original drawing command again. This supports the row-content/placement
separation on the existing hardware path.
[RenderNode reference](https://developer.android.com/reference/android/graphics/RenderNode).

Use a complete frame composition of background, reused/updated text and current
overlays. Display-list reuse reduces repeated recording; it does not promise that
Android never composites unchanged pixels. Do not skip required drawing to a
fresh Canvas on the assumption that its previous pixels survive. The compatibility
painter and cache-loss recovery must still produce the same visible result.

The same reconciliation policy can apply to shell/TUI, snapshots/deltas, slow
network and reversals. It cannot guarantee that the diff is always small: actual
whole-screen content or font/width changes may affect all rows. Missing newly
exposed rows require authoritative data. Unmarked TUI output can contain a real
intermediate clear; comparing it correctly does not identify a later intended
layout. Retain existing synchronized-output boundaries without inventing an
application-completion heuristic. These are limits of available information,
not application-specific exclusions from the local response policy.

Rows are the recommended first unit: existing text nodes already represent rows,
and keeping them reusable across movement addresses this behavior directly. A
later finer unit would be terminal cell runs covering complete glyph footprints,
including wide/continuation cells and combining text, rather than Unicode code
points. Its extra comparison/cache/submission cost requires a measured benefit.

## Supporting architecture details

### Complementary optimization: discover a reliable target early

Extend the existing window-level IME owner and grid measurement path instead of
installing a competing AndroidView insets callback. Track one local transition
identity and its latest desired target; this identity is scheduling state, not a
wire revision or input capability.

For a system-driven animation with a stable window and known end insets, derive
the target from the same measured chrome, inset consumption and cell metrics as
the final grid. For the current bottom-only transition, the existing geometry
helper already derives endpoint available height as:

```text
end_available = current_available + current_consumed_bottom - target_consumed_bottom
target_rows = max(1, floor(end_available / cell_height))
```

Use physical pixels from one consistent layout observation. The bottom terms
must represent the already-consumed navigation/IME union, not a second subtraction
of the keyboard height. Apply the repository's current viewport limits. Width,
top insets, font and chrome must still match the observation; otherwise recompute
from the full layout. Emit the deduplicated request outside Compose measurement.

Android documents end-state insets/layout before the animation-start callback,
which makes early discovery feasible for standard system animations. The current
Compose/AndroidView path still needs verification: callback order alone does not
prove that reading this child View's height yields its final height.
[Android IME animation guidance](https://developer.android.com/develop/ui/views/layout/sw-keyboard).

Submit one reliable target per unchanged intent, not every interpolated height.
At final layout, verify and send a correction only if the actual grid differs.
On reversal, preserve the existing in-flight protocol transaction and replace
the pending desired target. A reversal may legitimately require another resize.
The final measurement must not be suppressed merely because a predicted target
was already sent. Clear transition state on detach/recreation.

Interactive/cancelled animations can lack a settled endpoint. Android also
documents cancellation before onStart and special end-state rules for
application-controlled animations. Unknown targets must fall back to final
measurement; a previous keyboard height or visibility flag is insufficient.
Verify API 26–29 compatibility separately from native API 30+ callbacks.
[WindowInsetsAnimation callback contract](https://developer.android.com/reference/android/view/WindowInsetsAnimation.Callback#onPrepare(android.view.WindowInsetsAnimation)).

### Separate protocol progress from temporary presentation

Keep the last actually drawn source available for clip/pan during the keyboard
transition. Install all required snapshots/deltas and ACK them immediately under
the current protocol rules; prepare the newest eligible display candidate off
Main while the animation proceeds.

An early small-grid result must not suddenly replace a still-large visible region
with empty space. Adopt a target candidate at a drawing opportunity where its
mapping fits the current visible geometry, or at the final layout. Retained old
content may cover the intervening animation. At the settled endpoint, present
the newest eligible matching candidate promptly, then continue ordinary live
updates. If it is late, keep the available old content with the existing bounded
clip/pan rule while waiting for valid data; do not add a second settling timer.

Use at most the existing drawn source plus one newest pending presentation, not
a frame queue. Keep pinned data within the native budget. Late obsolete targets
must not restart an animation or overwrite the latest desired geometry; their
mandatory protocol effects still apply. Equal A-B-A dimensions alone do not
establish source identity or authorize stale input.

This retention is visual only. Actual geometry changes still retire stale mouse,
selection and hit-test coordinates. Preserve keyboard/preedit continuity on
healthy resize and existing hard barriers on reconnect/takeover. Source authority
must come from the installed/drawn source, never a reused RenderNode.

Neither the first new-size snapshot nor the first later output batch identifies
completed TUI relayout. DEC 2026 provides explicit batch visibility, not a resize
transaction identity. Preserve the host's bounded marked-output recovery. Do not
wait for a process-specific marker, guess from blank pixels, delay snapshot ACK,
or add an output debounce to the product.
[Synchronized-output semantics](https://github.com/contour-terminal/vt-extensions/blob/master/synchronized-output.md).

Growing a TUI cannot reveal content the client never received. Available retained
history can fill only the positions it actually owns; missing rows remain neutral
until authoritative data arrives. Continuous presentation does not mean inventing
the new layout or guaranteeing no intermediate layout for unmarked output.

### Preserve compatible text rows across resizing and snapshots

Split coordinate invalidation from text-render invalidation. A height change
requires updated clipping, positioning and hit tests; with stable width, font,
cell metrics and resolved row content, it need not discard every recorded glyph.

Keep a bounded cache whose render dependencies include row content, dimensions,
font configuration and resolved styles/colors. Compare rows at their actual
mapped positions, preserving reuse across changed ordinals; equality of ordinal,
row count or a hash alone is insufficient.
Real changes, including styled blank cells, must re-record the affected rows.
Font/width changes and lost display lists retain correct repaint behavior. Keep
cursor/selection/preedit as current-source overlays and retain the API 26–28 /
software painter. Detach/null presentation releases retained resources.

A complete network snapshot remains a valid correctness boundary. Local drawing
can compare its rows with retained text and reuse equal results. Thus transport
encoding and display reuse should be independent even across a height change.
Changing the wire format is unnecessary for this first improvement.

The existing PERF-1 full-row copy/projection finding remains a second opportunity:
share immutable rows through core/native/FFI only if stage measurements show that
work materially contributes to this pause. It reduces preparation cost but cannot
replace earlier resize scheduling. Do not combine it with an unmeasured renderer
or protocol replacement.

## Evidence still needed and acceptance

The existing neutral-TUI keyboard test waits for settled geometry and checks
selected endpoint patches (`TerminalUiTest.kt:157` in the Android androidTest
source directory). The real Herdr test at `:589` checks tab/pane interaction, not
keyboard-to-layout latency. Neither measures the reported pause.

Capture device monotonic events for animation start/end, target discovery,
resize submission, matching native installation, projection start/end and actual
frame submission. Record existing revision/attachment/geometry correlation and
host-local receive/model timings. Do not subtract unsynchronized host/device
clocks to claim one-way latency.

Use a neutral fixture with a known redraw counter and reported geometry to
identify its intended layout; distinguish those test markers from product
protocol. Repeated dimensions and queued old output must not be counted as a
completed new layout. Record a full video for Herdr and inspect the actual UI,
since its first new-size snapshot alone cannot supply that event.

For both opening and closing, compare the same device, IME, scene and network:

- Does a reliable target request occur before animation completion, with no
  requests for intermediate animation heights?
- Does keyboard-arrival-to-intended-layout latency improve across repeated runs?
  Separate target-ready-before-arrival cases from target-ready-after-arrival.
- When a matching prepared candidate is already available, is it submitted at
  the first applicable drawing opportunity without another protocol round trip?
- Are equal text rows reused, while changed rows and dynamic overlays remain
  correct? Record node reuse/recording counts separately from frame timing.
- Do full-transition pixels preserve owned content and correct clipping without
  stale-source clicks, composition resets, selection drift or invented history?
- Do reversals, cancellation before start, high/low/hidden cursor, API fallback,
  width/font changes and reconnect converge correctly with bounded resources?

No device was connected during the initial investigation. The later first build
adds focused emulator movement/content-reuse checks; this full-transition remote
timing matrix remains unperformed. The prior 39 native/semantic tests and Android
build/lint results remain review evidence, not a latency baseline.
