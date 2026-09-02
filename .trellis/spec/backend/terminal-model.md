# Host-Authoritative Terminal Model Contract

## 1. Scope / Trigger

Apply this contract when changing terminal DTOs in `zterm-core`, the
Alacritty-backed model in `zterm-terminal`, terminal query replies, side
events, snapshots/checkpoints/deltas, history, or PTY-output safety bounds.

`zterm-terminal` owns the only terminal parser/grid/state engine. The daemon
owns one model per Session; controllers consume Zterm-owned values and
allowlisted ANSI and never own an upstream terminal type.

## 2. Ownership and Signatures

`zterm-core::terminal` owns only transport-neutral values:

- size, screen, cell/style/cursor, modes, events, and updates;
- snapshot, delta/resync, and history values;
- screen-selection metadata and the final snapshot byte limiter.

`zterm-terminal` owns the host-only engine boundary:

```rust
TerminalModel::new(size: TerminalSize, scrollback_rows: usize)
    -> Result<TerminalModel, TerminalError>
TerminalModel::ingest(&mut self, bytes: &[u8])
    -> Result<TerminalUpdate, TerminalError>
TerminalModel::preflight_resize(&self, size: TerminalSize)
    -> Result<Revision, TerminalError>
TerminalModel::resize(&mut self, size: TerminalSize)
    -> Result<TerminalUpdate, TerminalError>
TerminalModel::checkpoint(&self) -> TerminalCheckpoint
TerminalModel::snapshot(&self) -> TerminalSnapshot
TerminalModel::delta_or_resync(&self, checkpoint: &TerminalCheckpoint)
    -> TerminalDeltaResult
TerminalModel::state(&self) -> TerminalState
TerminalModel::history_page(direction, cursor, maximum_rows)
    -> Result<TerminalHistoryResult, TerminalError>
TerminalSnapshot::limit_ansi_payload(&mut self, maximum_bytes: usize) -> bool
```

`TerminalCheckpoint` is opaque and content-redacted. No public signature or
Debug implementation exposes an `alacritty_terminal` or `vte` type.

## 3. Dependency and Engine Boundary

- The workspace pins the official crates.io
  `alacritty_terminal = "=0.26.0"` with default features disabled.
- `zterm-terminal` is the only direct engine owner. `zterm-core` and
  `zterm-proto` have no `zterm-terminal`, Alacritty, or `vte` dependency. The
  CLI has no direct engine dependency; it includes the engine only transitively
  because the host binary also contains the daemon.
- `Term<BoundedEventSink>` and the re-exported `vte::ansi::Processor` are the
  sole terminal state engine. The Zterm ingress policy frames controls and
  applies product policy but never stores a grid or history.
- Engine configuration uses the requested bounded scrollback, disables OSC 52
  and Kitty keyboard mode, and normalizes initial alternate-scroll to off.
- Zterm never calls Alacritty tty, event-loop, renderer, process-spawn, or
  selection APIs. `portable-pty` remains the sole PTY/process owner.
- All Zterm-owned crates inherit `unsafe_code = "forbid"`. No wrapper, FFI,
  raw pointer, custom `Send`/`Sync`, dual parser, or runtime fallback is allowed.

## 4. Ordered Ingest and Revisions

- Non-empty PTY chunks are ingested in order and advance exactly one checked
  `Revision`, including chunks ending in a partial control. Empty input is a
  no-op. A successful same-size or changed-size resize also advances exactly
  one revision.
- Revision and allocation preflight complete before terminal mutation. Reply
  overflow is terminal-fatal to the driver; it must not continue with an
  unknown child-reply stream.
- Whole-input, one-byte, fixed-size, and deterministic-random chunking must
  produce identical semantic state, replies, and allowed side events.
- Query replies are exactly primary DA `CSI ?1;2c`, DSR status `CSI 0n`,
  standard CPR, and private `CSI ?row;columnR`. Secondary DA, window/color/mode
  queries, and other private markers receive no reply.

## 5. Ingress and Side-Effect Policy

`TerminalIngressPolicy` is a streaming, chunk-invariant trust boundary with
these hard caps:

