# Technical Design: `alacritty_terminal` Host Engine

> Sections 1–31 retain the implemented engine/scroll evolution. Where they describe ANSI wire,
> fixed wire major, capability fallback, or legacy presentation, Section 32 direct cutover is the
> authoritative final architecture.

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
                       ProjectedScreen   semantic history   modes/events
                              │
                    snapshot / row delta / checkpoint
                              │ Zterm-owned semantic values
                              ▼
                  SessionService → local/remote attachment
                              │
                    AttachmentSurface + compositor
                              │ allowlisted desktop ANSI
                              ▼
                         sole presenter
```

外层 Ghostty/kitty/Alacritty只显示 sole presenter 最终编码的 ANSI。它与 daemon 的 Alacritty `Term` 是两个独立
状态机；没有库对象嵌套、FFI嵌套或共享 PTY ownership。

## 2. Fixed Decisions

| Topic | Decision |
| --- | --- |
| Engine | Official crates.io `alacritty_terminal = "=0.26.0"` |
| Features | `default-features = false`; no serde requirement |
| Safety | All Zterm crates inherit `unsafe_code = "forbid"` |
| PTY | Keep `portable-pty`; do not call Alacritty `tty`/`event_loop` |
| Session | One Session = one PTY = one model; no pane tree |
| Wire | Coordinated next-major/ALPN cutover; semantic surface/history only |
| Threading | Reuse existing ordered model thread and model mutex |
| Client isolation | Move engine owner out of `zterm-core` into `zterm-terminal` |
| Snapshot/delta | Zterm-owned semantic surfaces/full-row patches + owned row checkpoints |
| Security | Streaming allowlist before engine; no arbitrary upstream formatter/event reply |
| Rollback | Source revert; no runtime fallback or dual parser |
| Performance | No benchmark, comparison, SLO, RSS requalification or performance claim |
| Memory admission | Remove the aggregate 128 MiB terminal-memory gate; retain count/dimension and untrusted-input caps |
| Scroll owner | Per attachment; never Alacritty's shared `display_offset` |
| CLI scroll amount | One line per complete host-owned SGR wheel report; one event on child-owned paths |
| Scrollbar | Stable one-column main-screen gutter above width four; alternate screen reclaims it |
| Mobile seam | Core/proto metrics + line/offset actions; no CLI glyph/pixel policy in wire |

## 3. Crate and Dependency Layout

### `zterm-core`

Retains transport-neutral types and behavior:

- `TerminalSize`, `ActiveScreen`, `TerminalColor`, `TerminalStyle`, `TerminalCell`,
  `TerminalCursor`, `TerminalModes`;
- history-window anchors/results and generic viewport cache;
- bounded side events and `TerminalUpdate`;
- `TerminalSurfaceSnapshot`, `TerminalSurfaceDelta`, semantic history-window results;
- centralized semantic validation and content-redacted `Debug` implementations.

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

## 18. Scroll Follow-up Architecture

The post-release scroll change adds a presentation projection beside the existing live
snapshot/delta path. It does not add another terminal engine and does not mutate Alacritty's global
display offset.

```text
physical Ghostty / kitty / other terminal
        │ SGR mouse, keys, paste, SIGWINCH
        ▼
CLI HostInputCodec + ChromeLayout
        │
        ├─ child-owned event ───────────────► existing PTY input command
        │
        └─ Zterm-owned ScrollAction
                    │ local IPC / optional remote bridge
                    ▼
           Session ActorAttachment.scroll
                    │ offset/action under model lock
                    ▼
       TerminalModel::viewport_frame(offset)
                    │ canonical history + live rows, metrics
                    ▼
            TerminalViewportFrame
                    │
                    ▼
       CLI history renderer + scrollbar renderer
```

There are deliberately three different owners:

- Alacritty `Term` owns canonical main/alternate grids, retained rows and child-requested modes.
- one `ActorAttachment` owns only its scroll action baseline; it is reset on detach and is never
  copied into the Session/model or remote-resume checkpoint;
- the CLI owns physical terminal capture, its last returned frame/metrics, drag state and gutter
  presentation. A later Android client owns equivalent local presentation without the CLI glyphs.

The existing history-page API remains intact for explicit legacy fallback. New peers use the
viewport path because a page containing history alone cannot represent an offset of three rows from
live without also carrying the remaining visible screen rows.

## 19. Scroll Domain and Projection Math

Core adds transport-neutral values equivalent to:

```rust
struct TerminalScrollMetrics {
    epoch: Revision,
    revision: Revision,
    offset_from_bottom: u64,
    max_offset_from_bottom: u64,
    viewport_rows: u16,
}

enum TerminalScrollAction {
    ByLines(i32),       // positive = older/up; negative = newer/down
    ToOffset(u64),
}

