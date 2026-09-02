# Technical Design: `alacritty_terminal` Host Engine

## 1. Design Summary

本设计用一个 host-only `zterm-terminal` crate包住官方 `alacritty_terminal 0.26.0`。Alacritty
负责解析普通VT input并维护grid/history/modes；Zterm继续负责输入安全策略、公开semantic
types、query replies、snapshot/delta/history wire、count/dimension limits和PTY/session生命周期。

```text
hosted process
    │ PTY output bytes
    ▼
portable-pty reader ── fixed no-drop queue ── TerminalDriver model thread
                                                │
                                                ▼
                                     TerminalIngressPolicy
                                     ├─ host replies/events
                                     ├─ drop OSC8/52/unknown strings
                                     └─ allowed VT bytes
                                                │
                                                ▼
                              alacritty vte::ansi::Processor
                                                │
                                                ▼
                                      Term<BoundedEventSink>
                                                │
                              ┌─────────────────┼─────────────────┐
                              ▼                 ▼                 ▼
                       ProjectedScreen    history encoder    modes/events
                              │
                    snapshot / row delta / checkpoint
                              │ Zterm-owned values + allowlisted ANSI
                              ▼
                  SessionService → local/remote attachment → CLI/UI
```

外层 Ghostty/kitty/Alacritty只显示上述 Zterm ANSI。它与 daemon 的 Alacritty `Term` 是两个独立
状态机；没有库对象嵌套、FFI嵌套或共享 PTY ownership。

## 2. Fixed Decisions

| Topic | Decision |
| --- | --- |
| Engine | Official crates.io `alacritty_terminal = "=0.26.0"` |
| Features | `default-features = false`; no serde requirement |
| Safety | All Zterm crates inherit `unsafe_code = "forbid"` |
| PTY | Keep `portable-pty`; do not call Alacritty `tty`/`event_loop` |
| Session | One Session = one PTY = one model; no pane tree |
| Wire | Keep current protobuf/wire major and Zterm ANSI projection |
| Threading | Reuse existing ordered model thread and model mutex |
| Client isolation | Move engine owner out of `zterm-core` into `zterm-terminal` |
| Snapshot/delta | Zterm-owned canonical ANSI encoder + owned row checkpoints |
| Security | Streaming allowlist before engine; no arbitrary upstream formatter/event reply |
| Rollback | Source revert; no runtime fallback or dual parser |
| Performance | No benchmark, comparison, SLO, RSS requalification or performance claim |
| Memory admission | Remove the aggregate 128 MiB terminal-memory gate; retain count/dimension and untrusted-input caps |

## 3. Crate and Dependency Layout

### `zterm-core`

Retains transport-neutral types and behavior:

- `TerminalSize`, `ActiveScreen`, `TerminalColor`, `TerminalStyle`, `TerminalCell`,
  `TerminalCursor`, `TerminalModes`;
- history direction/cursor/page/result;
- bounded side events and `TerminalUpdate`;
- `TerminalSnapshot`, `TerminalDelta`, `TerminalDeltaResult`;
- `MAIN_SCREEN_SELECTION_ANSI`, `ALTERNATE_SCREEN_SELECTION_ANSI`;
- `TerminalSnapshot::limit_ansi_payload` and redacted `Debug` implementations.

It deletes `vt100` and has no dependency on `zterm-terminal`, Alacritty or `vte`.

### `zterm-terminal`

New modules:

```text
crates/terminal/src/
├── lib.rs          public host boundary and re-exports
├── model.rs        TerminalModel, revision/history orchestration
├── engine.rs       Term/Processor/EventSink adapter
├── ingress.rs      bounded CSI/control-string policy
├── projection.rs   Alacritty grid -> fixed Zterm projection
└── ansi.rs         full screen/row/history allowlisted encoder
```

Public host types are `TerminalModel`, `TerminalCheckpoint`, `TerminalError` and
Zterm-owned updates/snapshots. Every public signature uses Zterm core types; upstream types remain
private. `TerminalCheckpoint` has a content-redacted `Debug` and does not expose cells.

### `zterm-daemon`, CLI, and proto

- daemon imports model/checkpoint/error from `zterm-terminal` and DTOs from core;
- proto remains core-only; CLI continues to depend on daemon because it is the combined host binary,
  so its full transitive graph includes the engine, but its terminal UI has no direct engine dependency
  and imports only core/proto-owned terminal values;
- terminal model integration tests move from core to terminal ownership; the obsolete
  `terminal_state` measurement target is removed rather than moved;