| Input-controlled value | Cap |
| --- | ---: |
| ESC/CSI bytes | 256 |
| OSC/DCS/APC/PM/SOS bytes | 1,024 |
| canonical reply bytes per update | 64 KiB |
| side events per update | 32 |
| title/icon source bytes | 256 |

- The policy recognizes 7-bit and C1 introducers, BEL/C1-ST/split-ESC-ST
  termination, and CAN/SUB cancellation. Overflow discards through the current
  terminator and emits one content-free bounded classification; payload bytes
  never become printable fallback text.
- Inside a partial ESC/CSI sequence, ESC is an ECMA-48 anywhere transition that
  discards the old syntax and restarts framing, including across input chunks.
  C1 introducers likewise restart in their policy-owned state. Embedded C0/DEL
  bytes execute or are ignored without joining the buffered syntax, so they
  cannot obscure a filtered sequence such as synchronized-update 2026.
- BEL maps to audible bell, `ESC g` to visual bell,
  `CSI 8;rows;columns t` to a validated resize request, OSC 0/2 to title, and
  OSC 1 to icon-name. Title/icon values retain no more than 256 source bytes.
- OSC 52 maps to a payload-free clipboard rejection. OSC 8, other OSC,
  DCS/APC/PM/SOS, synchronized-update 2026, Kitty keyboard controls, REP, and
  underline-color controls are consumed or rejected before the engine.
- Underline-color filtering follows top-level SGR parameter boundaries: numeric
  aliases such as leading-zero `058` are contained, while `58`/`59` used as an
  indexed or RGB foreground/background color component remain ordinary color.
- Engine callbacks are themselves bounded before model collection. They never
  forward upstream `PtyWrite`, clipboard/title/color payloads, closures,
  lifecycle events, or Debug output.
- More than 32 events retain 31 events plus one saturating
  `EventsDropped { count }` summary. No rejected secret may appear in state,
  replies, snapshots, deltas, history, Debug, or logs.

## 6. Cell Extras and Persistent-State Bounds

- `MAX_CELL_TEXT_BYTES = 22`. Projection stores valid UTF-8 in fixed inline
  storage and adds only complete scalars. A visually blank default cell is
  normalized to empty; a styled blank remains one space.
- Before feeding any zero-width scalar, the engine checks the target cell. The
  base scalar plus combining scalars may not exceed 22 UTF-8 bytes.
- Combining usage is tracked separately for the main and alternate grids and
  bounded across the Session at 4,096 retained cells and 64 KiB retained
  combining bytes. The alternate grid is reconciled on screen switches; the
  active grid/history is reconciled on resize and when a conservative counter
  reaches a limit, so overwritten or evicted extras release quota.
- A scalar crossing the cell, cell-count, or byte cap is discarded before
  Alacritty can grow `CellExtra` and produces a bounded
  `UnsupportedSequence(Character)` classification.
- OSC 8 and underline-color inputs never reach the engine, so hyperlinks and
  unsupported underline-color extras cannot create unmetered cell heap state.
- Eight live Sessions, a maximum 240x80 viewport, 2,000 history rows, and wire
  frame bounds remain separate service limits.

These are hostile-input safety limits, not estimated model memory admission.
There is no aggregate terminal-memory projection, 128 MiB admission gate, or
256 MiB RSS gate. Alacritty allocation/capacity is allocator-owned and is
released when the Session model is dropped.

## 7. Projection, Snapshot, Delta, and Checkpoint

- Projection reads the active grid at display offset zero and maps only the
  current Zterm subset: indexed/RGB/default colors, bold/dim/italic/underline/
  inverse, wide head/spacer, cursor, and supported input modes. Hyperlinks,
  strike, hidden, underline color/style detail, palette state, Kitty keyboard,
  and graphics are not advertised.
- Only the Zterm allowlisted encoder creates client ANSI. Its vocabulary is
  printable UTF-8, reviewed SGR, CUP, ED2, EL2, home, cursor visibility,
  supported input modes, and the two top-level Zterm screen selectors. It emits
  no OSC/DCS/APC/PM/SOS or arbitrary upstream bytes.
- A full snapshot contains recent main history first and the complete latest
  active screen second. The core 8 MiB limiter removes only oldest complete
  history lines and never truncates the active screen.