enum TerminalViewportResult {
    Frame { disposition: ExactOrRebased, metrics, rows },
    Live { metrics },
    Changed { epoch, revision },
    Gap { epoch, revision },
}
```

Names may be adjusted to existing Rust conventions during implementation, but meanings and bounds
are fixed. `offset_from_bottom == 0` is live. `max_offset_from_bottom` is the retained main-history
row count for the represented epoch. `viewport_rows` must equal the validated current model height;
the client cannot request an arbitrary render allocation.

For model height `R`, retained history `H`, clamped offset `O = min(requested, H)`, row `i` in a
history frame is read directly from:

```text
Alacritty Line(i - O), for i in 0 .. R
```

Thus `O=3` yields `Line(-3), Line(-2), Line(-1), Line(0) ... Line(R-4)`. `O=H` starts at the oldest
retained row; all frames still contain exactly `R` rows by continuing into the current main screen.
Projection uses `project_row` and `encode_history_row`-equivalent allowlisted output under one model
lock. It never invokes `Grid::scroll_display`, never changes `display_offset`, and never advances the
model revision or any live checkpoint.

`ActorAttachment.scroll` stores the last accepted epoch, max offset and current offset. Before a
relative action:

1. obtain the current extent from the same locked model;
2. if the epoch matches, add `current_max - previous_max` before the user's delta so output appended
   below capacity keeps the same logical content pinned;
3. if identity changed because of resize, clear or eviction, clamp the old offset into the current
   extent and mark the replacement `Rebased`;
4. apply the signed delta with checked/saturating bounds, then create one frame from that exact
   epoch/revision.

An absolute `ToOffset` maps directly into the current extent and is suitable for track click/drag.
If the final offset is zero, the response is `Live`; the CLI uses the existing full-sync handshake
instead of trying to reconstruct live cursor/modes from history rows. If main history is unavailable
or the authoritative screen is alternate, the typed `Changed/Gap` path is used—never a partially
mixed frame.

While a history frame is visible, incoming live deltas update the renderer's authoritative
revision/modes without overwriting the frozen frame. A subsequent action performs the adjustment
above. If exact row identity no longer exists, `Rebased` replaces the entire frame at the closest
valid retained offset. This is bounded and deterministic even though an evicted row cannot be
recovered.

## 20. Proto, Wire, and Version Skew

Proto v1 receives additive messages for scroll metrics/action, a viewport request and a viewport
frame. Exact field numbers and enum values are allocated after the current terminal history fields;
existing numbers are never reused. `TerminalSnapshot` and `TerminalDelta` receive an optional live
metrics field with offset zero. The field is absent when a legacy peer authored the message or when
no valid main-screen extent can be asserted.

The new request/frame use the next unused terminal wire kinds after 314. Existing bit 18 is
`AGENT_EVENTS`, so `Capabilities::TERMINAL_VIEWPORT` uses the next free bit 19 and gates use across
remote peers:

- request is a control frame and stays below the existing 1 MiB control bound;
- response is content, contains at most the validated viewport height of canonical rows, and stays
  below the existing 8 MiB content bound;
- attachment ID, request ID, deadline, one-response correlation and redacted `Debug` follow the
  current history request/page rules;
- one semantic viewport request may be outstanding per view. Relative wheel deltas are accumulated
  within a fixed signed bound while pending; an absolute drag target replaces the older queued
  target. No physical-event-sized queue is introduced.

New client/new server uses the semantic viewport. A new client connected through an old remote
daemon sees no capability, never sends the unknown kind, keeps the stable main gutter blank until
legacy history bounds are known, and uses the current bounded page browser without claiming exact
continuous scrolling or drag. An old client ignores the optional snapshot/delta fields and retains
its old behavior. Local same-version IPC always uses the new messages, while validation still treats
missing/unknown enum values and mismatched attachment IDs as malformed input.

The wire carries metrics, actions, outcomes and canonical row bytes only. It does not carry Unicode
track/thumb characters, color, gutter width, pixel coordinates, touch velocity or platform UI
policy. Android can reuse the state/action vocabulary, but its renderer and gesture-to-line adapter
remain a separate task.

## 21. Input Ownership and Resume State Machine

Physical capture is an outer-terminal implementation detail, separate from child modes. The CLI
keeps `DECSET 1003 + 1006` active so it can observe candidate mouse events; daemon-authored
`TerminalModes` decide the semantic owner.

```text
history visible?
  yes -> Zterm owns wheel/page navigation
  no  -> event targets visible scrollbar gutter?
           yes -> Zterm chrome owns it
           no  -> child mouse reporting active?
                    yes -> encode exactly one allowed mouse report
                    no  -> alternate screen + alternate-scroll?
                             yes -> encode exactly one cursor key
                             no  -> main screen Zterm history owns wheel
```

The gutter is outside the child PTY rectangle, so a click at its physical column is never clamped
into or forged as the child's final column. Child-rectangle routing has no application-name or TERM
heuristic. Herdr/Pi-style SGR mouse declarations naturally select the one-report branch.

CLI constants are:

- one host-owned SGR wheel report: one line (the 2026-09-03 smooth-viewport follow-up supersedes
  the v0.1.11 three-line input constant without changing the 315/316 wire action contract);
- PageUp/PageDown: `max(viewport_rows - 1, 1)` lines;
- one child-owned wheel report: one encoded report with no multiplier;
- alternate-scroll: one normal/application cursor sequence with no multiplier.

Any ordinary key, paste or prefix input in history enters `ResumePending`. Input remains in the
existing fixed byte bound while the CLI requests a current live snapshot. If authoritative screen
state changes gutter geometry, the resize coalescer submits that one geometry and keeps input fenced
through the resulting sync. Only after the replacement is flushed and transport is Active is the
retained input written exactly once. PageDown or a scrollbar jump reaching offset zero uses the same
resume path without child input.

Every renderer transaction that can contain daemon-authored ANSI writes in this order:

```text
child snapshot/delta OR viewport rows
    -> status/chrome repair needed for that transaction
    -> HOST_INPUT_CAPTURE
    -> one flush
```

This unconditional final reassertion fixes the current reset bug. Cleanup remains centralized in
`TerminalGuard`; error and signal paths cannot bypass its raw-mode/mouse restoration.

## 22. CLI Chrome Layout and Scrollbar

`ChromeLayout` is derived from physical size, remote status-row presence, product viewport limits
and the effective presentation screen:

```text
usable_rows = min(physical_rows - status_rows, max_viewport_rows)
usable_cols = min(physical_columns, max_viewport_columns)

main and usable_cols > 4:
    child = usable_rows x (usable_cols - 1)
    gutter_column = usable_cols
alternate or usable_cols <= 4:
    child = usable_rows x usable_cols
    gutter = none
