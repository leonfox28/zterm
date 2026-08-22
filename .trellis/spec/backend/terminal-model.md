# Host-Authoritative Terminal Model Contract

## 1. Scope / Trigger

Apply this contract when changing `zterm-core::terminal`, its VT parser,
terminal query replies, side events, reconnect snapshots/checkpoints/deltas,
or the structural resource projection. The daemon owns the authoritative
model; controllers consume zterm-owned state and ANSI payloads and never own a
second parser contract.

The current implementation uses exactly `vt100 0.16.2`. Its types remain
private so the implementation can be replaced behind the same corpus and
public behavior if a later Gate proves it unsuitable.

## 2. Signatures

The retained public boundary is:

```rust
TerminalModel::new(size: TerminalSize, scrollback_rows: usize)
    -> Result<TerminalModel, TerminalError>
TerminalModel::ingest(&mut self, bytes: &[u8])
    -> Result<TerminalUpdate, TerminalError>
TerminalModel::resize(&mut self, size: TerminalSize)
    -> Result<TerminalUpdate, TerminalError>
TerminalModel::checkpoint(&self) -> TerminalCheckpoint
TerminalModel::snapshot(&self) -> TerminalSnapshot
TerminalModel::delta_or_resync(&self, checkpoint: &TerminalCheckpoint)
    -> TerminalDeltaResult
TerminalModel::state(&self) -> TerminalState
TerminalModel::resource_projection(&self) -> TerminalResourceProjection
```

`Revision` is the zterm-owned checked newtype shared by core, protocol
conversions, and the daemon driver. `TerminalSnapshot` carries `revision`, `size`, `active_screen`,
`screen_ansi`, `recent_history_ansi`, and `modes`. `TerminalDelta` carries
`from_revision`, `to_revision`, `size`, `active_screen`, `ansi`, and `modes`.
`TerminalCheckpoint` is opaque. No public field or signature may expose a
`vt100` type.

One update retains at most `MAX_SIDE_EVENTS_PER_UPDATE = 32` events. A title
or icon-name event retains at most `MAX_TITLE_BYTES = 256` source bytes.

## 3. Contracts

- Non-empty PTY chunks are ingested in order and advance one checked `Revision`
  revision. Empty input is a no-op. A successful resize also advances exactly
  one revision, including a same-size resize.
- `TerminalState` is the semantic comparison boundary: current screen, size,
  visible cells and styles, cursor and active style, and supported input modes.
- A reconnect client applies `recent_history_ansi` before `screen_ansi`.
  Snapshot replay into a fresh model must reproduce the latest semantic state.
- A checkpoint privately clones the parser screen. A delta is a single merged
  latest-state update, not a queue of intermediate revisions. Size mismatch,
  a future checkpoint, or a delta whose ANSI payload is no smaller than the
  full snapshot returns `Resync`.
- `vt100` formats only the active visible grid. When the alternate screen is
  active, Foundation snapshots do not serialize the inactive main grid or its
  history. This is the approved latest-active-screen reconnect boundary, not a
  promise of concurrent inactive-screen preview.
- The supported query replies are primary DA `CSI ?1;2c`, DSR status OK
  `CSI 0n`, standard CPR, and private `CSI ?6n` CPR. Other private markers such
  as `CSI >6n` are unsupported and produce no reply.
- OSC 52 clipboard reads/writes are rejected. Unknown OSC/DCS/APC payloads are
  never copied into replies, rendered state, snapshots, or deltas. Side events
  retain only bounded classifications or allowed bounded title/icon text.
- The resource projection is checked arithmetic over vt100 0.16.2's fixed
  inline cell slots. It excludes parser state, row/container overhead,
  snapshots, transient workload allocations, RSS, and throughput; those
  remain Foundation resource measurements and the projection must not be
  presented as an RSS limit by itself.
- The Foundation-measured admission baseline for the later session registry is
  2,000 scrollback rows, at most 240x80 cells, at most eight live sessions, and
  at most 128 MiB summed fixed-cell projection under a 256 MiB process-RSS
  target. Check all four bounds before construction and resize. The fallback
  viewport when no controller size exists is 120x40.

