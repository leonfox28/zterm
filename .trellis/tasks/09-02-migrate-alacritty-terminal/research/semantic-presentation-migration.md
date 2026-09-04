# Semantic Presentation Migration Research

> Historical compatibility analysis: the user later chose a coordinated direct cutover and will
> upgrade every node. `semantic-presentation-direct-cutover.md` is authoritative wherever this file
> discusses legacy ANSI retention, presentation-family negotiation, capabilities, or old/new tests.

## Question and root-cause classification

The reported fixture is that a nested full-screen TUI keeps a themed cell in
its rightmost column in Zterm's authoritative Alacritty model and in a full
snapshot, but the cell disappears after the first incremental presentation in
Ghostty. Herdr is useful evidence, but it is not the scope of the solution.

The causal chain contains two defects at different levels:

1. **Local implementation defect in the legacy ANSI encoder**: a changed row
   always ends in `EL0`. ECMA-48 erase-in-line mode zero includes the current
   cell, and a terminal may leave the cursor on the final cell in pending-wrap
   state. A full-width replacement can therefore erase the cell it just drew.
   The existing row-replacement contract is sufficient once it clears only a
   non-empty suffix from an explicit column.
2. **Architecture / boundary defect in active presentation**: daemon-authored
   terminal ANSI, attachment-local status, gutter cleanup, scrollbar, capture
   modes, and cursor restoration are calculated by separate owners and joined
   only as ordered byte fragments. No component owns one complete desired
   physical frame or advances one physical baseline after successful output.
   The earlier gutter, status, return-live, and right-margin failures are
   sibling manifestations of this missing invariant.

This classification follows the project-wide
`root-cause-and-architecture-thinking-guide.md`. Product code must not be
changed if new evidence invalidates either causal claim; the task must return
to planning instead of compensating with repaint, timing, process-name, glyph,
or theme heuristics.

## Current boundary map

### Authoritative model

- `zterm-terminal` privately owns Alacritty `Term`, its grid/history, and
  `ProjectedScreen` / `ProjectedRow` / `ProjectedCell`.
- The projection already contains the required semantic information: exact
  viewport size, active screen, full-width rows, wrap flags, bounded cell text,
  wide-head/continuation flags, style, cursor, and input modes.
- `TerminalState` exposes most visible semantics but flattens rows and drops
  wrap flags, so it is not an exact transport surface.
- `TerminalCheckpoint` already retains a private projected screen. Resume
  checkpoints are therefore presentation-encoding neutral even though the
  current `sync_latest` result is ANSI.

### Wire and attachment

- Wire kinds 301/302 carry ANSI snapshot/delta; 317/318 carry bounded ANSI
  history windows. Capability bits are allocated through bit 20.
- Local IPC has no hello/welcome negotiation. Its first frame is a
  `TerminalAttachRequest`, so end-client presentation acceptance must be
  carried there. Relying only on the remote daemon's connection capability
  would allow a new bridge to send semantic frames to an old local CLI.
- Protobuf's unknown-field behavior supplies the needed compatibility shape:
  an old daemon ignores a new attach preference and returns legacy ANSI; an old
  client omits the preference and a new daemon must return legacy ANSI.
- A remote bridge already forwards an attach request after connection
  capability negotiation. It can request semantic presentation only when both
  the local end client requested it and the remote welcome advertises it.
- A resumed attachment may initially receive a delta. If reconnect negotiation
  changes presentation encoding, the bridge must discard the known revision
  for that epoch and require a full snapshot before switching adapters.

### Current CLI presentation

- `ChromeLayout` assigns a main-screen child rectangle plus optional gutter,
  and gives the full content width to the alternate screen. A remote status row
  is separately reserved.
- `ViewportController` keeps a generic cache instantiated with ANSI rows.
- `TerminalRenderer` validates and writes daemon ANSI, while `StatusRenderer`
  and `write_scrollbar` append more ANSI. `present_atomic` only wraps the
  resulting fragments in one DEC 2026 transaction and commits logical
  revision after write/flush.
- DEC 2026 hides intermediate painting but cannot correct the final ordering or
  reconcile several independent baselines.