```

Before the initial snapshot identifies the screen, preparation conservatively uses main geometry;
normal new shell sessions therefore never change width when history first appears. Attaching to an
already-alternate application may require one correction to full width after its first snapshot.

The scrollbar spans only `usable_rows`; a remote status row and unsupported physical overflow are
outside its hit-test rectangle. With track height `T`, visible rows `V`, maximum offset `M`:

```text
thumb_len = M == 0 ? 0 : max(1, floor(T * V / (V + M)))
travel    = T - thumb_len
thumb_top = M == 0 ? travel : round((M - offset) * travel / M)
```

All products use checked `u128` intermediates and clamp to the track. The CLI plans to render `▕`
for track and `▐` for thumb, with explicit save/restore cursor and SGR reset. No history means the
reserved cell column is cleared but no track is drawn. Track click maps the pointer to an absolute
offset centered on the thumb. Drag stores a bounded grab-row within the thumb, updates the latest
absolute target on motion, and ends on release/capture loss.

In live main, the gutter stays reserved even if a child has mouse reporting. Deliberate input in the
visible gutter is host chrome; input in the `N-1` child columns follows child modes. In alternate,
the full `N` columns are handed to the child and no scrollbar hit target exists. The authoritative
alternate snapshot, composed before Zterm chrome, owns and repaints the reclaimed column; Zterm must
not append a stale-gutter clear afterward. A physical shrink to four columns or fewer similarly clips
the former gutter instead of clearing through a coordinate that could clamp onto child content.

The CLI records the last successfully presented gutter separately from current desired layout. If
Main retains ownership while the gutter moves, it clears the committed old column before drawing the
final desired gutter last in the same transaction. Multiple layout changes before presentation still
compare against that committed baseline, and failed writes/flushes do not advance it. Alternate-to-
Main draws the newly Zterm-owned gutter after child content.

The effective presentation screen remains Main while a Zterm history frame is pinned, even if a
background child delta declares Alternate. This prevents a hidden child state transition from
stealing gestures or resizing underneath frozen history. On live resume, the latest authoritative
screen is reconciled once.

`ResizeCoalescer::last_submitted` remains the loop breaker. A live Main→Alternate transition changes
`N-1` to `N` at most once; the resize-produced replacement still says Alternate and therefore is a
no-op. Alternate→Main is symmetric. The child may draw once before Zterm observes the mode and once
after SIGWINCH; this accepted two-phase redraw is tested for eventual geometry, no input loss and no
oscillation rather than described as impossible.

## 23. Failure, Reconnect, and Rollback

- A malformed/out-of-order viewport frame is a protocol error under the existing terminal driver
  correlation rules; it is never rendered.
- A `Changed/Gap` response is content-free, so it leaves the last complete host presentation intact
  and uses the existing live-resume path on ordinary input. `Rebased` is a complete replacement, so
  no old/new epoch rows coexist.
- Transport reconnect and controller takeover discard the attachment-local scroll state and return
  to live synchronization. Presentation offset is not part of remote resume identity.
- A capability-less peer uses the existing history-page fallback. Capability negotiation, not an
  intentionally failing unknown message, selects the path.
- The implementation can be rolled back as source changes. Additive proto fields/kinds have no
  persistent data migration; old peers ignore fields and never receive kinds they did not negotiate.

## 24. Scroll Verification Design

The narrow unit layer proves projection formulas, signed/absolute action clamps, epoch rebase,
scrollbar integer geometry and input ownership. Protocol tests prove enum/field stability, size
bounds, one-outstanding correlation, old-peer fallback and Debug redaction. CLI pseudo-terminal
tests record exact output order so `HOST_INPUT_CAPTURE` follows snapshots, ordinary deltas, resyncs
and history frames, and cleanup removes it.

Integration fixtures exercise main shell, Herdr/Pi-style `1049/1000/1002/1003/1006`, alternate-scroll,
background mode changes while pinned, width 4/5, remote status rows, drag bursts and main/alternate
resize synchronization. Release evidence requires real macOS and Linux local/direct/relay smoke
before beginning Android work. These are functional checks; no throughput, latency, CPU or RSS
benchmark is added or run.

## 25. Smooth Desktop and Future-Mobile Viewport Architecture

The new path separates authoritative terminal storage from interactive presentation:

```text
Alacritty Term in daemon
        │ read-only contiguous window at one revision
        ▼
TerminalHistoryWindowFrame (anchor + coordinates + bounded rows)
        │
        ▼
ViewportCache<Row> in client core
        ├─ desktop row adapter -> atomic ANSI presentation
        └─ future Android row adapter -> native renderer
```

The daemon remains the only owner of the grid, history, revision, reflow and eviction. The client
owns desired/presented offset, a bounded row window, prefetch state and gesture pacing. The new path
is stateless at the Session boundary: authorization and correlation are retained, but the request
does not update `ActorAttachment.scroll`. Legacy 315/316 continues to use that field only for peers
without the new capability.

The desktop presenter still needs its own atomic output boundary. Local cache removes network RTT
from an interaction, but it does not prevent a host terminal from displaying partial ANSI writes.
Every desktop replacement is therefore buffered as:

```text
CSI ?2026h -> hide cursor as needed -> content -> status/gutter
           -> HOST_INPUT_CAPTURE -> final cursor/modes -> CSI ?2026l
           -> one write_all -> one flush
```

No request transition renders a loading/returning frame. History rows are written before `EL` clears
their remaining tail; no row is cleared with `EL2` before its replacement content. Cleanup emits
`CSI ?2026l` even after a partial/error path. Child-originated DEC 2026 remains rejected by the daemon
ingress policy; this host presentation mode is a separate outer-terminal state machine.

## 26. History Window Domain and Coordinate Math

Core values are equivalent to:

```rust
struct TerminalHistoryWindowAnchor {
    epoch: Revision,
    revision: Revision,
    max_offset_from_bottom: u64,
    viewport: TerminalSize,
}

