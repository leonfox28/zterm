# Research: Renderer-neutral local viewport cache / history window

- Query: 设计最小的、renderer-neutral 的本地 viewport cache / history-window 架构，使桌面 CLI 不再对每个滚轮或 scrollbar drag 事件请求 daemon，并让同一状态机以后可用于 Android；同时划清独立的 wheel multiplier 与 DEC 2026/no-clear 修复边界。
- Scope: internal（当前 core/terminal/proto/daemon/CLI、测试与 Trellis specs）+ external references already pinned by sibling Herdr research
- Date: 2026-09-03

## Findings

### 1. 结论与最小交付边界

推荐新增一个 **stateless history-window** 协议，而不是改变现有 315/316 的语义，也不是把 312/313 pager 当作新的主路径：

- 保留 `TerminalViewportRequest/Frame` kinds 315/316 与 capability bit 19 的现有契约，作为只支持当前连续 viewport 协议的 peer fallback。
- 保留 `TerminalHistoryRequest/Page` kinds 312/313 与 capability bit 17 的现有契约，作为更旧 peer 的 history-only fallback。
- 新增 `TerminalHistoryWindowRequest/Frame` kinds **317/318** 与 `Capabilities::TERMINAL_HISTORY_WINDOW == 1 << 20`。
- 新路径由 client 显式携带 anchor、绝对 target 与预取 margin。Session/daemon 只读投影一个连续 window，不保存或推进新路径的 presentation offset。
- 第一阶段 wire payload 明确命名为 `ansi_rows`，只服务当前桌面 presenter；range/anchor/cache reducer 与 row payload 解耦。Android semantic cells 是以后单独协商的 payload capability/kind，不在本次虚设未定格式。

最小可实现效果是：第一次 cache miss 最多发生一次请求；之后 wheel、Page 和 drag 命中 cache 时只改变 client-local desired offset 并本地重画；只在初次填充、低水位预取、远距离 jump、epoch/size 不兼容或 cache miss 时访问 daemon。它不要求实现 Android、不要求 cell diff，也不增加任何 Session aggregate-memory admission。

### 2. 为什么必须新增 kind/capability

#### 不扩展 315/316

315/316 当前是有状态 action 协议：request 只有相对/绝对 action，Session 的 `ActorAttachment.scroll_metrics` 是隐式 baseline，Frame 必须正好包含 `viewport_rows` 行，offset 归零另走 `Live`。这些语义已经由 core、local IPC、remote bridge 和兼容性测试冻结：

- `crates/core/src/terminal.rs:238-328` 定义 attachment-local metrics/action/full-height Frame/Live/Changed/Gap。
- `crates/terminal/src/model.rs:301-375` 从 daemon-owned previous metrics 解析 action，并只投影一屏。
- `crates/daemon/src/session.rs:2401-2408,3650-3669` 在 `ActorAttachment` 上保存并替换 315/316 baseline。
- `crates/daemon/src/local_ipc.rs:1597-1649` 明确校验 Frame 行数必须等于 `viewport_rows`。
- `proto/zterm/v1/wire.proto:61-62` 与 `crates/proto/tests/compatibility.rs:483-491` 固定 kinds/capability。

将多屏 range 塞进 315/316 会同时改变“谁拥有 offset”“Frame 是否恰好一屏”“offset zero 是否可带 rows”三项既有不变量。旧 peer 只知道 bit 19，无法据此判断对方是否理解新字段；依靠 unknown-field ignore 再从 response 猜能力，会让严格 validator 和 fallback 都变成双义。新 bit/kind 使旧路径完全不变，并允许 window 独立做严格验证。

#### 不复用 312/313 pager

pager 只返回 Alacritty negative-line history rows，方向是 Newest/Older/Newer，且上限 80 行（`crates/core/src/terminal.rs:169-236`、`crates/terminal/src/model.rs:378-455`）。它不能表达 offset 3 所需的 `Line(-3)..Line(R-4)` history+live 混合 viewport，不能自然表达 scrollbar 任意绝对 target，也无法一次返回前后预取区。因此它只适合 mixed-version fallback，不能成为 touch/drag 热路径。