- The terminal lifecycle guard is a legitimate outer owner before and after an
  active attachment. During the active UI, however, all display and host-mode
  changes need one presenter.

## Target invariants

1. The child PTY byte stream ends at one daemon-owned terminal model.
2. The daemon transports bounded Zterm semantic state, not Alacritty types and
   not ANSI, on a negotiated new/new attachment.
3. One client-side `AttachmentSurface` retains the complete validated live
   semantic state at one revision.
4. Live terminal rows or cached history rows, status, gutter, cursor, and host
   mode policy are laid out and composed before backend encoding.
5. One `DesktopPresenter` is the only active physical writer in semantic mode.
   It transitions from the last successfully flushed `ComposedFrame` to the
   next complete desired frame.
6. An output failure makes the physical baseline unknown. The next attempt is
   a complete resynchronization; no speculative baseline is committed.
7. A reported application is a regression/smoke fixture only. No process name,
   title, glyph, theme, timing delay, or repaint loop participates in
   correctness.
8. Legacy ANSI remains an explicit mixed-version adapter. Its row replacement
   is fixed independently, but success on that adapter alone cannot complete
   this migration.

## Semantic domain shape

The renderer-neutral types belong in `zterm-core::terminal`. They reuse the
existing `TerminalCell`, `TerminalStyle`, `TerminalColor`, `TerminalCursor`,
`TerminalModes`, `TerminalSize`, and `ActiveScreen` values.

Conceptual additions:

```rust
pub struct TerminalSurfaceRow {
    pub cells: Vec<TerminalCell>, // exactly surface.size.columns
    pub wrapped: bool,
}

pub struct TerminalSurface {
    pub size: TerminalSize,
    pub active_screen: ActiveScreen,
    pub rows: Vec<TerminalSurfaceRow>, // exactly surface.size.rows
    pub cursor: TerminalCursor,
    pub modes: TerminalModes,
}

pub struct TerminalSurfaceSnapshot {
    pub revision: Revision,
    pub surface: TerminalSurface,
    pub scroll_metrics: Option<TerminalScrollMetrics>,
}

pub struct TerminalSurfaceRowPatch {
    pub row_index: u16,
    pub row: TerminalSurfaceRow,
}

pub struct TerminalSurfaceDelta {
    pub from_revision: Revision,
    pub to_revision: Revision,
    pub size: TerminalSize,
    pub active_screen: ActiveScreen,
    pub cursor: TerminalCursor,
    pub modes: TerminalModes,
    pub rows: Vec<TerminalSurfaceRowPatch>,
    pub scroll_metrics: Option<TerminalScrollMetrics>,
}
```

A semantic delta is valid only against the exact revision, size, and active
screen baseline. Size/screen/checkpoint-format mismatch produces a semantic
snapshot. Changed rows are full-width replacements with unique increasing
indices; a revision may advance with zero row patches while cursor, modes, or
metrics still change. This keeps the patch complete and deterministic without
inventing a terminal-command mini-language.

Semantic history-window results reuse the existing anchor, target, disposition,
and coordinate contract, replacing `Vec<Vec<u8>>` with
`Vec<TerminalSurfaceRow>`. The generic `ViewportCache<Row>` remains the shared
client-owned cache reducer.

### Validation and bounds

- Rows and columns must be non-zero and within existing product limits.
- Snapshot row count and every row's cell count must exactly match its size.
- Delta row indices must be strictly increasing, unique, and in range; delta
  revision must advance and the supplied baseline must match exactly.
- Cell content remains bounded by the existing 22-byte model cap, valid UTF-8,
  and free of C0/C1/ESC controls before any desktop backend can emit it.
- A wide head must have a following continuation in the same row; a
  continuation must immediately follow a wide head, carry no independent
  content, and no wide head may start in the final column.
- Cursor coordinates must be in range. Main scroll metrics must be structurally
  valid and match the surface revision/height; alternate surfaces have none.
- All semantic content messages retain the existing 8 MiB frame limit and
  content-redacted `Debug`. Maximum-dimension encoded-size fixtures prove the
  straightforward full-cell representation fits; no speculative packed-memory
  or aggregate-session admission rule is reintroduced.