struct TerminalHistoryWindowRequest {
    anchor: TerminalHistoryWindowAnchor,
    target_offset_from_bottom: u64,
    older_margin_rows: u16,
    newer_margin_rows: u16,
}

struct TerminalHistoryWindowFrame {
    disposition: ExactOrRebased,
    anchor: TerminalHistoryWindowAnchor,
    target_offset_from_bottom: u64,
    first_row_from_live_top: i64,
    rows: Vec<Row>,
}
```

The exact names may follow existing conventions, but the coordinates are fixed. For response height
`R` and retained history `H`, the current live top is coordinate `0`, history is `[-H,0)`, and the
live screen is `[0,R)`. A viewport at offset `O` requires `[-O,R-O)`. With requested margins `A`
(older) and `B` (newer), the server returns:

```text
start = max(-H, -O - A)
end   = min( R,  R - O + B)
rows  = every logical row in [start, end), top to bottom
```

The client may render `O` exactly when `[-O,R-O)` is fully contained in the cached range. Its slice
starts at checked index `(-O - start)` and has exactly `R` rows. The response is invalid unless row
count equals `end - start`, every coordinate is representable, and total rows are at most
`MAX_HISTORY_WINDOW_ROWS = 240`. Request margins satisfy `A + B <= 2R`, so one response is at most
three maximum-height screens; the existing 8 MiB content cap remains independently authoritative.

On a valid same-epoch/same-size request with `H_current >= H_anchor`, the server resolves a target
expressed in anchor coordinates as:

```text
O_current = min(H_current, O_requested + (H_current - H_anchor))
```

This pins content across rows appended below the view. An epoch/size change or extent decrease
clamps the target into current bounds and returns a complete `Rebased` frame. A structurally invalid
or future anchor returns `Gap`; alternate screen returns `Changed`. The model never mutates while
projecting any outcome.

## 27. Client Cache Reducer and Fetch Policy

`zterm-core` owns a generic `ViewportCache<Row>`-style reducer with no transport, async runtime,
ANSI, clock, mouse or pixel types. It owns:

- current anchor and contiguous `[first_row, first_row + rows.len())` coordinates;
- desired and last-presented offsets;
- whether a complete visible slice exists;
- one in-flight request description and one latest queued target;
- deterministic install/rebase/invalidate and prefetch-decision transitions.

The desktop layer supplies canonical ANSI rows and wall-clock drag pacing. A future Android layer can
supply semantic rows and pixel gesture physics without changing the range/anchor reducer.

Fetch policy for height `R` is bounded and directional:

- opportunistically fetch around live after activation when valid main history exists;
- a first miss requests up to two screens on the likely travel side and the current screen, never
  more than 240 total rows;
- a middle target normally requests one screen of margin on each side;
- within `max(R/2, 1)` rows of a scrollable cache edge, issue one background prefetch;
- while a request is pending, local moves inside the cache continue immediately and only the latest
  uncovered target is retained;
- a response is displayed only if it contains the complete latest desired viewport. Otherwise its
  safe rows may replace/merge the cache, the existing displayed frame remains, and one request for
  the latest target follows.

Same-epoch live metrics whose history extent grows translate cached coordinates downward by the
extent delta and increase pinned offsets by the same amount. A prefetched-live cache is invalidated
by a live revision it cannot update safely. A pinned frozen cache may survive ordinary same-epoch
live changes, but resize/reflow, epoch change, extent decrease, explicit resume, true reconnect and
takeover invalidate it. No cached row enters a resume checkpoint or server attachment state.

## 28. Additive Wire and Fallback

Wire allocation is append-only:

```text
Capabilities::TERMINAL_HISTORY_WINDOW = 1 << 20
WireKind::TerminalHistoryWindowRequest = 317   (control)
WireKind::TerminalHistoryWindowFrame   = 318   (content)
```

The request carries attachment ID, anchor, target offset and margins. The frame carries attachment
ID, Frame/Changed/Gap outcome, Exact/Rebased disposition, current anchor, resolved target,
`first_row_from_live_top`, canonical rows, and current epoch/revision for content-free outcomes.
Debug output contains only structure, counts and total bytes.

New/new peers use 317/318. If a remote peer lacks bit 20, the local bridge exposes legacy semantic
viewport bit 19 when available and the CLI uses 315/316; without bit 19 it retains the 312/313 pager.
No peer receives an unnegotiated kind. One window request may be outstanding per view; stream loss
completes it once with a correlated content-free Gap before `Reconnecting`, exactly like the existing
read-only viewport/history controls.

The initial payload remains independently encoded canonical ANSI rows because that path already has
bounded projection, redaction and desktop consumers. Android adds a separately negotiated semantic
row encoding in its own task. It reuses these coordinates, anchors, reducer transitions and bounds;
the current task does not add a parser, renderer, or unused cell wire.

## 29. Presentation and Cache Verification

Tests are layered by owner:

- model: coordinate formula, exact/rebased/gap/alternate, append pinning, sizes and unchanged state;
- core cache: local moves without effects, slice formula, prefetch threshold, stale/latest response,
  append translation, invalidation and fixed maximum rows;
- proto/wire: 317/318 and bit 20 stability, conversion, bounds, redaction and old-peer exclusion;
- Session/local/remote: authorization, sync fence, correlation, one outstanding/latest queued,
  reconnect Gap and bit20 -> bit19 -> pager fallback;
- CLI: one-line host wheel, child one-report, drag 33 ms plus final release, local cache hit, edge
  prefetch/jump miss, no request repaint, DEC 2026 byte order, one write/flush and cleanup reset;
- real PTY: quiet-shell first scroll, sustained wheel, thumb drag, nested-TUI mode enter/exit and
  detach restoration on the owning macOS/Linux local/direct/relay environments.

No benchmark or fixed latency assertion is part of acceptance. The observable contract is that a
cache hit renders locally without a viewport request and that an unavailable target never exposes a
partial/blank frame.

## 30. Host-owned Viewport Presentation Cadence

The desktop CLI adopts Herdr's event-driven latest-frame pattern only for Zterm-owned cached
viewport presentation. `MIN_VIEWPORT_PRESENT_INTERVAL` is 16 ms. This is a minimum interval between
eligible presentations, not a periodic 60 Hz task and not a claim of compositor/vsync alignment.
When idle, no timer remains active and no frame is emitted. The constant belongs to the desktop CLI;
the generic cache reducer and future Android renderer remain clock- and refresh-rate-neutral.

Each decoded host wheel report still applies one checked row to the desired offset immediately.
All reports decoded from one stdin delivery are reduced before presentation is considered, so the
usual Ghostty three-report delivery can produce one latest frame without treating byte chunking as a
gesture contract. If multiple deliveries arrive inside the 16 ms interval, the CLI keeps one dirty
bit and one deadline; when eligible it presents the newest complete cached slice exactly once.
Reports split across the interval may produce two frames, but never an unbounded series of
back-to-back writes. Intermediate offsets may be skipped visually; accumulated offset and final
direction may not be skipped.

The pacer owns presentation timing only. It does not own input semantics, cache coordinates, remote
requests, or render bytes. A host interaction first mutates the existing `ViewportController`, then
executes any history-window request/prefetch immediately, and finally marks the current complete
slice dirty. Child-owned mouse reports and alternate-scroll translations bypass the pacer and are
written to the PTY one-for-one. The existing 33 ms drag request pacing stays independent; a final
thumb release must flush the latest complete local position rather than leave it waiting behind a
timer.

One pending deadline is added to the active terminal `tokio::select!`. The deadline carries no frame
or scroll offset: the controller remains the single source of latest desired state. On expiry, the
CLI renders only if the view is still host-owned history and a complete slice is available. Any
immediate render that already contains the latest viewport marks the dirty state presented and
cancels the deadline. Return-live/resume, authoritative snapshot or resync, resize/reflow, true
reconnect, transport-state replacement, detach, and cleanup cancel pending work before a stale
history frame can be emitted. A live delta observed behind a still-pinned compatible history view
does not cancel a valid pending local presentation merely because the model revision advanced.

Presentation remains one DEC 2026 transaction through the existing rendering path. There is no
global PTY-output frame scheduler, no extra framebuffer, no physical wheel-gesture heuristic, and no
change to the Alacritty model, protocol, daemon, one-Session/one-PTY ownership, or unsafe policy.

Verification uses a controllable clock/pure pacer where possible and byte-level CLI integration
tests for side effects. It covers same-delivery three-report reduction, cross-delivery coalescing,
deadline expiry, opposite directions, clamp/cache miss and immediate prefetch, final drag release,
child-owned forwarding, and cancellation by every authoritative transition. Real Ghostty smoke
confirms perceived pacing, while tests assert exact offsets and transaction counts rather than an
OS scheduler's wall-clock precision.

## 31. Cross-Emulator Right-Margin-Safe Delta Encoding

This section specifies a necessary compatibility-path invariant, not the complete presentation
architecture. The user subsequently clarified that an extent-only repair must not be treated as the
final solution while daemon-authored ANSI and attachment-local chrome remain separate physical
writers. Section 32 now specifies the confirmed full migration. This section remains mandatory for
the mixed-version adapter, but neither section is authorized until the reconverged final summary is
followed by a new explicit implementation approval.

The canonical ANSI delta contract is a row replacement, not a terminal-cursor simulation shortcut.
For a terminal width `W`, a replacement row has a visual extent `E` in grid cells. Cells `[0, E)` are
painted by the replacement and cells `[E, W)` are stale suffix only when `E < W`. The encoder may
erase exactly that suffix; it must never emit an inclusive erase when `E == W`.

The changed-row sequence becomes:

```text
CUP(row, 1)
SGR reset
encode replacement content and return visual extent E
SGR reset
if E < W:
    CUP(row, E + 1)       # physical, one-based column of first stale cell
    EL0                   # clears only [E, W)
