# Terminal presentation continuity design

Status: implemented; local quality checks complete, desktop GUI visual evidence unavailable. Baseline `be66a16`. Product choices are in [prd.md](prd.md).
This design retains the current semantic wire format and changes publication and presentation
ownership inside the existing host, desktop CLI and Android client.

## 1. Separate state, eligibility and presentation

Three boundaries must remain distinct:

| Boundary | Owner | Meaning |
| --- | --- | --- |
| Working terminal state | `zterm-terminal` | Ordered PTY bytes have been parsed and their terminal/control effects processed. This can include an unfinished synchronized-output batch. |
| Eligible terminal view | `zterm-terminal`, published by `terminal_driver` | One coherent semantic state may be read by attachments. Its revision identifies its exact cells, colors, cursor, modes, size, screen and history metadata. |
| Applied / displayed client state | Shared client and each platform presenter | Protocol updates have been validated and installed; a platform separately commits the source and geometry it actually submitted for display. |

Local geometry can change without a remote revision. A working revision is not proof of an
application frame boundary, and platform submission is not proof of physical scanout. Visual
evidence is required in addition to state tests.

The normal flow remains:

```text
PTY bytes -> bounded ingress -> working model -> eligible semantic view
    -> existing snapshot/delta -> ordered client installation
    -> latest display candidate + local geometry -> platform presentation commit
```

No second terminal parser, per-revision frame queue or guessed application identity is introduced.

## 2. Host synchronized-output ownership

### Parsing and eligible state

Handle `CSI ? 2026 h`, `CSI ? 2026 l` and `CSI ? 2026 $ p` at the existing streaming ingress
dispatch boundary. Answer the mode query with set/reset status. Preserve split-sequence, supported
C1 and combined-private-mode parsing. Do not search raw chunks for marker substrings or forward
2026 into upstream VTE's raw synchronized-output buffer. Incremental resource enforcement, ordered
PTY replies, color queries and other required control effects continue while presentation is held.

Use one model-owned publication state, conceptually:

- `Live`: the current model is eligible and can be projected under the model lock.
- `Held { frozen_view, deadline }`: working state continues changing, while visible reads use one
  frozen eligible view captured before the first begin marker's subsequent mutations.

The frozen view includes its own scroll metrics and color snapshot; readers must not combine old
rows with metadata from the working model. Reuse immutable projection/checkpoint storage where
practical. The design does not require a full copy on every byte or unmarked ingest.

| Event | Publication behavior |
| --- | --- |
| First begin in `Live` | Commit metadata at this boundary, freeze the eligible view, enter `Held`, arm the hard deadline. |
| Repeated begin in `Held` | Remain held; do not create nested depth, recopy the view or extend the hard deadline. |
| End in `Held` | Commit working metadata, make the current state eligible and service eligible-boundary reads. |
| End in `Live` | Idempotent mode reset; no invented batch or wait. |
| Unmarked output | Publish at existing processing/presentation opportunities without a new guessing delay. |
| Timeout, terminal reset, resize or EOF | End an unfinished hold through the serialized model owner, publish a valid current state and preserve lifecycle progress. These are explicit exceptions to normal batch completeness. |

For `BSU A ... ESU A BSU B ...` within one PTY ingest, the eligible view after the call must be A,
with B still private. This requires a boundary hook or segmented model advance at the actual
parser event. Notification after the entire ingest, followed by capture of the working model, is
insufficient. A boundary cannot be deferred past bytes from the next batch.

### Revision and metadata contract

Retain a single monotonic `Revision` allocation source, but distinguish working progress from the
published watermark. The current exactly-one-revision-per-nonempty-ingest assumption must be
relaxed for internal publication boundaries. Normal unmarked ingest can retain its current cadence.
One revision must never identify two different published states: further mutations after a captured
boundary receive a later token, even when they occur in the same ingest.

Commit colors, screen identity and history epoch/retained-row metadata at each boundary before
projection. Do not leave these commits until the original ingest's end. `sync_changed` compares its
checkpoint with the **published** watermark; comparing with a later held working revision would
repeatedly offer the same old view. Revision watches and waiters observe publication, while drain
accounting continues to record processed bytes/chunks independently.

Wire snapshots/deltas already carry exact revisions and do not require consecutive numbers. Keep
their existing format, validation and checkpoint behavior. No wire-level begin/end messages are
needed: the host supplies eligible semantic updates to both clients.