### 3. 建议的 core 与 proto 类型

Transport-neutral core 类型建议为：

```rust
pub const MAX_HISTORY_WINDOW_ROWS: usize = 240;

pub struct TerminalHistoryWindowAnchor {
    pub epoch: Revision,
    pub revision: Revision,
    pub max_offset_from_bottom: u64,
    pub viewport: TerminalSize,
}

pub struct TerminalHistoryWindowQuery {
    pub anchor: TerminalHistoryWindowAnchor,
    pub target_offset_from_bottom: u64,
    pub older_margin_rows: u16,
    pub newer_margin_rows: u16,
}

pub struct TerminalHistoryWindowFrame {
    pub disposition: TerminalViewportDisposition, // Exact | Rebased
    pub anchor: TerminalHistoryWindowAnchor,      // authoritative response anchor
    pub target_offset_from_bottom: u64,            // resolved target O
    pub first_row_from_live_top: i64,
    pub ansi_rows: Vec<Vec<u8>>,
}

pub enum TerminalHistoryWindowResult {
    Frame(TerminalHistoryWindowFrame),
    HistoryChanged { epoch: Revision, revision: Revision },
    HistoryGap { epoch: Revision, revision: Revision },
}
```

单独的 anchor 比复用 `TerminalScrollMetrics` 更清楚：metrics 的 `offset_from_bottom` 是 presentation 状态，而 window anchor 只描述一个坐标空间。它也补上了 columns，使 width reflow 不能被误认为同一 identity。

建议 protobuf shape（field numbers 是新 message 内的建议值，不复用现有 message 字段）：

```proto
message TerminalHistoryWindowAnchor {
  uint64 epoch = 1;
  uint64 revision = 2;
  uint64 max_offset_from_bottom = 3;
  uint32 viewport_rows = 4;
  uint32 viewport_columns = 5;
}

message TerminalHistoryWindowRequest {
  AttachmentId attachment_id = 1;
  TerminalHistoryWindowAnchor anchor = 2;
  uint64 target_offset_from_bottom = 3;
  uint32 older_margin_rows = 4;
  uint32 newer_margin_rows = 5;
}

enum TerminalHistoryWindowOutcome {
  TERMINAL_HISTORY_WINDOW_OUTCOME_UNSPECIFIED = 0;
  TERMINAL_HISTORY_WINDOW_OUTCOME_FRAME = 1;
  TERMINAL_HISTORY_WINDOW_OUTCOME_CHANGED = 2;
  TERMINAL_HISTORY_WINDOW_OUTCOME_GAP = 3;
}

message TerminalHistoryWindowFrame {
  AttachmentId attachment_id = 1;
  TerminalHistoryWindowOutcome outcome = 2;
  TerminalViewportDisposition disposition = 3;
  TerminalHistoryWindowAnchor anchor = 4;
  uint64 target_offset_from_bottom = 5;
  sint64 first_row_from_live_top = 6;
  repeated bytes ansi_rows = 7;
  uint64 current_epoch = 8;
  uint64 current_revision = 9;
}
```

不需要 `Live` outcome：target zero 可用于 silent prefetch，但从 history 真正恢复 live 仍必须使用现有 full-sync handshake，以恢复 cursor、modes 和最新 live screen，而不是把无 cursor/mode 的 row window 当作 live snapshot。

### 4. 行坐标、range 与 slice 的精确定义

对于 response anchor：

- `R = viewport.rows`
- `H = max_offset_from_bottom`
- live viewport 顶行为坐标 `0`
- retained history 是负坐标 `[-H, 0)`
- 当前 live screen 是 `[0, R)`
- 整个可投影区间是 `[-H, R)`，行坐标从上到下递增
- target offset `O` 必须满足 `0 <= O <= H`

目标可见 viewport 为：

```text
visible_start = -O
visible_end   = R - O                 // exclusive
visible      = [visible_start, visible_end)
```

给定 older/newer margins：

```text
window_start = max(-H, visible_start - older_margin_rows)
window_end   = min( R, visible_end   + newer_margin_rows) // exclusive
ansi_rows[i] = encode(Line(window_start + i))
```

