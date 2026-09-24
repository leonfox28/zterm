# Android frame preparation

## 1. Scope / trigger

Read with `android-app.md` and `../backend/shared-client.md` before changing
Android row conversion, display cadence or source lifetimes.
The native actor remains the sole ordered semantic installation/ACK owner;
Android selects complete already-installed frames for presentation.

## 2. Signatures and owners

- `NativeFrameSource.presentation_rows_from(previous: Option<Arc<NativeFrameSource>>)
  -> Vec<NativeRowUpdate>` returns one `Reuse { index: u32 }` or
  `Replace { row: NativeRow }` per row of this source's exported window.
- `presentation_rows() -> Vec<NativeRow>` remains the complete projection for
  independent consumers and comparison tests.
- `TerminalFrameProjection.resolve(next: NativeFrame): NativeFrame` is suspended
  on row conversion, owns one paired retained source/list baseline, and returns
  the candidate's metadata/source with resolved immutable row references.
- `collectTerminalFrames(current, wait, clock, deliver)` owns one observation's
  projection lifetime. `current` and `wait(afterGeneration)` each transfer a
  native source handle; `deliver` consumes a resolved handle synchronously on Main.
- `TerminalFrameClock.visible` and `awaitFrame()` own one cancellable
  Choreographer callback; the Repository updates visibility from the native View.

## 3. Contracts

A reuse index addresses the **exact list resolved from the supplied previous
source**, not a logical ordinal, wire revision or arbitrary drawn window. Kotlin
must retain that source as long as it keeps this baseline, then close it on
replacement, eager fallback, cancellation and observation termination. Candidate,
Repository and View handles remain independently owned. Every delivered frame
uses its current source/input/geometry authority even if all row objects match.
Treat generated row/cell records as immutable: replace/copy values rather than
mutating a shared record after projection.

Native reuse requires the same attachment origin and color/appearance context.
Borrowed semantic row keys use Hash plus full Eq over cells, exact text, wide
flags, every style field (including strike/conceal) and soft-wrap continuity.
Native attributes use bits 0/1/2 for bold/dim/italic and 3/4 for strike/conceal.
Cell equality/hash includes hyperlink identity/target so reuse cannot preserve an
obsolete action. Native page accounting includes shared hyperlink allocations,
deduplicated by allocation within each page; separate pages remain conservatively
accounted. Existing cache and pin limits stay unchanged.
Prefer an equal row at the same index, then match an equal row elsewhere in the bounded previous window.
Core Hash derives add no fields/wire semantics. A missing/incompatible baseline
returns only Replace records; colors/appearance conservatively replace the
whole window. Only replacements allocate/resolve/serialize cell DTOs. The
native page budget (including pins) and export bound of three viewports plus
one row remain unchanged; scratch lookup is bounded by that window.

Metadata-only frames reuse the complete Kotlin row list. For an observed active
content update with unchanged input epoch/readiness, geometry, screen, pointer,
selection, history offset, notice and error, the collector releases the unprepared
candidate, waits one display tick, then reads the native latest frame. It skips
intermediate **presentation candidates**, never required wire deltas. Initial,
metadata-only and observed authority transitions bypass the gate. A transition
arriving during the wait is included in that next re-read; native input fences
and ACKs already applied independently. Hiding releases a pending wait without
vsync. Cancellation removes the callback and releases pending/baseline handles.
The final native frame is readable after actor closure and must be delivered
before Closed ends observation. No timer, content queue, extra native observer
or per-frame network resize belongs here. Display pacing can add up to one tick
before preparation; do not call it a network-latency improvement.

The existing cell painter and row RenderNodes remain unchanged by preparation.
Do not infer a net renderer improvement from fewer text calls: compatible ASCII
runs may improve long text while sparse/decorated controls regress. Any future
batching experiment must preserve per-cell clipping, font shaping, wide/combined
text and individual fractional background coverage, then measure controls too.
Glyph ink bounds touching a cell edge can still leak antialias pixels under a
shared run clip; matching advance widths alone is insufficient.

## 4. Validation and error matrix

| Case | Required result |
| --- | --- |
| Equal row moves within/new height window | Reuse its old list index; deliver current coordinate source |
| Wide/combining/style/wrap changes | Replace complete semantic row |
| Origin or palette/appearance differs / no baseline | Full bounded replacement, no borrowed row authority |
| Metadata only | No row projection; newest metadata/source still delivered |
| Burst while awaiting display | Project latest already-installed complete native frame |
| End/lease loss during gate; View becomes hidden | Re-read final state and deliver; no wait for a future visible window |
| Cancel during projection | Close candidate and retained baseline; never deliver partial rows |

Closed/foreign source misuse is a local lifetime error, not a request to recover
wire state. Only generated native updates can supply reuse indices; no remote
payload or Kotlin ordinal conversion may construct this mapping.

## 5. Good / base / bad cases

Good: a 40-row TUI changes one row; one cell-bearing row crosses FFI and 39
Kotlin row objects survive. Base: first observation has no predecessor and
projects its bounded window. Bad: matching pixels keep an obsolete selection
source alive, or an unconsumed source waits for JVM collection after cancellation.

## 6. Tests and assertion points

`relative_projection_reuses_exact_rows_across_content_and_geometry_changes`
reconstructs source-relative updates against full projection, including moved,
sparse/dense/styled/wide/combining/wrapped/resize/palette/origin cases and retired
coordinate authority. `TerminalFramesTest` exercises the production projection
owner and real display clock: reference identity, fresh authority, Main/worker
separation, burst convergence, hidden final delivery and cancellation cleanup.
Its supported UniFFI no-handle fakes test foreign lifetime handling, not JNI.
`NativeFrameProjectionTest` requires `projectionFixture=1` and a ticket from a
new `presentation_fixture` host; it compares actual UniFFI reconstruction, sparse
reuse, resize growth and stale selection rejection on an owned Session.

Keep the existing keyboard/row-layer/pixel suite as a separate visual gate.
Performance comparisons must use matching warmup and control scenes; emulator
measurements never establish phone smoothness by themselves.

## 7. Wrong versus correct

Wrong: deserialize the entire new window, then call Kotlin equals to claim
allocation-free diff. Correct: send typed references for equal native rows and
serialize only replacements, preserving explicit source ownership.

Wrong: coalesce required wire deltas or delay ACK until Choreographer. Correct:
advance authoritative native state immediately and pace only Android preparation.

Wrong: adopt text batching from a long-ASCII benchmark alone. Correct: preserve
exact cell pixels and establish a reliable benefit with sparse/styled controls
before changing the existing painter.

Application-title changes are frame metadata only and must reuse prepared rows.
Cursor shape/blinking are dynamic overlay metadata; phase is View-local, never a
wire revision or a row-cache key. Conceal suppresses glyphs and decorations after
background painting; strike uses the existing text metrics. Selection and cursor
overlays never repaint concealed glyphs.