### Deadline and serialized recovery

Use an initial 150 ms hard hold limit, as one host-owned engineering constant, independent of
unmarked-output scheduling. Normally closed batches publish immediately at end; this limit is not
an added frame delay. The value can be tuned with evidence without inventing a second timer layer.

Replace indefinite-only model-queue waiting with deadline-aware wakeup. Check expiration between
bounded processing units as well as on an empty queue, so continuous output or repeated begin
markers cannot starve recovery. A clock/deadline test must prove that begin followed by no more
bytes releases the hold. Scheduling and lock acquisition may add normal runtime overhead; do not
describe this as a real-time scheduling guarantee.

Preserve the existing serialized PTY/model resize commit order. Resize ends a hold and publishes
the correct new-size model; it does not wait for child SIGWINCH handling. Reset/EOF/failure and
attachment lifecycle signals must also progress without waiting for a future end marker.

### Consistent snapshots, attachments and history

`sync_latest`, `sync_changed`, explicit `latest_snapshot` and initial attachment snapshots all read
the same eligible view. A new attachment can promptly receive the previously eligible view while
a batch is held. Required control replies do not wait for any of these reads.

History is different: a requested window can include mutable live rows, and working output can
evict old history. A frozen visible screen cannot reconstruct arbitrary old history. Use a bounded
pending read during `Held` rather than copying the entire scrollback or fabricating a history gap:

1. Validate identity, query and deadline through the existing Session admission path. If `Live`,
   project immediately as today. Otherwise register one bounded pending read for the eligible
   controller attachment and return control to the Session actor.
2. At the next eligible boundary, project the request against that exact model state **before**
   processing another batch. Store at most the one capped response until delivery. Real epoch
   changes/eviction use the existing `HistoryChanged` / `HistoryGap` semantics.
3. Keep typed query/reply and cancellation ownership in the driver/Session layer; the terminal model
   supplies synchronous boundary/projection behavior, not runtime futures or transport handles.
4. Waiting must hold no Session actor, commit/model mutex or attachment stream reader. Deliver the
   correlated result through the existing outbound writer, rechecking attachment/control lifetime.
   Cancel on request deadline, disconnect, takeover or end. Enforce the bounded request slot using
   normal request admission/error handling; do not silently replace an admitted request.

This intentionally extends today's stateless history read path only for an in-progress marked
batch. Simply waking a reader after ESU is insufficient: it might run after B has already begun.
The synchronous model boundary must capture the pending result, and async delivery must not block
PTY processing, input admission or required snapshot ACKs.

## 3. Resize, application output and protocol progress

A resize has independent milestones:

```text
local clip/pan -> settled desired size -> PTY/model resize -> valid new-size snapshot
                                                  -> child output and optional BSU/ESU batches
```

Neither snapshot dimensions nor the first ESU after resize proves that the application completed
its new layout. Do not drop ordered bytes, treat blank output as an unfinished frame, or wait
indefinitely for a TUI repaint. Later application changes may affect any region.

Preserve the existing single in-flight resize/latest-desired-size coalescing. For rapid A-B-A,
required protocol updates still apply, but obsolete responses must not restart a visual animation
toward an old intent. Geometry generations are local identities; no application completion identity
is inferred from a remote revision or from equal dimensions.

The Session withholds later updates while awaiting snapshot ACK. Therefore:

- Desktop retains ACK after successful presentation of the installed coherent candidate. It must
  present that valid candidate promptly rather than hold its ACK for a future application repaint.
- Android retains native installation/ACK independently of Kotlin drawing. Track actual displayed
  source ownership separately so an ACK does not prematurely retire what is still on screen.
- Ordinary deltas retain ordinary revision-applied handling. Snapshots and resume deltas retain
  their origin-specific exact ACK correlation and are acknowledged only once.

Coalesce display candidates only after validating/applying required delta dependencies. This
changes neither reconnect recovery nor the protocol's barrier semantics.

## 4. Shared height-transition rules

For equal width/font and a live visible cursor, shrinking from the old grid to `target_rows` uses
the pinned host engine's rule:

```text
shift_rows = max(0, cursor_row + 1 - target_rows)
```

Clip at the bottom when the cursor already fits; otherwise move the old content up by exactly
`shift_rows`. Compare the resulting displayed positions with the authoritative candidate. This is
a temporary presentation transform; it does not permanently resize or discard the client's source.