Frame 的 `first_row_from_live_top = window_start`，rows 覆盖 `[window_start, window_start + rows.len())`。所有 signed/unsigned 转换和加减必须 checked；产品的 `H <= 2000, R <= 80` 仍不足以成为跳过 wire validation 的理由。

Client 只有在完整可见 slice 被一个 cache window 包含时才可本地 render：

```text
cache_start <= -O
R - O <= cache_start + cache_rows.len()
```

slice indices 为：

```text
slice_start = (-O) - cache_start
slice_end   = slice_start + R
```

必须一次取恰好 `R` 行；不得从旧 window 和新 response 各取一部分。

### 5. Anchor/revision 在各种事件下的契约

#### 普通 append，同 epoch/size，extent 单调增长

Request anchor 合法条件为：非零且受限的 rows/columns、`epoch <= revision`、`target <= anchor.max`、`anchor.revision <= model.revision`。若 current anchor 与 request anchor 的 epoch/size 相同，且 `H_current >= H_anchor`：

```text
growth = H_current - H_anchor
resolved_O = min(H_current, target_O + growth)
disposition = Exact
```

这样 request 在排队/传输期间新增的整行不会使用户查看的内容向下漂。仅 revision 增长、但 history extent 不增长时 `growth = 0`；server 仍从一个当前 model revision 原子投影整个 window。

Client cache 自身是 response revision 上的 immutable snapshot。若 pinned history 期间收到更新的 live metrics，并且 epoch/size 相同且 max 单调增长，保留 cache-coordinate target `O_cache`，对当前 scrollbar 报告：

```text
O_current = O_cache + (H_latest - H_cache)
```

反向把当前绝对 target 映射到旧 cache 时使用 checked subtraction；target 落入 cache response 之后才出现的新行区间时视为 cache miss，不伪造 rows。

#### Resize/reflow、clear、extent decrease、capacity eviction

Epoch 或 rows/columns 不同，或 current extent 小于 request anchor extent时，旧坐标 identity 不再可证明。Server 在当前 main screen 上返回一个完整新 window：

```text
resolved_O = min(request.target_offset_from_bottom, H_current)
disposition = Rebased
```

Client 原子替换整个 window；绝不拼接两个 epoch/size 的 rows。物理 resize 后到达、但 size 与 client 当前 expected size 不同的 response 应丢弃，并为 latest size/target 重新请求。

#### Alternate screen

Live 状态下 alternate screen 没有 main-history metrics，不启用 host history/window。Pinned history 下若 background child 进入 alternate，client 可继续显示当前已完成的 main-history frame，保持 effective presentation screen 为 Main；只在现有 immutable cache 覆盖范围内导航。任何需要 daemon 新 projection 的 miss 得到 `HistoryChanged`，保留最后完整画面，不能合成 alternate rows。普通输入仍走既有 resume/full-sync 并显示当前 authoritative alternate screen。

#### Reconnect、detach、takeover

真正的 transport reconnect、controller takeover、detach 或 fresh attachment 清除 window、desired target、drag、in-flight/latest correlation，并回到 live synchronization；不把 presentation cache 写入 `RemoteResumeCheckpoint`、controller lease、磁盘或 Session model。Background replacement snapshot 不是 reconnect：只要 attachment/stream epoch 没变，pinned immutable window 可以保留，新的不兼容 size/epoch 在下一次 refill 时整窗 rebase。

#### Gap 与 malformed response

Future revision、结构非法 anchor、future/越界 target 返回 content-free `HistoryGap`。Frame receiver 还必须验证：

- anchor 合法且 `current_epoch/current_revision` 与它一致；
- disposition 非 unspecified；
- target 不超过 max；
- `R <= ansi_rows.len() <= 240`；
- window range 位于 `[-H,R)` 且完整包含 `[-O,R-O)`；
- total frame 不超过现有 8 MiB content limit；
- attachment ID、request ID 和 pending response kind 完全匹配。

Changed/Gap 不允许携带 anchor、disposition 或 rows。

### 6. Bounded cache shape 与预取策略