- `just ci-windows` explicitly tests `zterm-terminal`, while workspace Unix jobs already include it.

Dependency-policy tests assert:

```text
zterm-cli ───────────────► zterm-daemon ─────► zterm-terminal ─► alacritty_terminal ─► vte
      └─────────────────► zterm-core          └► zterm-core / zterm-platform / zterm-proto
zterm-proto ─────────────► zterm-core
zterm-core / zterm-proto -X-> zterm-terminal / alacritty_terminal / vte
zterm-cli Cargo.toml -----X-> direct zterm-terminal / alacritty_terminal / vte dependency
```

## 4. Terminal Model Shape

Conceptual private fields:

```rust
pub struct TerminalModel {
    engine: AlacrittyEngine,
    ingress: TerminalIngressPolicy,
    revision: Revision,
    scrollback_rows: usize,
    history_epoch: Revision,
    retained_history_rows: usize,
}

struct AlacrittyEngine {
    processor: alacritty_terminal::vte::ansi::Processor,
    term: alacritty_terminal::Term<BoundedEventSink>,
    output: SharedBoundedOutput,
    legacy_x10_mouse: bool,
    combining_budget: CombiningBudget,
}
```

The exact field split may avoid a self-referential sink by sharing only a small bounded output
collector through `Arc<Mutex<_>>`. No callback borrows `TerminalModel`, and no raw pointer is used.

`EngineSize` privately implements Alacritty `grid::Dimensions`; production code does not use the
upstream test-only `TermSize`. Configuration is:

```text
scrolling_history = requested bounded rows
osc52              = Disabled
kitty_keyboard     = false
default cursor     = current block/default behavior
selection/vi mode  = unused
```

`Term` and `Processor` are created, mutated and dropped through safe Rust. They may move with the
outer `TerminalModel` before/inside the existing owner thread; no custom `Send`/`Sync` implementation
is added.

## 5. Ordered Ingest Transaction

`TerminalModel::ingest(bytes)` behaves as follows:

1. Empty bytes return the current revision and empty output.
2. Preflight `revision.checked_next()` and reset the per-update bounded output collector.
3. Pass bytes to `TerminalIngressPolicy`, preserving its state across calls.
4. The policy streams ordinary text and approved grid/mode controls to the engine in causal order;
   host-owned queries/events are handled at their exact boundary and dangerous strings are dropped.
5. Each engine feed drains `BoundedEventSink` immediately into the same ordered collector.
6. Normalize/cap any zero-width content touched by that feed and update the active screen's combining
   budget. A screen transition reconciles the newly active budget before more printable input.
7. On success, commit exactly the preflight revision, refresh history identity, and return one
   `TerminalUpdate`.

Partial CSI/OSC at the end of a nonempty input still advances the external revision once, matching
current behavior even though no visible grid cell changes. Completion in a later input belongs to
that later revision.

If a fixed policy/reply/resource bound is exceeded, no payload is logged. A policy-string overflow is
contained and reported as a bounded unsupported event. An internal synchronization/reply-cap failure
is terminal-fatal: the driver records failure and wakes all waiters rather than continuing with an
unknown reply stream.

## 6. `TerminalIngressPolicy`

### Purpose

Alacritty deliberately implements more terminal behavior than Zterm currently advertises, and some
upstream events contain strings or PTY replies. Zterm must therefore own the trust boundary before
calling the engine. This component is a bounded ECMA-48 framer/policy layer, not another grid/parser.

### States and caps

The implementation keeps only enough state to distinguish:

- Ground/incremental UTF-8;
- short ESC sequence;
- CSI through its final byte;
- OSC through BEL or ST;
- ignored DCS/APC/PM/SOS through ST;
- discard-until-terminator after overflow.

Initial exact caps, covered by unit/adversarial tests:

```text
MAX_CONTROL_SEQUENCE_BYTES = 256
MAX_CONTROL_STRING_BYTES   = 1_024
MAX_REPLY_BYTES_PER_UPDATE = 64 KiB
MAX_SIDE_EVENTS_PER_UPDATE = 32      (existing)
MAX_TITLE_BYTES            = 256     (existing)
```