Width flags are authoritative terminal-model semantics. The desktop presenter
uses explicit absolute positioning around changed runs, so a host emulator's
Unicode-width disagreement cannot become its cursor source of truth.

## Additive wire design

Allocate capability bit 21 as `TERMINAL_SEMANTIC_PRESENTATION`. The capability
means the endpoint implements the complete bundle: semantic live
snapshot/delta plus semantic history-window response. It is advertised only
alongside the existing terminal service and history-window capability.

Add a `TerminalPresentationEncoding` field to `TerminalAttachRequest`:

- unspecified/legacy requests the existing ANSI family;
- `SEMANTIC_CELLS_V1` requests semantic cells but always permits a legacy
  response for old-server compatibility.

Allocate three new content kinds without changing wire major or existing
numbers:

- 319: semantic snapshot;
- 320: semantic delta;
- 321: semantic history-window frame.

The existing history-window request kind 317 is representation-neutral and is
reused. Kinds 315/316 and 312/313 remain unchanged for a wholly legacy
attachment. A semantic attachment never accepts ANSI snapshot/delta/history
content mid-epoch, and a legacy attachment never accepts semantic content.

Negotiation matrix:

| End client | Local/serving daemon | Remote daemon | Result |
| --- | --- | --- | --- |
| new | new | local or bit-21 remote | semantic family |
| new | old | any | legacy; old daemon ignores preference |
| old | new | any | legacy; preference is absent |
| new | new | remote without bit 21 | bridge requests legacy |

Representation is frozen for one attachment transport epoch. A reconnect may
renegotiate; an encoding change forces a full snapshot, clears the incompatible
row cache, and invalidates the physical presenter baseline. It may not be
smuggled through a cross-encoding resume delta.

## Model, session, and compatibility ownership

- `ProjectedScreen` remains private and is converted directly to semantic
  domain values. No Alacritty type enters core/proto/daemon UI APIs.
- Attachment synchronization selects one presentation encoding before asking
  the model for an update. The checkpoint remains semantic and can support
  either encoder, but the semantic path must not call `encode_full`,
  `encode_delta`, or `recent_history_ansi`.
- Session attachment state records the selected encoding so every initial,
  update, final-drain, sync-required, and history response uses one family.
- The semantic snapshot omits `recent_history_ansi`. The client-owned history
  window is the scroll truth; opportunistic prefetch uses the existing bounded
  window request rather than replaying text into an outer terminal's physical
  scrollback.
- The existing ANSI DTOs and encoder become a named legacy compatibility
  adapter. It receives the extent-aware suffix correction: write row content,
  and only when visual extent is less than width, `CUP` to `extent + 1` and
  erase the non-empty suffix. It never erases after a full-width row.

## Attachment surface, viewport, and composition

`AttachmentSurface` owns the full validated live surface and its revision. It
installs snapshots and applies only contiguous full-row patches. A gap requests
a full sync without mutating the currently presented frame.

Semantic mode instantiates `ViewportCache<TerminalSurfaceRow>`; legacy mode
keeps the existing `ViewportCache<Vec<u8>>` behind the compatibility adapter.
While pinned in history, semantic live updates continue updating
`AttachmentSurface`, but the visible source remains the cached main-screen
slice with a hidden cursor. Returning live composes the latest complete surface
once. Existing same-epoch anchoring, latest-wins fetch, 16 ms desktop cadence,
33 ms drag request pacing, and one-report/one-line input ownership remain.

`ChromeLayout` first allocates non-overlapping regions:

- Main: bounded child rectangle plus optional one-column host gutter;
- Alternate: full bounded content width, no gutter;
- Remote: a separate final physical status row when height permits.

The compositor consumes a full visible row source, current chrome state, and
host-mode policy and produces one renderer-neutral `ComposedFrame`. A sparse
absolute-row representation avoids allocating `physical_rows *
physical_columns` when a TTY reports very large `u16` dimensions. Its owned
cell count is explicitly bounded by the product content rectangle plus one
status row. Region overlap or an out-of-bounds write is an internal error, not
last-writer ordering.

The compositor always starts from a complete retained semantic source. A
delta changes the source, not the output algorithm. Therefore Main/Alternate
gutter transfer, status movement, history/live transition, and chrome-only
updates naturally compare two complete frames; no post-child cleanup can
erase a newly owned child column.