建议把纯状态机放在新的 `crates/core/src/viewport_cache.rs`，类型对 row 泛型化，例如 `ViewportCache<Row>` / `CachedViewportWindow<Row>`。它只负责 checked range math、Live/History/ResumePending 状态、desired/presented offset、cache hit/miss、one-in-flight/latest-target reducer；不得依赖 ANSI、stdout、Tokio、mouse/touch、像素、Alacritty 或 PTY。`zterm-core::terminal` 继续拥有 wire/domain window DTO。

每个 view 只保留：

```text
latest_live_anchor: Option<Anchor>
window: Option<{ anchor, first_row, rows }>
presented_offset / desired_offset
pending_window_request: Option<{ request_id/generation, requested_target }>
latest_queued_target: Option<absolute offset>
presentation state + stale/incompatible marker
```

缓存是一段连续 window，不累计 page 链表。硬界限：

- `MAX_HISTORY_WINDOW_ROWS = 240`，即最大 viewport height 80 的三屏；
- request 必须满足 `older_margin_rows + newer_margin_rows <= 2 * R`；
- response 继续受 8 MiB frame gate，request 受 1 MiB control gate；
- replacement 可短暂同时持有 old displayed window 与一个 decoded response，但 commit 后只留一个；实现应 move rows，避免无意义 clone。

推荐 placement：

- 靠近 live bottom：`older = 2R, newer = 0`
- 中间：`older = R, newer = R`
- 靠近 oldest：`older = 0, newer = 2R`

当目标可见 slice 距任一 cache edge 少于 `ceil(R/2)` 行时，发一个 background recenter/prefetch。若当前已有 request，只更新 latest absolute target，不排事件队列。

Live attach 的可选但有价值的最小预热是：收到一个稳定的 main-screen full snapshot 后静默请求 target 0、`older=2R` 的 window，不改变显示。Live delta 的 ANSI 无法可靠 patch row cache；因此后续 live delta 可把这份预热标为 dirty，而不是每个 delta 都重新请求。第一次 wheel 若 cache 仍与最新 live baseline 兼容则立即显示；否则保持 live 完整画面并请求一次。持续输出下的 idle debounce refresh 可后续调优，不应成为首版 correctness 条件。

这不是 aggregate admission：不得因为多个 Session cache 的估算总和跨过 128 MiB 而拒绝 create/resize。仍保留每个 request/frame/row-count 的安全上限；cache 随 client view drop 释放。

### 7. 复用 one-in-flight/latest coalescing

当前 CLI 已有 `viewport_pending + queued_scroll`（`crates/cli/src/terminal_ui.rs:2236-2244,2459-2467,2571-2621,2624-2653`），local client 也只允许一个 pending viewport request（`crates/daemon/src/local_ipc.rs:1041-1072,1225-1235`）。新路径应保留这个背压模型，但 queued value 统一为 **latest absolute desired offset**，不再累计每个物理 event 的 RPC action：

1. Wheel/drag 更新 reducer 的 bounded desired offset。
2. Cache hit：立即从本地 slice render；必要时只安排一次低水位 prefetch。
3. Cache miss/jump：保持上一个完整 presentation，不画 loading blank；若没有 in-flight，发一个 window request。
4. Pending 期间的新事件只覆盖 `latest_queued_target`。
5. Response 若覆盖 latest desired target，则原子 commit 并 render；若不覆盖，不展示该 stale target，也不替换仍在显示的 usable cache，立即为 latest target 发下一次请求。
6. Drag release 强制提交最终 target；中间 motion 可在 client presentation adapter 以约 16--33 ms cadence 合并，但 cadence/timer 不进入共享 reducer。

Remote bridge 现有 pending-control map 与 epoch-loss Gap 机制也可复用：为 318 增加 response-kind uniqueness/correlation；stream loss 将 pending 318 完成成 content-free Gap 后进入 reconnect，绝不在新 stream replay。

### 8. ANSI desktop 与未来 Android 的分阶段边界

本次只实现：

- renderer-neutral anchor/range/query/reducer；
- daemon 从 authoritative Alacritty model 一次锁内投影连续 rows；
- `ansi_rows` wire payload；
- CLI 将恰好一屏 ANSI rows 与 chrome 组成一次 host presentation。