restore final cursor and terminal modes as before
```

`E` is measured in projected terminal cells, not UTF-8 bytes, scalar values, grapheme count, or the
outer terminal's observed cursor. A trailing wide continuation belongs to its leading wide glyph;
if that glyph reaches the right margin, `E == W`. A wrapped row is encoded through the full grid
width and likewise has no suffix clear. Styled/default-background cells already considered visually
significant by projection remain part of the extent. An empty replacement has `E == 0` and explicitly
clears from physical column 1.

This does not add an extent cache or another projection pass. The existing row encoder already
computes this exact `end` to omit trailing default cells; it now returns the value to its caller.
The one branch represents an unavoidable protocol distinction: ECMA `EL0` has no zero-length form,
so an empty suffix must not issue it.

The explicit second `CUP` is intentional even when the encoder believes it knows the current cursor.
Printing the final physical cell can leave an emulator in pending-wrap with the cursor logically on
that cell. Ghostty's erase-right includes the current cell, whereas Alacritty/tmux accepting the old
sequence made the defect invisible to replay-only tests. Positioning by semantic extent eliminates
that cross-emulator dependency for ASCII, combining sequences, wide glyphs and autowrap states.

No new long-lived presentation state or failure transition is introduced. The encoder still builds
one byte buffer before the CLI writes it; existing write/flush success remains the only commit point
for outer chrome authority. Content remains before suffix cleanup, and the containing CLI frame
remains one DEC 2026 transaction. Main/Alternate region ownership, child mouse forwarding, host
history cadence, capture/status composition and final cursor restoration do not move into the
terminal model.

This design deliberately does not forward child DECAWM or nested DEC 2026 modes to the physical
terminal. Zterm virtualizes child output into an authoritative model and emits canonical outer ANSI;
forwarding private rendering state would couple two terminal state machines and would not make an
inclusive zero-length suffix erase correct. Application/glyph/theme detection and forced full-frame
repaints are also forbidden.

Herdr reaches a different final-output contract because it owns a complete semantic `FrameData` at
the presenter. A nested Pi/TUI is parsed into pane cells, Herdr chrome is overlaid into the same
buffer, and a retained-frame encoder compares cells and writes literal blanks for removed glyphs.
That removes row-tail erasure but requires per-cell comparisons, narrow/wide overlap invalidation,
style/run batching and host-autowrap lifecycle. Section 32 adopts that owning invariant through
Zterm-native semantic types and an explicit desktop presenter; this Section 31 encoder remains only
the compatibility implementation for peers that cannot negotiate the new path.

Verification is layered:

- terminal encoder byte tests establish that a full-width replacement has no `EL0`, while a short or
  empty replacement explicitly positions and clears the exact suffix in default rendition;
- semantic replay tests cover full-width ASCII, styled final cells, combining content, a wide glyph
  ending at the margin, wrapped rows, minimum width and resize, using a strict Ghostty-like
  pending-wrap/erase oracle in addition to Alacritty replay;
- CLI transaction tests start from committed Main and Alternate frames, inject a generic child-owned
  right-edge cell plus incremental update/wheel path, and assert final-writer ordering, one
  write/flush, capture/status coexistence and failed-write retry;
- real Ghostty smoke uses Herdr as one external fixture, while Linux verification uses a generic
  right-margin fixture so acceptance never depends on detecting that application.

Nested child synchronized-output boundaries remain a separately testable optimization. They may be
specified only if a new failing test demonstrates tearing after this row-replacement invariant is
correct; they are not bundled into the right-margin fix.

## 32. Semantic Composition and Single Presenter Direct Cutover (Approved)

### 32.1 Root cause and owning invariant

The right-margin symptom exposed a local legacy-encoder error, but that obsolete path is deleted by
the direct cutover. The repeated gutter, status,
return-live, and nested-TUI defects reveal a broader ownership failure. The current system has one
authoritative child model but several independent physical writers. Joining their byte fragments in
one DEC 2026 envelope provides batching, not a complete desired state or a shared commit point.

The owning invariant is therefore:

> During an active semantic attachment, every visible cell, cursor decision, and physical input mode
> is derived from one complete `ComposedFrame`; exactly one presenter may transition the outer
> terminal, and its baseline advances only after the whole transition is written and flushed.

```text
child PTY bytes
      |
      v