- A checkpoint retains format, revision, size, active-screen identity, and one
  fixed projected active viewport. It retains neither Alacritty state, inactive
  screen, nor history; capacity is exactly `rows * columns` cells.
- Delta compares owned rows, redraws only changed rows, and restores
  cursor/modes. Future revision, format/size/screen mismatch, every-row change,
  or delta ANSI not smaller than full ANSI returns `Resync`. A newer revision
  whose complete projected state is identical returns a revision-only delta
  with empty ANSI.

## 8. History Contract

- Main history is read oldest-to-newest through Alacritty negative-line
  indexing without changing display offset, revision, checkpoint, or viewport.
- A page retains at most `MAX_HISTORY_PAGE_ROWS = 80` formatted rows.
- Monotonic append below capacity preserves epoch. Resize, clear/decrease,
  capacity eviction, or identity ambiguity advances epoch and returns Changed
  or Gap instead of splicing unverifiable rows.
- History while the alternate screen is active returns Changed; alternate
  history is never invented.

## 9. Validation and Error Matrix

| Condition | Required result |
| --- | --- |
| zero row/column | `TerminalError::InvalidSize`; no mutation |
| checked `rows * columns`/history arithmetic overflow | `TerminalError::AllocationOverflow`; no allocation |
| revision would exceed `u64::MAX` | `TerminalError::RevisionOverflow`; no mutation |
| canonical replies exceed 64 KiB/update | `TerminalError::ReplyOverflow`; driver fails closed |
| history row request is zero or over 80 | `InvalidHistoryPageSize`; no mutation |
| stale/invalid history cursor | Changed or Gap; never mixed rows |
| alternate screen history request | Changed |
| event/title/control/combining cap reached | bounded summary/classification; no payload leak |
| future/incompatible/inefficient checkpoint | one full `Resync` |
| active screen alone exceeds requested frame budget | preserve screen, clear history, return `false` |
| CI forces colored Cargo output | dependency-tree policy overrides color to `never` before byte comparison |

## 10. Required Evidence

- `cargo test -p zterm-terminal --all-features` covers the semantic corpus,
  exact query replies, chunk strategies, projection, history, snapshot/delta,
  resync, allocation/revision overflow, and active-screen normalization.
- `security_policy` covers control-string and secret containment, title/event/
  reply bounds, per-cell combining flood, both-screen/session combining cell
  and byte limits, synchronized updates, OSC 8/52, and malformed input.
- Lifecycle tests prove dropping the model releases the engine while an opaque
  checkpoint remains usable and engine/history-free.
- Daemon driver/session and real-PTY tests remain required for drain, reply
  ordering, resize, detach/reconnect, and lifecycle ownership.
- Format, Clippy with warnings denied, workspace tests, docs, cargo-deny,
  source policy, and dependency-tree isolation are required.
- The executable dependency-tree policy owns `CARGO_TERM_COLOR=never` and must
  produce the same canonical ASCII tree when its caller sets
  `CARGO_TERM_COLOR=always`; terminal presentation escapes are never graph data.

Do not run or recreate terminal throughput, latency, CPU, RSS, or candidate
comparison benchmarks for this contract.

## 11. Wrong vs Correct

Wrong:

```rust
pub struct TerminalCheckpoint {
    pub term: alacritty_terminal::Term<MyListener>,
}

client.write(upstream_event_payload);
session_registry.reserve(estimated_alacritty_bytes);
```

Correct:

```rust
let checkpoint = model.checkpoint();
match model.delta_or_resync(&checkpoint) {
    TerminalDeltaResult::Delta(delta) => apply(delta.ansi),
    TerminalDeltaResult::Resync(snapshot) => replace_with(snapshot),
}
```

Keep one host-authoritative model, expose only Zterm-owned contracts, bound
input-controlled state before the engine, and recover clients from latest
state rather than an output log.

For byte-exact dependency evidence, inheriting presentation settings is also
wrong:

```sh
# Wrong: CI can inject ANSI escapes into the captured bytes.
tree=$(cargo tree --charset ascii)

# Correct: the comparison owner fixes its own presentation contract.
tree=$(CARGO_TERM_COLOR=never cargo tree --charset ascii)
```