本次明确不实现：Android project、touch physics、native text renderer、semantic-cell wire encoding、graphics/protocol expansion，或在 CLI 再运行一个 VT parser。

以后 Android 可直接复用 `ViewportCache<Row>` reducer 与 anchor/range contract，让 gesture adapter 把 continuous touch displacement/velocity 转成 local absolute target；网络仍只处理 prefetch/miss。Android row 类型应来自一个单独协商的 semantic-window payload（例如 packed semantic rows/cells），不能把当前 `bytes ansi_rows` 静默解释成 cells。是否扩展 317/318 的新 optional semantic payload，还是分配另一对 kinds，应在 semantic cell schema、style subset、Unicode/combining/wide-cell 编码与 payload benchmark 完成后再决定。

### 9. 与 cache 独立、应先落地的 P0 修复

Local cache 降低 request/repaint 频率，但它不修复每次实际 presentation 内暴露的中间空白。以下是独立 P0，不能等同于 cache：

#### Wheel multiplier

当前三条 host-owned branch 都把每个收到的 SGR wheel report 乘成三行（`crates/cli/src/terminal_ui.rs:798-800,838,2386-2388`）。物理 terminal 一次滚轮动作可能发多个 report，因此用户观察到 3 reports × 3 lines = 9 lines。将 **每个 host-owned report 改为 1 logical line**；PageUp/PageDown 仍是 `max(R-1,1)`。Child mouse 与 alternate-scroll 仍只转发一个 report/sequence，不做 host multiplier。触摸端以后使用像素/字体 cell-height accumulator，不继承桌面 wheel 常量。

#### DEC 2026 与 no-clear-before-content

Herdr sibling research 证明低闪烁路径是完整帧/差分、outer-host DEC synchronized output、no clear-before-draw 和 bounded cadence 的组合。Zterm 当前：

- snapshot/delta/history/chrome 最后只 flush 一次，但没有 outer host DEC 2026（`crates/cli/src/terminal_ui.rs:3106-3197,3406-3416`）；一次 flush 并不阻止 terminal 在收字节过程中显示中间状态。
- history 每行先 `CUP + reset + EL2` 再写内容（`:3419-3443`）。
- terminal delta 也先 EL2 再写新 row（`crates/terminal/src/ansi.rs:29-40`）。
- 发出每个 viewport/history request 前无条件 `render_view_stdout`，会先画 loading/旧 frame，response 后再画一次（`crates/cli/src/terminal_ui.rs:3458-3488`）。

P0 边界应为：

1. 只在最终 CLI -> 用户实际 Ghostty/kitty/其他 outer terminal 的 presentation boundary 包 `CSI ? 2026 h` ... `CSI ? 2026 l`；不要发给 child PTY，也不要让 daemon Alacritty model 解析它。
2. 一个 transaction 内顺序为：begin synchronized output、hide cursor、完整 child/history rows、status/scrollbar chrome、`HOST_INPUT_CAPTURE`、最终 cursor/state、end synchronized output、一次 flush。
3. `RESTORE_TERMINAL_UI` 也显式发 `CSI ? 2026 l`，防止异常/partial write 后 host 留在同步状态。
4. 删除无 display-state 改变的 request-time repaint；cache miss 保留上一个完整画面。
5. 普通 delta/history repaint 改为先写目标内容，再用 `EL` 清理尾部（或等价固定宽度覆盖）；不要在可见内容写入前 EL2。首个没有 baseline 的 full snapshot 仍可 ED2。
6. 用 byte-exact tests 覆盖 full-width、wrapped、wide/combining row，确保 tail clear 不落到下一行；在不支持 DEC 2026 的 host 上 no-clear 顺序仍是必要 fallback。

### 10. Minimum affected source and test files

#### Product source