one daemon Alacritty model + private ProjectedScreen
      |
      | bounded semantic snapshot / full-row patch / history rows
      v
client AttachmentSurface + ViewportCache<TerminalSurfaceRow>
      |
      | choose complete live or history source
      v
ChromeLayout + compositor(status, gutter, cursor, host modes)
      |
      v
one complete sparse ComposedFrame
      |
      v
DesktopPresenter(last successfully flushed frame)
      |
      | DEC 2026 + exact cell runs + cursor/modes; one write + flush
      v
outer terminal (Ghostty, Kitty, Alacritty, or another VT consumer)
```

Herdr and PiAgent do not appear in any decision branch. Their behavior is explained by standard
active-screen, mouse-mode, surface, and layout state. The project-wide classification process is
recorded in `root-cause-and-architecture-thinking-guide.md`; new contradictory evidence reopens this
design rather than adding a compensating application or repaint rule.

### 32.2 Core semantic values

`zterm-core::terminal` owns one renderer-neutral terminal presentation contract:

```rust
pub struct TerminalSurfaceRow {
    pub cells: Vec<TerminalCell>,
    pub wrapped: bool,
}

pub struct TerminalSurface {
    pub size: TerminalSize,
    pub active_screen: ActiveScreen,
    pub rows: Vec<TerminalSurfaceRow>,
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

Snapshot rows and cells are rectangular and exact. A delta is a set of unique, increasing,
full-width row replacements plus the complete resulting cursor/mode/metrics metadata. It applies
only to the exact revision, size, and active screen. Size, screen, projection-format, or checkpoint
mismatch produces a full semantic snapshot. A revision-only transition may carry zero changed rows;
it is not discarded when other semantic metadata changes.

Semantic history-window values reuse Section 26's anchor, target, disposition, and coordinate math,
but contain `TerminalSurfaceRow` rather than ANSI bytes. `ViewportCache<Row>` stays generic, so no
second cache state machine is introduced.

Validation is performed before values reach a renderer:

- non-zero dimensions within existing product limits;
- exactly `size.rows` rows and `size.columns` cells per row;
- valid advancing revisions, ordered/in-range delta rows, in-range cursor, and metrics consistent
  with snapshot revision/height/screen;
- valid UTF-8 strings no longer than the existing 22-byte cell cap and containing no C0/C1/ESC;
- every wide head followed by one empty continuation, every continuation preceded by its head, and
  no head beginning in the final column;
- content-frame size at or below 8 MiB and content-redacted `Debug` at core/proto/daemon boundaries.

The source model's wide flags are authoritative. The desktop backend never infers its next cursor
from Unicode display width, so a host's width-table difference cannot propagate into later runs.

### 32.3 Proto encoding and coordinated wire cutover

Semantic presentation is mandatory, not negotiated. The release increments the product wire major
and its normal/pair ALPN identifiers. Local readiness and authenticated network handshakes reject an
older binary before it can open an attachment. No presentation preference or feature capability is
needed.

The protobuf source/package and generated Rust module move directly from `proto/zterm/v1`,
`zterm.v1`, and `zterm_proto::v1` to `proto/zterm/v2`, `zterm.v2`, and `zterm_proto::v2`. The build
does not generate or expose both schema majors. Independently versioned data messages such as
`PairTicketV1` may keep their names and unchanged binary shapes; that does not preserve a wire-v1
transport path or require migrating trusted-device state.

Within the new wire major the canonical terminal content kinds are:

```text
kind 301  TerminalSemanticSnapshot             (content)
kind 302  TerminalSemanticDelta                (content)
kind 317  TerminalHistoryWindowRequest         (control)
kind 318  TerminalSemanticHistoryWindowFrame   (content)
```

The old ANSI payloads at 301/302/318, pager 312/313, stateful viewport 315/316, temporary semantic
kinds 319..321, capability bits 17/19/20/21, and `TerminalPresentationEncoding` field/enum are
removed. Reusing the primary content numbers is safe because the wire major and ALPN differ. Unknown
or old peers fail explicitly; there is no fallback matrix.

The remote bridge decodes only enough protobuf envelope metadata to validate session/attachment
identity, revision, request correlation, and frame bounds before replacing its private attachment
ID and forwarding. It does not convert terminal cells, construct ANSI, or choose a representation.

### 32.4 Model, checkpoint, session, and resume flow

`ProjectedScreen` and its inline cells remain private to `zterm-terminal`. The model projects once
under its existing lock and converts the projection and changed rows directly into semantic values.
`encode_full`, `encode_delta`, row ANSI encoding, `recent_history_ansi`, and their DTOs are deleted.
A semantic snapshot intentionally has no recent-history stream; the bounded
history-window/cache path is the scroll source of truth on desktop and mobile.

`TerminalCheckpoint` remains a semantic projected screen. Every initial update, snapshot
acknowledgement, latest/final update, sync-required response, and history-window response uses the
semantic type directly; `ActorAttachment` stores no presentation-family state. An exact remote
checkpoint may resume with a delta.

This changes presentation only. The existing one-Session/one-PTY/model rule, ordered model thread,
fixed no-drop PTY queue, shared revision watermark, query-reply ordering, controller/takeover rules,
read-only history lock, detach/reconnect behavior, and final drain remain the lifecycle authority.

### 32.5 Client attachment state and visible source

The client owns one `AttachmentSurface` with the latest validated complete surface and
revision. It performs transactional updates:

1. validate a snapshot or validate a delta against the existing baseline;
2. build/install the replacement surface or row set without touching the committed physical frame;
3. expose one complete desired source to the compositor;
4. request resync on a gap while retaining the last complete presentation.

History uses one `ViewportCache<TerminalSurfaceRow>`, retaining the already implemented
anchor translation, bounded prefetch, one-in-flight/latest-target behavior, 16 ms desktop
presentation cadence, 33 ms drag-request pacing, and one complete SGR wheel report per host-owned
line.

When pinned in history, live deltas continue advancing `AttachmentSurface`, but the compositor uses
the complete cached main-screen slice and hides the child cursor. A background child transition to
Alternate does not seize wheel ownership or replace the pinned source. Return-live switches once to
the latest complete live surface and then reconciles layout/PTY size through the existing resize
coalescer.

### 32.6 Region layout and compositor

Layout assigns ownership before painting:

```text
Main:      [ child columns 1..N-1 ][ Zterm gutter N ]
Alternate: [ child columns 1..N                     ]
Remote:    content regions above + status at physical final row
```

The gutter condition (`Main` and usable width greater than four), status-row condition, viewport
caps, and Main/Alternate resize behavior stay unchanged. What changes is that clearing or drawing a
region is no longer a post-child operation. The compositor always starts from a complete retained
visible row source and writes each region into one desired frame. A transferred gutter column is
therefore simply a child cell in the next Alternate frame.

Conceptually:

```rust
struct ComposedFrame {
    physical_size: TerminalSize,
    rows: BTreeMap<u16, ComposedRow>,
    cursor: ComposedCursor,
    host_modes: HostModeState,
    layout_identity: LayoutIdentity,
}
```

Rows use absolute one-based physical coordinates and contain only explicitly owned cells. This
sparse shape prevents an allocation proportional to arbitrary `u16 rows * columns`; owned content is
bounded by the product child rectangle plus one optional status row. Composition rejects overlapping
regions, cells outside the physical viewport, malformed wide spans, and unbounded status output.
Coordinates owned by the previous frame but no longer owned by the next become default blank during
the presenter diff, unless a resize already invalidates the whole baseline.

Status text, connection path/RTT, reconnect state, scrollbar track/thumb, cursor visibility, and host
mode policy are pure contributors. They cannot write stdout. The compositor hides the cursor for
history, clamps a live child cursor to the allocated child region, and keeps remote status outside
child coordinates.

### 32.7 Desktop presenter and physical mode policy

The lifecycle `TerminalGuard` remains the only writer before/after the active phase: it establishes
raw mode and the outer alternate screen, then always disables modes/restores the screen on exit. It
hands the active phase to one `DesktopPresenter`; child Main/Alternate never toggles the outer
screen.

For a known baseline, `DesktopPresenter`:

1. diffs absolute cells in the union of old/new owned coordinates;
2. expands every change through old and new wide-character spans;
3. groups adjacent cells where safe, starts each run with absolute `CUP`, and emits allowlisted SGR;
4. writes explicit default-style blanks for removals and never uses incremental `EL0`/`EL2`;
5. positions the desired cursor with absolute `CUP`, applies visibility/style and one host-mode
   transition, then ends the synchronized update.

Writing the final physical column may leave the host in pending-wrap, but the next run/cursor is
always absolutely positioned. Pending wrap is never used as state and cannot turn a zero-length
suffix into an erase.

`HostModeState` derives application cursor/keypad, bracketed paste, and focus observation from child
semantics where required for input fidelity. Physical mouse ownership is always Zterm SGR
any-motion capture (`1003` + `1006`); child mouse mode/encoding and alternate-scroll are consumed only
by the input router, which emits exactly one child event or host scroll action.

Missing baseline, resize, screen/layout identity change, or prior I/O
failure triggers full presentation: reset style, clear the outer alternate screen, and paint the
complete desired frame. A complete transition is first built in memory, enclosed by one DEC 2026
begin/end, then sent through exactly one `write_all` and one `flush`. Only success commits the cloned
frame. On either failure, the baseline becomes unknown and a best-effort DEC 2026 end is attempted
without replacing the original error; the next attempt is necessarily full.

Connection-status and reconnect changes go through the same compositor/presenter. The standalone
`RECONNECTING_STATUS` newline writer and independent active status/gutter render entry points are
removed. Unchanged composed frames produce no physical write.

### 32.8 Superseded-path deletion contract

The direct cutover deletes, rather than repairs or isolates, every ANSI presentation producer and
consumer: core snapshot/delta DTOs, terminal encoders, driver/session family wrappers, proto
messages/conversions, local/remote negotiation, CLI renderer/history byte cache, and their tests.
The pager and stateful viewport protocols are also removed because their only remaining purpose was
fallback for peers without the stateless semantic history window.

Deletion is reachability-based and includes modules, public aliases, generated artifacts, Cargo
features/dependencies, errors, and fixtures left without a current consumer. Generic snapshot
acknowledgement, resync, input, resize, detach, controller/takeover, final-drain, terminal guard,
history coordinates, and the generic cache remain because the semantic architecture still consumes
them. Removed numeric capability bits remain unassigned; removed kind numbers are either canonical
semantic kinds in wire major two or absent. No deprecated shim or dual-schema module is retained.
Source-policy tests reject reintroduction of ANSI presentation or family/downgrade branches.

### 32.9 Module boundaries

No new third-party renderer, Ratatui dependency, terminal parser, or speculative shared crate is
introduced:

```text
zterm-core     semantic rows/surfaces/patches/validation + generic cache
zterm-proto    semantic protobuf/wire conversion, bounds, redacted Debug
zterm-terminal private Alacritty projection + semantic producer
zterm-daemon   Session ownership + local/remote semantic forwarding
zterm-cli      terminal_ui.rs orchestration/input
               terminal_ui/surface.rs       AttachmentSurface / visible source
               terminal_ui/composition.rs   regions, chrome, ComposedFrame
               terminal_ui/ansi_presenter.rs sole semantic desktop presenter
```

Exact filenames may be consolidated when a module would otherwise be trivial, but ownership cannot
be folded back into independent stdout writers. `zterm-core` and `zterm-proto` remain free of
Alacritty and desktop ANSI dependencies, so Android can consume semantic surfaces and cache
coordinates. Android-specific layout, font/glyph rendering, IME, touch/fling, and vsync are deferred.

### 32.10 Verification, rollback, and release

Verification is contract-oriented rather than application-oriented:

- core/proto: round trips, exact rectangular shape, revisions, malformed controls/wide pairs,
  cursor/metrics, max dimensions/frame size, fixed existing numbers, redacted content;
- terminal: semantic snapshot/patch equivalence for whole/fragmented input, styles, Unicode,
  combining, wide/wrapped rows, screen/resize, history, and absence of ANSI presentation code;
- session/transport: initial snapshot and resume delta, ack/resync, final drain, takeover, reconnect,
  local/direct/relay, and explicit rejection of the previous wire major;
- client: transactional surface apply, history/live source choice, cache invalidation, region
  non-overlap, gutter/status transfer, cursor/modes, no active writer outside the presenter;
- presenter: strict desired-frame oracle, rightmost/wide/styled-blank transitions, one DEC 2026
  transaction/write/flush, unchanged no-op, partial failure then full retry;
- source deletion: no wire-v1 generated module, legacy DTO/encoder/kind/capability/preference/fallback,
  unreachable cutover residue, or secondary active writer;
- system: application-neutral nested TUI fixture plus Herdr as a real Ghostty/macOS smoke, and
  separately recorded macOS/Linux local/direct/relay evidence.

Each internal phase has a source rollback point and may be committed for review. Rollback before
release means reverting the complete wire-major cutover; there is no runtime compatibility switch.
There is no extent-only release and no declaration of completion until the semantic path, explicit
old-major rejection, quality gates, spec synchronization, and macOS/Linux acceptance all pass.

The user explicitly excluded throughput, latency, CPU, RSS, and comparative performance benchmarks.
Existing dimension/content safety bounds remain; the removed aggregate memory admission does not
return. Product code continues to inherit `unsafe_code = "forbid"`.

### 32.11 Rejected alternatives

- **Herdr/application detection**: treats evidence as scope and fails the next equivalent TUI.
- **Repeated/full repaint after wheel**: hides one symptom, adds latency/flicker, and leaves ownership
  split.
- **Daemon-only exact ANSI diff**: still cannot include attachment-local status/gutter or serve a
  native mobile renderer.
- **CLI reparse of daemon ANSI**: creates a second terminal state machine and untrusted parser while
  preserving a desktop-only boundary.
- **Daemon-owned final chrome**: mixes per-session model state with per-attachment/platform layout.
- **More `ReplaceRow`/`ClearRange` commands**: grows a pseudo-terminal protocol instead of
  transporting complete domain semantics.
- **Global PTY frame scheduler or child DEC 2026 passthrough**: changes timing but cannot establish a
  correct final desired frame.
- **Immediate shared presentation crate**: no current Android UI/layout implementation proves a
  stable shared compositor API; core semantic values/cache are the justified reusable seam.

### 32.12 Authorization boundary

The direct-cutover scope and one-release decision are captured. This revised design, matching PRD,
research, implementation phases, and manifests must be presented as one final planning summary. A
new explicit user approval after that summary was received; product implementation may resume.