## Sole desktop presenter

The semantic desktop presenter owns, during an active attachment:

- every terminal/status/gutter cell written to stdout;
- exact cursor position, visibility, and rendition restoration;
- physical keyboard/paste/focus modes required to observe child input;
- unconditional host SGR mouse capture (`1003` + `1006`);
- DEC 2026 begin/end batching and the committed physical baseline.

Child mouse modes and alternate-scroll remain semantic input-routing state;
they are not copied as competing outer mouse modes. The lifecycle guard alone
owns outer alternate-screen entry/raw-mode setup before presentation and the
unconditional cleanup after it.

For an incremental transition, the presenter expands changed coordinates to
cover old and new wide-cell spans, groups adjacent equal-style cells, positions
each run with absolute `CUP`, writes literal blanks for removals, and restores
the final cursor with another absolute `CUP`. It does not use `EL0`/`EL2` as an
incremental row-tail shortcut. Reaching the physical rightmost cell is safe
because no later operation derives location from pending-wrap state.

A full resync resets rendition, clears the outer alternate screen, and paints
the complete desired frame. Resize, active-screen/layout change, representation
change, missing baseline, or previous I/O failure triggers this path. One
transition is built fully in memory, wrapped in DEC 2026, then issued through
exactly one `write_all` and one `flush`. The baseline advances only after both
succeed. A write/flush failure retains no speculative baseline, makes the next
attempt full, and best-effort closes DEC 2026 without hiding the original I/O
error.

Connection status and reconnect indication are compositor inputs. They may
change the reserved status row while retaining the last complete terminal
surface; they cannot append a standalone newline or mutate the screen outside
the presenter.

## Code placement

No new external renderer, Ratatui dependency, terminal parser, or shared
presentation crate is justified for this scope.

- `zterm-core`: semantic DTOs, invariants, validation, generic cache types.
- `zterm-proto`: semantic protobuf conversion/redaction and new wire kinds.
- `zterm-terminal`: private projection, semantic snapshot/patch/history
  production, and explicitly named legacy ANSI adapter.
- `zterm-daemon`: per-attachment encoding selection, session lifecycle, local
  and remote forwarding/negotiation.
- `zterm-cli`: split the current large terminal UI into focused private
  surface, composition, and ANSI presenter modules; keep orchestration/input in
  `terminal_ui.rs` and isolate the legacy adapter.

This leaves Android with core/proto semantic surfaces and cache coordinates,
without linking Alacritty or parsing desktop ANSI. Android's pixel renderer,
font shaping, IME, touch/fling physics, and native vsync remain the next task.

## Verification and release boundary

The migration is internally staged but has one completion and release gate.
There is no extent-only interim release.

- Domain/proto: valid round trips; malformed shape, controls, wide-cell,
  cursor, revision, frame-size, and redacted-debug cases; fixed old numbers.
- Model: snapshot/patch equivalence across chunking, Unicode, styles, wide
  cells, wrap, screen/resize, empty-visible updates, history coordinates, and a
  proof that semantic mode constructs no ANSI.
- Session/transport: initial snapshot and resume delta; ack, gap/resync,
  reconnect, takeover, final drain, local/direct/relay forwarding, and the full
  old/new negotiation matrix.
- Client/compositor: non-overlapping layouts, complete live/history sources,
  status/gutter ownership transitions, screen/resize transitions, rightmost
  and wide-cell changes, styled blanks, cursor/modes, chrome-only updates, and
  no independent active writer.
- Presenter: exact desired-frame replay in a strict terminal oracle, one DEC
  2026 transaction, one write/flush, failure then full retry, and no erase-line
  incremental shortcut.
- Compatibility: generic full-width/short-row legacy fixtures and mixed peers.
- Real terminals: macOS and Linux local/direct/relay smoke; a generic nested
  TUI fixture is normative, while Herdr and any available PiAgent flow are
  external smoke examples.

The task is complete only when all new/new semantic paths, the legacy
compatibility invariant, project checks, spec synchronization, and separate
macOS/Linux evidence pass. Performance/RSS benchmarks remain deliberately
excluded, and product Rust remains `unsafe_code = "forbid"`.