## 4. Validation & Error Matrix

| Condition | Required result |
| --- | --- |
| Zero rows or columns at construction or resize | `TerminalError::InvalidSize`; existing state remains unchanged |
| Revision would exceed `u64::MAX` | `TerminalError::RevisionOverflow`; parser state and size remain unchanged |
| Cell-capacity arithmetic overflows `usize` | `TerminalError::ResourceProjectionOverflow` |
| A requested model or resize exceeds a Foundation admission bound | The future session registry rejects it before mutating or allocating the authoritative model |
| Checkpoint revision is newer than the model | Return one full `Resync` snapshot |
| Checkpoint size differs from current size | Return one full `Resync` snapshot |
| Delta ANSI length is at least the snapshot ANSI length | Return one full `Resync` snapshot |
| More than 32 side events occur in one update | Retain 31 events plus one `EventsDropped { count }` summary |
| Title or icon input exceeds 256 source bytes | Retain a bounded lossy string with `truncated: true` |
| OSC 52 or an unknown control payload is received | Classify or drop it; never reproduce its decoded or encoded payload |
| Unsupported CPR private marker is received | No PTY reply; emit bounded `UnsupportedSequence(Csi)` |

## 5. Good / Base / Bad Cases

- **Good:** ingest an ANSI prefix, snapshot and checkpoint it, ingest a suffix,
  replay snapshot plus merged delta into a fresh model, then compare semantic
  state across whole, one-byte, fixed, and deterministic pseudo-random chunks.
- **Base:** a resize or future checkpoint returns `Resync`; the controller
  discards its old projection and applies the latest full snapshot once.
- **Good safety case:** DA/DSR/CPR generate only the documented constant or
  cursor-derived replies; OSC 52 produces an effect-rejected event without any
  clipboard payload.
- **Good resource case:** sum checked projections before accepting a new model
  or resize, then separately retain the session-count, viewport, and scrollback
  ceilings validated by the resource Gate.
- **Bad:** expose `vt100::Screen`, compare only generated ANSI bytes, retain an
  unbounded delta/event queue, forward unknown OSC bytes, describe the
  structural resource projection as measured RSS, or admit a model based only
  on the current lazily allocated RSS.

## 6. Tests Required

- `cargo test -p zterm-core --test terminal_corpus`: assert main/alternate,
  clear/scroll/cursor, indexed/RGB color, wide and combining Unicode, modes,
  resize, exact supported replies, unsupported CPR containment, bounded side
  events, and absence of both decoded and encoded unsafe payloads under every
  chunk strategy.
- `cargo test -p zterm-core --test terminal_snapshot_delta`: replay snapshots
  into a fresh model, apply a merged delta, and compare semantic state rather
  than bytes. Cover alternate/main transitions, bounded history, resize/future/
  large-delta resync, invalid size, and resource overflow.
- `cargo test -p zterm-core`: cover revision overflow without mutation.
- `cargo bench -p zterm-core --bench terminal_state` and
  `sh tests/foundation/resource-gate.sh`: retain the machine-readable candidate
  matrix, saturated RSS/CPU evidence, and accepted three/eight-session bounds.
- Ordinary workspace format, Clippy with warnings denied, tests, docs, and
  dependency policy remain required on every change.

## 7. Wrong vs Correct

### Wrong

```rust
pub struct TerminalCheckpoint {
    pub screen: vt100::Screen,
}

attachment_queue.push(every_intermediate_delta);
controller.write_to_local_terminal(unknown_osc_payload);
```

This leaks the chosen parser, creates unbounded attachment state, and permits
remote terminal output to trigger uncontrolled local behavior.

### Correct

```rust
let checkpoint = model.checkpoint(); // opaque parser baseline
match model.delta_or_resync(&checkpoint) {
    TerminalDeltaResult::Delta(delta) => apply(delta.ansi),
    TerminalDeltaResult::Resync(snapshot) => replace_with(snapshot),
}
```

Keep one host-authoritative model, expose only zterm-owned contracts, and use a
single latest-state delta or full resynchronization.
