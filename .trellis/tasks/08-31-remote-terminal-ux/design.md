# Remote terminal resilience and UX design

## 1. Outcome and invariants

本任务在现有 remote attachment 上增加三个相互配合、但所有权清晰的能力：

1. resize/closure 竞态优先呈现已经到达的 typed lifecycle outcome，不再把可归类的关闭暴露成裸 `Broken pipe`；
2. 普通 main-screen 输出可以通过 wheel/trackpad 和无修饰 PageUp/PageDown 浏览 daemon 保留的有界历史；
3. remote attachment 底部固定一行三字段状态栏：`device | direct/relay/-- | RTT/--`。

以下不变量不变：

- `TerminalModel` 仍是唯一 VT parser 和历史真相源；CLI 只消费 daemon-authored typed state/ANSI row，不创建第二 parser。
- Session/PTY 生命周期不属于 local socket、Iroh connection、history viewport 或状态栏。
- local CLI 不创建 Endpoint，也不读取 route/IP；连接状态只从既有 `ConnectionBroker` 做脱敏投影。
- 所有新增队列、页、输入暂存和刷新频率都有固定上限；不增加磁盘 transcript。
- child terminal bytes、Device ID、IP、Relay URL 和 ticket 不进入状态栏、诊断、Debug 或日志。

## 2. Cross-layer data flow

```text
host PTY
  -> retained TerminalDriver
  -> one TerminalModel (screen + <= 2,000 main-history rows)
       -> live snapshot/delta ------------------------------┐
       -> bounded history rows + cursor -- HISTORY_PAGING --┤
                                                            v
remote normal-ALPN attachment <-> reconnect bridge <-> same-UID IPC
                                                            |
ConnectionDemand -> selected Iroh path kind + RTT ----------┤
frozen local Device alias ----------------------------------┤
                                                            v
CLI ViewportController -> child content rectangle + bottom status row
```

History messages cross the same authenticated attachment stream and retain its exact attachment/principal checks. Connection status is same-UID local IPC only and is never accepted from or emitted to the remote host.

## 3. Physical geometry and status chrome

### 3.1 Geometry owner

The CLI derives two sizes from every physical TTY size:

- remote attachment with at least two physical rows: `content = rows - 1`, `status = 1`;
- local attachment, or a one-row physical terminal: `content = rows`, `status = 0`.

The initial attach request and every coalesced resize publish only `content` to the daemon/remote PTY. The CLI retains the physical size for chrome placement. Growing back from one row re-enables the status row and sends the newly reduced content size once.

### 3.2 Status projection

A new complete, local-only typed observation contains:

- the exact local attachment ID;
- the attach-time frozen, validated local Device alias;
- redacted selected path kind: unknown/direct/relay;
- optional selected-path RTT as a bounded integer millisecond value.

The exact DeviceId remains the routing/retry authority. The alias is display-only and cannot retarget a live attachment. `ConnectionDemand` projects the currently promoted candidate's selected Iroh path and `Path::rtt()` without exposing the transport address. The bridge emits an initial unknown observation, one immediately after activation, and at most one changed observation per second. Reconnecting/non-active transport state makes the CLI render `-- | --` even if an older path sample exists.

### 3.3 Rendering

The logical text is exactly:

```text
<device> | <direct|relay|--> | <integer ms|-->
```

`StatusRenderer` owns a single output sequence which:

1. saves the child cursor/style;
2. positions on the physical bottom row outside the child viewport;
3. resets inherited SGR, enables reverse video, erases/fills the complete row, and writes display-width-clipped text;
4. resets SGR and restores the child cursor/style.

It uses the terminal theme's default foreground/background through reverse video. It does not query OSC colors or use fixed RGB/palette colors. Unicode truncation occurs only at UTF-8/display-cell boundaries. The status row is redrawn after every child snapshot/delta, status change, and physical resize so child clears cannot leave it missing. The renderer clears a previous in-bounds status position when a resize moves the row.

## 4. Authoritative bounded history

### 4.1 Core projection