- `crates/core/src/terminal.rs` — anchor/query/frame/result DTO 与 240-row bound。
- `crates/core/src/domain.rs` — capability bit 20。
- `crates/core/src/lib.rs` + new `crates/core/src/viewport_cache.rs` — export renderer-neutral generic reducer。
- `crates/terminal/src/model.rs` — read-only `history_window` reconciliation/range projection；保留 315/316 方法。
- `proto/zterm/v1/terminal.proto`, `proto/zterm/v1/wire.proto` — messages/outcome 与 kinds 317/318。
- `crates/proto/src/lib.rs` — kind classification/decoding, redacted Debug, conversions and frame validation helpers。
- `crates/daemon/src/terminal_driver.rs` — one-lock read-only window operation。
- `crates/daemon/src/operations.rs` — local view command/event conversion。
- `crates/daemon/src/session.rs` — controller/sync-fenced stateless window command；不写 `ActorAttachment.scroll_metrics`。
- `crates/daemon/src/session_wire.rs` — authenticated service dispatch。
- `crates/daemon/src/local_ipc.rs` — one-pending request, correlation, structural/range/size validation。
- `crates/daemon/src/remote_attachment.rs` — capability gate, forwarding, pending response and epoch-loss Gap。
- `crates/daemon/src/connection_broker.rs`, `crates/daemon/src/service.rs` — advertise/retain bit 20 where bit 19 is currently advertised。
- `crates/cli/src/terminal_ui.rs` — reducer adapter/cache slicing, absolute latest-target coalescing, prefetch/miss policy, wheel 1-line constant, no request-time blank repaint, DEC 2026 transaction/cleanup。
- `crates/terminal/src/ansi.rs` — independent no-clear-before-content delta repair。`projection.rs` already exposes the crate-private row projection needed by model and need not change for ANSI v1。

#### Minimum tests

- `crates/core/src/viewport_cache.rs` unit tests — exact slice math, edge thresholds, same-epoch growth mapping, size/epoch incompatibility, hit/miss, latest-wins, release final target。
- `crates/terminal/src/model.rs` unit tests — target 0 and mixed history/live window, both edge clips, append pinning, resize/reflow, clear/decrease, eviction, alternate and no mutation。
- `crates/proto/src/lib.rs`, `crates/proto/tests/compatibility.rs` — 317/318, bit 20, control/content limits, redacted Debug, old kinds frozen。
- touched daemon module tests in `terminal_driver.rs`, `session.rs`, `session_wire.rs`, `local_ipc.rs`, `remote_attachment.rs` — authorization/sync fence, stateless isolation, correlation, one pending, malformed bounds, capability ladder, lost-stream Gap。
- `crates/cli/src/terminal_ui.rs` unit/byte tests — one report/one line, cache hit creates no command, miss keeps completed frame, low-water one request, drag latest/release, exact DEC 2026/no-clear/chrome/capture/end/flush order。
- `crates/daemon/tests/local_session_ipc.rs` — real local PTY: first fill then many wheel/drag targets without one request per event; nested mouse ownership unchanged。
- `crates/daemon/tests/terminal_recovery.rs` or existing remote-attachment recovery fixture — pending window is completed once as Gap, cache clears, no replay after reconnect。

### 11. Related specs to update after implementation

- `.trellis/spec/backend/core-wire-domain.md` — new DTO, kind/capability, strict window validation and explicit `ansi_rows` encoding。
- `.trellis/spec/backend/terminal-model.md` — contiguous range formula, stateless projection, append/rebase/alternate rules, 240-row cap。
- `.trellis/spec/backend/terminal-driver.md` — one-lock read-only window call and no driver/client offset ownership。
- `.trellis/spec/backend/session-service.md` — controller fence and new path does not persist attachment scroll baseline。
- `.trellis/spec/backend/local-daemon-ipc.md` — cache/prefetch/latest-wins/reconnect/presentation contracts and one-line wheel rule。
- `.trellis/spec/guides/cross-layer-thinking-guide.md` — input -> local cache -> miss/prefetch -> transport -> atomic presentation lifecycle。
- `.trellis/spec/guides/cross-platform-thinking-guide.md` — macOS/Linux host acceptance and Android dependency/payload isolation。

Frontend spec files are currently placeholders (`.trellis/spec/frontend/index.md:7-35`) and there is no Android source tree; do not write speculative Android component conventions in this task. Record only the shared reducer/payload boundary now.

### Files Found