Use the committed visible source when establishing a transition, not a hidden transient cursor
from an unfinished batch. History browsing/selection retain their navigation ownership and do not
suddenly chase the live cursor. Missing/hidden cursors have no invented caret anchor.

Growing may pull available retained history according to the host rule; newly exposed rows that
the client does not possess remain neutral until authoritative content arrives. A history maximum
is not the history itself. Width/font changes permit real reflow and need correct fallback rather
than a claim that old positions are preserved.

Share a small pure row-transform helper only if both clients use it. Pixel rectangles, outer
emulator behavior, ANSI emission, Canvas and IME remain platform-owned.

## 5. Android: one grid geometry and one drawn source

At stable endpoints, measure physical pixels once after fixed chrome and consumed system insets:

```text
rows = floor(available_terminal_height / cell_height)
grid_height = rows * cell_height
remainder = available_terminal_height - grid_height

header
terminal grid (exact grid_height)
fixed-height toolbar buttons
app-owned remainder padding
system navigation safe area, or docked IME
```

The toolbar remains directly adjacent to the terminal. Put the remainder **below the toolbar**,
with its background, rather than above it. Preserve system-controlled navigation space; do not
resize the gesture area or add overlapping IME and navigation insets twice. Keep existing product
row caps and tiny-window behavior explicit: overflow beyond a cap is not a sub-row remainder.
When the existing 80-row cap binds, retain the capped viewport and neutral excess area with the
toolbar anchored as today; exclude excess from active-grid hit testing and move only the sub-row
remainder below the toolbar. Do not convert all excess into a large toolbar bottom pad. A window
smaller than one cell retains the existing one-row clipped minimum. These constrained cases are
outside the normal exact-fit/adjacency guarantee and do not raise the product viewport limits.

Use actual Paint-derived cell metrics and one geometry value for row calculation, content rectangle,
clip/pan, final handoff, hit testing, selection and IME anchor. Either a measured exact grid View or
an exact internal grid rectangle is valid, provided the external remainder is laid out below the
toolbar. Keep measurement one-way; avoid a feedback loop where adding padding changes the height
used to derive that same padding. Convert to dp only at the outer layout boundary where needed.

For the accepted fixture (17 px cells, 1000 -> 600 px usable height), stable grids are 986 -> 595 px,
and external padding is 14 -> 5 px. Cursor row 39 moves up five rows: both the transformed old and
new row 34 are at y=578 px. There is no extra 9 px handoff correction.

Retain IME animation fencing and final pre-draw measurement. Deliver native lifetime edges
from the Activity owner through direct observers: Compose can coalesce prepare/end before
recomposition. Register/dispose the observer with the TerminalView composition lifetime, and
read live animation state during layout measurement. Intermediate frames may follow
smooth pixel animation; only settled endpoints require integral rows. Do not snap each animation
step or send each intermediate size remotely. Earlier known-target resize is deferred.

Maintain at most bounded pending/drawn source handles. Commit the submitted geometry and source
together in `onDraw`; IME anchors use that geometry and the actual screen origin. Release retired
sources explicitly. Input readiness/preedit metadata must still propagate even if body rows can
be reused. A pending native frame must not silently become the hit-test or IME source before draw.

Keep normal complete Canvas draws initially. Skipping unchanged Canvas cells is unsafe without
retained rendering storage; a complete draw submitted coherently is not by itself a blank frame.
Row/RenderNode caching is deferred unless measurements show it is needed.

## 6. Healthy resize input continuity

The host already permits the same previously active controller to write during visual sync. Use
that existing admission rule, with separate local concepts:

| State | Keyboard/composition | Coordinates |
| --- | --- | --- |
| Healthy resize of the same live attachment | Keep valid input epoch/InputConnection and preedit. Admit ordinary text/keys through the existing ordered path and current accepted input modes. | Advance local geometry generation; fence stale pointer/selection/copy sources. |
| Initial attach, recovery, takeover, disconnected or ended | Preserve the existing barrier/lease rules; retire stale input epochs and callbacks as appropriate. | Retire affected geometry/source identities. |
| Frozen history / return to live | Preserve existing navigation and bounded resume-input behavior; do not broaden the healthy live-resize exception to this path. | Use the valid displayed navigation source. |