C0/C1 introducers, split `ESC \`, CAN/SUB cancellation, arbitrary external chunk boundaries and
malformed UTF-8 are part of the state-machine tests. An overlong control never falls back to Ground
as printable payload.

### Dispatch table

| Input | Owner/result | Sent to Alacritty? |
| --- | --- | --- |
| printable UTF-8, CR/LF/BS/HT, ordinary grid CSI/ESC | Alacritty state | yes |
| BEL | Alacritty `Event::Bell` -> `AudibleBell` | yes |
| `ESC g` | Zterm `VisualBell` | no |
| primary DA (`CSI c`/`0c`) | canonical `CSI ?1;2c` | no |
| DSR 5 / CPR 6 / private CPR ?6 | canonical bounded reply from current cursor | no |
| other DA/DSR/window/color/mode query | unsupported classification, no reply | no |
| `CSI 8;rows;cols t` | validated `ResizeRequested` event | no |
| DECSET/DECRST 9 | Zterm legacy X10 mouse bit | optionally no-op upstream |
| DECSET/DECRST 2026 | unsupported classification; content remains immediately processed | no |
| OSC 0/2 | bounded `TitleChanged` | no |
| OSC 1 | bounded `IconNameChanged` | no |
| OSC 52 | bounded clipboard read/write rejection | no |
| OSC 8 / other OSC | `UnsupportedSequence(Osc)` without payload | no |
| DCS/APC/PM/SOS | bounded unsupported classification without payload | no |

Ordinary approved mode CSI remains handled by Alacritty. The policy only tracks a Zterm-owned X10
mouse bit because Alacritty 0.26 does not expose DECSET 9, while current Zterm does.

No `Event::PtyWrite` is blindly trusted. `BoundedEventSink` accepts Bell and bounded non-content
signals, maps known upstream replies only as defense-in-depth, and rejects every unexpected reply,
clipboard/color/size closure, exit or child event. Zterm lifecycle never follows an Alacritty event
because its tty/event loop is unused. Event values are pattern-matched directly and never formatted
with upstream `Debug`, whose title/clipboard variants can contain content.

## 7. Cell Projection and Compatibility Normalization

Private projection uses inline text rather than one heap `String` per checkpoint cell:

```rust
struct ProjectedCell {
    text: InlineCellText, // valid UTF-8, MAX_CELL_TEXT_BYTES
    wide: bool,
    wide_continuation: bool,
    style: TerminalStyle,
}

struct ProjectedRow {
    cells: Box<[ProjectedCell]>,
    wrapped: bool,
}