`TerminalModel` gains a zterm-owned history projection; private `vt100` types remain hidden. A page is a bounded vector of daemon-formatted ANSI rows plus cursor metadata, not raw PTY bytes and not a cloned parser.

The cursor contains:

- a history epoch derived from the model's checked `Revision`;
- the model revision observed when the page was produced;
- the page's stable start row measured from the oldest retained main-history row;
- the returned row count and oldest/newest bounds.

The model updates the epoch only when existing row identities may have changed: resize, history shrink/clear, reaching or mutating at the fixed capacity where eviction cannot be distinguished safely, or another non-monotonic history transition. Monotonic append below capacity keeps the epoch, so a pinned cursor can continue paging while new rows append at the newest end. An epoch mismatch returns `history_gap`; a compatible epoch with a stale revision may still return a consistent page. Invalid ranges return `history_gap`, never a best-effort splice.

The model renders history from a cloned screen view and never changes the authoritative parser's scrollback offset. Page size has a fixed protocol maximum and the ordinary 8 MiB frame gate remains final authority.

### 4.2 Wire and attachment routing

The reserved `Capabilities::HISTORY_PAGING` bit becomes active only when both peers implement:

- one bounded `TerminalHistoryRequest` carrying attachment ID, cursor/start intent, and requested direction/range;
- one correlated `TerminalHistoryPage` carrying redacted cursor/bounds, formatted rows, and an explicit `ok`, `history_changed`, or `history_gap` outcome.

The host Session actor validates the attachment, controller/synchronization state, page bound, current main-screen eligibility, cursor epoch, and range exactly once. Remote bridge forwarding uses the existing correlated control budget; it adds no reconnect owner and does not replay a failed page request across epochs. Same-UID decoding revalidates the local attachment ID and page bounds before projecting a typed CLI event.

If a peer lacks `HISTORY_PAGING`, the attachment remains usable and zterm does not invent incomplete local history. The current release acceptance updates both endpoints, so normal tests require the negotiated bit.

## 5. CLI viewport and input routing

### 5.1 View state

One `ViewportController` owns three exhaustive states:

- `Live`: physical child content matches the acknowledged daemon revision;
- `History`: a bounded daemon-authored row window and cursor are displayed while the Session continues;
- `ResumePending`: a full replacement snapshot has been requested and any initiating input is held in one bounded accumulator.

At the bottom, snapshots/deltas render normally. Entering history fetches a prefetched bounded row window and pins it. While pinned, live events continue to be drained and their latest modes/revision are observed, but their ANSI is not applied to the historical physical viewport. Scrolling within the fetched window is local; reaching a window edge issues at most one correlated page request.

Returning to offset zero, pressing a normal key, or pasting requests the existing full `TerminalSyncRequest`. Only after the replacement snapshot is rendered, acknowledged, and the attachment is Active does the UI forward retained input exactly once. The accumulator is capped by the existing control/input payload budget; overflow is a typed resource error rather than silent loss or unbounded allocation. Resize updates geometry in every view state and preserves the cursor when its epoch/range remains valid.

Bracketed paste is one host-input event, not a stream of ordinary key chunks. The sole host-input codec retains the complete start/content/end sequence across stdin reads under the same fixed bound, and paste content bypasses the detach prefix. When a resume Snapshot and `Active` arrive before the paste tail, activation waits for the complete paste, then runs the authoritative stdin reader fence before releasing the retained unit exactly once.

`history_changed`/`history_gap` is shown as a short zterm-owned notice inside the history viewport. It never adds a fourth status field. The stale and new row sets are not concatenated; the user can return live or make a fresh history request.

### 5.2 Host gesture decoder

The outer UI enables a standard SGR mouse-reporting boundary while attached and restores every mouse mode on exit. A small bounded host-input codec recognizes only:

- SGR wheel reports;
- unmodified PageUp/PageDown;
- the existing detach prefix and bracketed-paste boundaries needed for exact resume buffering.

It is not a VT output parser. All unrecognized keyboard bytes remain byte-exact.

Routing is mode-derived:

- main screen with no child mouse ownership: wheel and unmodified Page keys navigate zterm history;
- child mouse reporting: mouse events are encoded according to authoritative `TerminalMouseMode`/`TerminalMouseEncoding` and forwarded;
- alternate screen or authoritative alternate-scroll: wheel is forwarded/emulated as child cursor scrolling, and Page keys are forwarded unchanged;
- clicks/motion with no child mouse owner are not injected into the shell.

`TerminalModes` gains `alternate_scroll`, populated by the existing safe callback boundary for DECSET/DECRST 1007 and carried through state, snapshot, delta, checkpoint, and protobuf conversion. There are no process-name branches for tmux, Herdr, editors, or pagers.

## 6. Resize/closure race

The observed error is a same-UID command-side write racing attachment closure. The fix stays at the existing local terminal driver boundary:

- a normal command error remains immediate;
- a command-side socket-closure error (`EPIPE`, reset, or equivalent mapped closure) enters one bounded correlation drain instead of terminating the driver immediately;
- the driver prioritizes already-buffered `SessionEnded`, `LeaseLost`, remote typed service error, or transport lifecycle events from the event side;
- if no typed event arrives within the bound, return one normalized daemon/attachment closure diagnostic without embedding the raw OS error;
- no resize/input command is replayed, and no new reconnect loop is introduced.

The CLI therefore cannot let a resize command win merely because its oneshot completed a few microseconds before the authoritative event. Real daemon stop, lease loss, Session end, and driver failure keep their existing categories.

## 7. Compatibility, security, and bounds

- Protobuf additions are additive; new wire kinds are registered in the single `WireKind`/message mapping and compatibility corpus.
- History rows and terminal content remain redacted from `Debug`, logs, status, and errors.
- Status events are local-only and reject remote normal-ALPN use.
- Device aliases retain existing 128-byte/control-character validation; status clipping is presentation, not validation.
- RTT uses selected-path observation only, is rounded/clamped to a wire integer, and never persists.
- One history request may be outstanding per CLI viewport; bridge pending-control limit remains eight.
- History page rows, resume-input bytes, event queues, stdin chunks, snapshot/delta frames, and model history all retain explicit bounds.
- Terminal restoration includes mouse capture, SGR, cursor visibility, outer alternate screen, raw termios, signals, cancellation, panic, and ordinary completion.

## 8. Validation strategy

### Pure/core and protocol

- history page ordering/styles, Unicode width, cursor bounds, monotonic append, capacity eviction, clear/resize epoch changes, main/alternate eligibility, and frame limiting;
- `alternate_scroll` state/snapshot/delta/checkpoint round-trip;
- additive protobuf compatibility, unknown capability behavior, local-only status kind, and malformed/oversize page rejection.

### Daemon/bridge

- local and remote history request correlation, authorization/attachment identity, interleaved live deltas, reconnect failure, pending-control capacity, and no replay;
- selected relay/direct path plus RTT projection, path migration, reconnect clearing, frozen alias, and no address/ID leakage;
- deterministic resize versus buffered SessionEnded/LeaseLost/EOF races proving typed outcome wins and raw `Broken pipe` is absent.

### CLI/PTY

- geometry is physical rows minus one only for remote views, including one-row fallback and repeated resize;
- full-row reverse status rendering, Unicode/narrow columns, child SGR/cursor/clear isolation, and exact terminal restoration;
- wheel/Page routing under main, alternate, mouse, alternate-scroll, tmux, and Herdr modes;
- live/pinned/resume state, page prefetch, gap notice, full resync, and exact bounded key/paste forwarding.

The final manual check uses macOS Ghostty for rapid resize, long shell output scrollback, tmux/Herdr, direct-to-relay path changes where available, and visual verification of the three-field reverse status row.

## 9. Rollback boundary

The work lands in ordered slices. Core/protocol additions are inert until daemon and CLI consumers are connected. If status observation fails, it degrades to `-- | --` without changing attachment transport. If history negotiation is absent, live attachment remains intact. Any failure in history/UI restoration blocks release; it is not hidden by disabling existing typed terminal safety gates.