Represent why input is gated explicitly. A generic `Synchronizing` state is not enough to identify
a healthy resize. The exception starts only from a previously active live attachment and ends on
real connection/control/session changes. No new offline queue, predictive echo or replay is added.
Input modes remain asynchronous as today; mode-aware encoding uses the latest accepted mode
snapshot and continues to respect platform presentation ownership.

On Android, resize no longer causes an unconditional epoch change, composition clear or
`restartInput`. Split geometry generation from the connection/control epoch in native source and
FFI state. This also rejects stale A-B-A coordinate callbacks with equal dimensions. Avoid hiding
the cursor, disabling ordinary keyboard shortcuts or showing a connection spinner solely for this
healthy geometry transition. Preserve accurate UI for real recovery and control loss.

On desktop, do not flush pending stdin or restart its reader solely for healthy geometry change.
Keep output-dependent outer input modes committed after successful presentation, and separately
invalidate coordinate-based interaction. Retain all genuine reconnect/lease epoch protections.

At every existing queue/admission boundary, an accepted text/key unit is sent once or gets an
explicit failure through the normal error path. Audit silent early returns, especially Android's
`inputReady` gate. Rejection must not be reported as accepted, and failure must not trigger automatic
replay into a later connection.

## 7. Desktop: valid diff versus complete coverage

Retain final resolved-cell comparison for text, styles/colors, wide-cell pairs, cursor, selection
and chrome. When the physical baseline is valid, emit only changed runs. An executed Shell line is
not assumed immutable.

When size/layout or outer-emulator reflow makes the physical baseline unknown, cover every owned
target cell that may be stale, including blanks, wide-cell remnants and retired chrome. Replace
the current clear-then-diff fallback with explicit coverage inside the existing outer 2026 wrapper
where feasible. Simply deleting ED2 while treating unknown cells as empty would leave old text.
Clear residual owned areas explicitly; never wipe unrelated outer-emulator history to compensate.

Use a transformed old baseline only where its physical mapping is verified. Otherwise perform
correct coverage and accept the outer emulator's limitations. Unsupported outer 2026 still has to
converge correctly, but atomic scanout cannot be promised.

Commit baseline and input-mode state only after successful write/flush. Partial writes invalidate
the assumed baseline and require existing error cleanup, including best-effort closure of an outer
synchronized-output batch. Retain the current viewport pacer, not a new global PTY debounce.

## 8. Bounded work, compatibility and verification

Memory remains one working model, at most one frozen visible view, existing attachment checkpoints,
one capped pending history result, and existing bounded client source/cache storage. Repeated begin
markers must not create copies or queued frames. Runtime handles stay out of the terminal model.

Same-size wire deltas and cross-size snapshots remain compatible with existing clients. Host
upgrade enables application 2026 support; new client presentation/geometry behavior does not
depend on a wire version change. Local Android FFI can change, with generated bindings regenerated
through the existing build. Internal model revision, driver history-read and client input-epoch
contracts require corresponding spec updates when their code changes land.

Start with a synthetic baseline recording and correlate geometry, resize submission, model/public
revision, batch boundaries, applied revision, draw submission and input readiness. Do not record
user terminal text. Separate blank frames, unwanted displacement, input interruption and frame
budget misses. Use owner tests for semantic/ACK/deadline invariants and desktop/device visual
checks for continuity; builds and arithmetic fixtures alone are insufficient.

Projection reuse is a measured follow-up inside this task only if needed to meet accepted behavior:
avoid rebuilding identical Android rows on metadata-only events, without delaying lifecycle or
input changes. Broader render caches and early resize remain deferred.

Rollback can isolate host publication, desktop coverage and Android geometry/input changes. Keep
each ownership change coherent with its tests/spec. Do not revert only the ingress 2026 rejection
after advertising support, or only blank coverage after removing ED2. No deployment or release
process change is part of this task.

Reference evidence: [initial research](research/cross-client-presentation.md) and
[second review](research/second-review.md). Primary references include
[synchronized output](https://github.com/contour-terminal/vt-extensions/blob/master/synchronized-output.md),
[Android keyboard animation](https://developer.android.com/develop/ui/views/layout/sw-keyboard),
[Compose inset consumption](https://developer.android.com/develop/ui/compose/system/insets-ui) and
[Android retained rendering behavior](https://developer.android.com/develop/ui/views/graphics/hardware-accel).