struct ProjectedScreen {
    version: u16,
    size: TerminalSize,
    active_screen: ActiveScreen,
    rows: Box<[ProjectedRow]>,
    cursor: TerminalCursor,
    modes: TerminalModes,
}
```

`InlineCellText` uses safe array/index operations only. The explicit initial cap is 22 UTF-8 bytes,
matching the old parser's fixed cell payload scale; a base scalar is retained first, then only whole
combining scalars that fit. No invalid UTF-8 or partial scalar is produced.

Alacritty mapping:

- `Color::Indexed` -> indexed; `Color::Spec` -> RGB;
- named black..white/bright colors -> indexes 0..15;
- foreground/background default-family names -> Default;
- flags -> current bold/dim/italic/any-underline/inverse subset;
- strike/hidden/underline color/hyperlink are not projected;
- `WIDE_CHAR` and `WIDE_CHAR_SPACER` map to current wide flags;
- a visually empty default cell becomes empty `contents`; a styled blank remains one space;
- zero-width scalars append only while per-cell and session budgets permit.

Initial session caps are 4,096 cells with dynamic combining storage and 64 KiB total retained
combining payload across both screens. Usage is tracked per main/alternate screen and reconciled by a
bounded full-grid scan only at threshold or screen/resize transitions. Entering alternate resets its
budget with Alacritty's reset grid; returning to main recounts main/history before accepting more.
Unsupported hyperlink and underline-color sources are removed before ordinary text can clone them.

These caps are product security constants and upgrade-review inputs, not a reconstruction of the
removed cross-Session memory quota and not performance tunables.

## 8. Semantic State and Modes

`state()` creates public `TerminalState` from the current `ProjectedScreen`. The active grid is always
viewed at display offset zero; Zterm never calls Alacritty scroll-display/selection APIs.

Mode mapping:

- `APP_CURSOR`, `APP_KEYPAD`, `BRACKETED_PASTE`, `FOCUS_IN_OUT`, `ALTERNATE_SCROLL` map directly;
- mouse click/drag/motion map to PressRelease/ButtonMotion/AnyMotion;
- policy-owned DECSET 9 maps to Press when no stronger mode is active;
- `UTF8_MOUSE` and `SGR_MOUSE` map to the current encoding enum;
- Kitty keyboard bits, vi mode, urgency and unsupported modes do not enter public state.

Cursor row/column is clamped defensively to the validated viewport before conversion to `u16`.
Cursor visibility comes from `SHOW_CURSOR`; cursor style template maps through the same style subset.

## 9. Canonical ANSI Encoder

Only Zterm's encoder creates network/display ANSI. It emits a small reviewed vocabulary:

- printable UTF-8 cell text;
- SGR reset and current foreground/background/bold/dim/italic/underline/inverse parameters;
- CUP, ED 2, EL 2, home;
- cursor show/hide;
- app cursor/keypad, bracketed paste, focus, alternate scroll and current mouse mode/encoding;
- Zterm screen metadata selectors only at the top-level positions described below.

It never emits OSC, DCS, APC, PM, SOS, hyperlinks, palette mutation, arbitrary upstream replies or
unknown private modes. A test-only vocabulary validator scans every snapshot/delta/history fixture
and rejects output outside this grammar.

### Full active screen

1. Prefix `MAIN_SCREEN_SELECTION_ANSI`; when active screen is Alternate, immediately prefix
   `ALTERNATE_SCREEN_SELECTION_ANSI` as required by the existing CLI metadata contract.
2. Reset SGR and all controlled modes to a known baseline; clear/home.
3. Encode each visible row with explicit CUP. Skip wide continuation cells. Trim only visually empty
   default trailing cells; emit styled trailing blanks through the last meaningful cell.
4. Restore cursor position, active cursor template style, visibility and controlled input modes.

### Changed-row delta

Checkpoint format version, size, revision and active screen must match. For each changed row:

```text
CUP(row, 1) + SGR reset + EL 2 + canonical row
```

Then restore cursor/template/modes. No selector is needed while screen identity is unchanged. A
screen change always resyncs, avoiding any assumption about inaccessible inactive Alacritty grids or
client history. Empty delta is valid for a newer revision with no visible semantic change. If encoded
delta length is not smaller than full snapshot, return full resync.

## 10. Checkpoint and Snapshot Ownership

`TerminalCheckpoint` contains exactly:

- format version;
- revision, size and active screen;
- one fixed latest `ProjectedScreen` baseline.

It contains no Alacritty handle, history, dirty iterator or inactive grid. The hidden test helper is
updated to report `rows * columns` retained cell capacity rather than the old fresh-parser
`rows * columns * 2`; retained scrollback remains zero. SessionService allows one controller plus one
pending takeover, so at most two steady attachment checkpoints remain owned per Session. This is a
lifecycle invariant, not an aggregate byte-admission formula.

Snapshot history is encoded before the screen. `TerminalSnapshot::limit_ansi_payload` remains in core
unchanged: it removes only oldest complete reset/CRLF-delimited history rows and never truncates the
screen. Proto still owns the final 8 MiB frame gate.

## 11. Scrollback and History Identity

When main is active:

```text
history_size = term.grid().history_size()
oldest line  = Line(-(history_size as i32))
newest line  = Line(-1)
```

Page start remains relative to oldest retained row zero. Row encoding reads exact negative lines and
never calls `scroll_display`; rows return oldest-to-newest with reset guards and no screen selector.

Identity policy remains deliberately conservative:

- append while history is below cap keeps epoch;
- a decrease/clear changes epoch;
- any successful resize changes epoch because reflow can change physical rows;
- once at capacity, an ingest which may have evicted a row changes epoch;
- entering alternate makes history requests return Changed without inventing alternate history;
- returning main refreshes the retained count; any ambiguity changes epoch.

Cursor validation and maximum 80-row page behavior remain in core-owned result types.

## 12. Resource Policy After Removing Aggregate Admission

The migration deletes `TerminalResourceProjection` and the core
`ResourceLimits::aggregate_cell_projection_bytes` field. It does not replace them with a model-size,
high-water or `size_of::<Cell>` estimate. Session create and resize therefore cannot fail merely
because an estimated sum crosses 128 MiB.

Alacritty owns and manages its grid allocations, row cache, resize retention and the VTE processor's
approximately 2 MiB synchronized-update capacity. Zterm neither duplicates these buffers nor tries
to infer when the allocator returns them. A Session drop still drops its `TerminalModel`, checkpoints
and Alacritty state through ordinary Rust ownership.

Removing that product quota does not make PTY output an unbounded allocation authority. The following
independent limits remain:

- maximum eight live Sessions, 240x80 viewport and 2,000 history rows;
- fixed ingress sequence/string, reply, event and title/icon bounds;
- per-cell and per-Session combining-content bounds;
- existing snapshot/protobuf frame bounds and latest-only attachment semantics.

All size conversions and `rows * columns` allocations still use checked arithmetic. Tests validate
these explicit bounds and object release, but do not estimate total terminal bytes, measure RSS or
assert a 128/256 MiB result. Alacritty may retain capacity after a viewport shrinks until the Session
is dropped; that is an accepted consequence of this decision.

## 13. Driver, PTY, and Session Integration

No new terminal or writer actor is introduced:

- reader thread continues blocking on PTY and pushing owned 8 KiB chunks into capacity-8 queue;
- model thread pops in order, calls `ingest`, writes bounded replies through the existing `PtyIo`
  mutex, publishes latest revision, and wakes failures;
- snapshot/checkpoint/history/resize continue locking the shared model;
- child interruption remains independent from a potentially blocked PTY writer;
- attachment drop does not touch process/model lifetime.

Imports/error mapping change, and the obsolete aggregate terminal-memory reservation bookkeeping is
removed. Existing failure, startup, reaper and finalization tests remain authoritative.

Resize remains:

```text
validate size/session dimensions + preflight next revision
       -> native portable-pty resize
       -> Term::resize + revision commit
       -> revision publish