- `crates/core/src/terminal.rs` — current history pager, scroll metrics/action and exactly-one-screen viewport result.
- `crates/core/src/domain.rs` — capability registry and 80x240/2,000-row product limits.
- `crates/terminal/src/model.rs` — authoritative Alacritty-backed projection, same-epoch append adjustment, pager and conservative epoch advancement.
- `crates/terminal/src/projection.rs` — renderer-neutral projected row/cell boundary used by the allowlisted encoder.
- `crates/terminal/src/ansi.rs` — full/delta/history ANSI encoders; current delta clear-before-content sequence.
- `proto/zterm/v1/terminal.proto`, `proto/zterm/v1/wire.proto` — existing history/viewport messages and kinds through 316.
- `crates/proto/src/lib.rs`, `crates/proto/tests/compatibility.rs` — 1/8 MiB gates, content classification, conversion, frozen kinds/capabilities.
- `crates/daemon/src/operations.rs`, `terminal_driver.rs`, `session.rs`, `session_wire.rs` — model operation through attachment/session service.
- `crates/daemon/src/local_ipc.rs`, `remote_attachment.rs` — one-in-flight request correlation, validation, remote capability/failure behavior.
- `crates/cli/src/terminal_ui.rs` — current per-event semantic request, latest coalescing, scrollbar, renderer and host mouse/presentation lifecycle.
- `.trellis/tasks/09-02-migrate-alacritty-terminal/research/herdr-flicker-rendering.md` — pinned Herdr render/cadence/DEC 2026 evidence.
- `.trellis/tasks/09-02-migrate-alacritty-terminal/research/scroll-viewport-integration.md` — prior 315/316 design and cross-layer failure analysis.
- `.trellis/tasks/09-02-migrate-alacritty-terminal/research/nested-tui-scroll-routing.md` — mode-driven single-owner input contract for Herdr/Pi/tmux nesting.

### External References

- Product engine version is official `alacritty_terminal 0.26.0`; Zterm already hides its `Grid`/`Line` types behind `zterm-terminal` and must keep doing so.
- Herdr evidence is pinned at commit `cc88b3b8e5bb9f7d9f23ed6ae85a52fd7b5b9ed6`: [drag throttle/latest target](https://github.com/herdrdev/herdr/blob/cc88b3b8e5bb9f7d9f23ed6ae85a52fd7b5b9ed6/src/client/shell/mouse.rs#L37-L137), [DEC 2026 full/diff presentation](https://github.com/herdrdev/herdr/blob/cc88b3b8e5bb9f7d9f23ed6ae85a52fd7b5b9ed6/src/protocol/render_ansi.rs#L651-L715), and [same-buffer terminal/scrollbar composition](https://github.com/herdrdev/herdr/blob/cc88b3b8e5bb9f7d9f23ed6ae85a52fd7b5b9ed6/src/ui/panes.rs#L341-L366).
- No new external dependency or third-party wrapper is required by this architecture.

## Caveats / Not Found

- Current `TerminalModel::refresh_history_epoch_after_ingest` treats every nonempty ingest while history is exactly at capacity as possible eviction (`crates/terminal/src/model.rs:464-475`). This is correctness-conservative but can cause frequent Rebased windows during sustained output after 2,000 rows. The cache remains safe because it never mixes generations, but absolute drag/refill may request more often. Exact eviction detection is a separate optimization and should not be guessed into this MVP.
- ANSI live deltas cannot update a row-addressable cache without another parser. Therefore a live prefetch becomes dirty after later live output; the MVP must preserve correctness and accept one first-gesture miss rather than add a second VT engine to CLI. An idle refresh policy can be measured later.
- A 240-row count bound does not replace the 8 MiB encoded-frame limit; unusually large styled/combining rows must still fail closed. Conversely, the frame limit must not be converted into a cross-Session admission estimate.
- DEC 2026 only gives atomic presentation on outer terminals that implement synchronized output. Correct write ordering/no-clear-before-content and one flush remain required for other terminals.
- This research did not modify product/spec code, run performance benchmarks, or validate macOS/Linux local/direct/relay behavior. Those remain implementation/check/release evidence, and Android remains intentionally out of scope.