```

Unexpected synchronization failure after native resize is terminal-fatal; ordinary invalid size or
revision exhaustion fails before native mutation. The independent live-session count limit remains a
create-time admission rule, not a resize-time memory estimate.

## 14. Stable Hosted Capability Profile

The product login-shell builder sets:

```text
TERM=xterm-256color
COLORTERM=truecolor
```

after resolving the effective account, independent of daemon/CLI parent environment. Existing
HOME/SHELL/cwd/login argv0 behavior remains. Explicit low-level fixture commands keep their explicit
environment contract unless a test opts in.

This prevents a Zterm session from claiming `xterm-ghostty`, `xterm-kitty`, tmux or another outer
terminal identity merely because of how the daemon was launched. Zterm's exact DA/query policy and
encoder grammar define the actual advertised subset. Hosted native tests verify the terminfo/profile
works on shipped macOS/Linux floors; missing `xterm-256color` is a migration blocker, not a reason to
inherit the outer TERM.

## 15. Platform and Release Design

- Registry source plus Cargo.lock is sufficient; no Zig compiler, C compiler integration, vendored
  Ghostty tree, build.rs network fetch or Git source allowlist is added.
- Alacritty default serde is disabled. Its Unix/Windows tty modules still compile as part of the
  package, but Zterm's dependency isolation and static source check forbid product references to them.
- Unix workspace tests run on existing macOS/Linux arm64/x86_64 jobs.
- `just ci-windows` adds `-p zterm-terminal`; this proves shared compilation/tests, not Windows login
  PTY runtime.
- existing release-readiness and formal build paths compile the daemon transitively, inspect target/
  platform floors and confirm there is no new terminal dylib dependency.
- core/proto dependency graph is the mobile deliverable. No Android/iOS engine job or runtime claim is
  added.

## 16. Cutover Sequence and Rollback

The implementation may temporarily contain old core model code and the new unconnected terminal
crate on the development branch. No binary runs both engines and there is no feature switch.

1. Freeze/extend Zterm-owned expected fixtures.
2. Add official dependency and new crate with direct adapter tests.
3. Implement policy, projection, security caps, encoder, delta and history.
4. Move model tests, switch daemon imports and remove aggregate memory accounting.
5. Set hosted capability profile and run driver/session/CLI/platform tests.
6. Delete vt100 code/dependency, update specs/docs/policy, run final gates.

If a correctness/security/resource/platform gate fails, revert the migration commits to the last
single-vt100 source state. There is no persisted terminal state or wire migration to reverse.

## 17. Verification Boundary

Required evidence is functional and structural:

- unit/corpus/snapshot/history/size-overflow/adversarial tests;
- driver/session/local/remote/CLI real-PTY regression tests;
- format, Clippy, docs, cargo-deny, dependency graph and source policy;
- hosted macOS/Linux/Windows plus four native release-readiness builds.

Explicitly excluded evidence:

- `cargo bench ... terminal_state` execution;
- `tests/foundation/resource-gate.sh` RSS/CPU execution;
- vt100-vs-Alacritty or Ghostty-vs-Alacritty throughput comparison;
- latency, CPU, build-time or binary-size optimization targets beyond existing release hard limits.

The old `terminal_state` measurement target and `tests/foundation/resource-gate.sh` are removed with
the obsolete 128/256 MiB policy rather than retargeted. Future profiling, if requested, starts as a
separate task with an explicit question and workload.
