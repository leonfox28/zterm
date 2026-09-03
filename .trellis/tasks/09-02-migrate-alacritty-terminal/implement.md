# Implementation Plan

> 本文只在用户批准本任务最终计划后执行。批准前不得运行 `task.py start`，不得修改产品
> 代码。本计划完全取代旧 Ghostty wrapper/FFI 方案。
> Phases 0–8 retain completed history. The direct-cutover revisions in Phases 9–15 supersede every
> earlier fixed-wire/ANSI/fallback statement.

## Guardrails

- 只使用 Alacritty 官方 crates.io `alacritty_terminal = "=0.26.0"`，并关闭 default
  features；不使用 fork、Git revision、社区 wrapper、Ghostty、Zig 或 C FFI。
- 所有 Zterm crate 继续继承 `unsafe_code = "forbid"`；不得增加 unsafe island、raw binding、
  `unsafe impl Send/Sync` 或绕过 workspace lint。
- `portable-pty` 仍是唯一 PTY/process owner；一个 Session 仍对应一个 PTY、一个 root child、
  一个 authoritative model。
- 协调升级 product wire major 与 normal/pair ALPN，删除 presentation compatibility；保留
  attachment/session 生命周期和 no-drop PTY drain。任何 Alacritty type 都不得穿过
  `zterm-terminal` 私有边界。
- production binary 不同时运行两个 parser，不提供 runtime fallback。最终 tests 也不保留
  `vt100` 作为 oracle。
- 本任务不新增或运行 throughput、latency、CPU、RSS、build-time、binary-size 对比 benchmark，
  也不形成性能结论。取消128 MiB aggregate memory admission；count/dimension及不可信输入驱动
  buffer/content的cap/overflow tests仍属于正确性验证，必须保留。
- 保留用户在父任务及工作树中的既有修改；实现提交不得回退或顺手整理无关文件。

## Delivery Shape

这是一个跨 crate、但必须按顺序完成的原子迁移。当前不拆 child task，避免出现两个可启动
engine、临时 public API 或部分 cutover 被误认为可发布状态。每个 phase 有显式 rollback
point；最终只有单次 source cutover，没有 feature flag。

## Phase 0 — Freeze the Zterm Contract

### Step 0.1 — Record the starting state

- [x] 记录实施开始时的 commit、dirty files、Rust/Cargo versions 和当前 dependency tree。
- [x] 确认父任务 `.trellis/tasks/08-20-cross-platform-relay-terminal-mvp/task.json` 的既有
  用户修改，不得覆盖。
- [x] 建立 `research/migration-differences.md`，预设四类：must preserve、approved visual
  normalization、security hardening、blocker。

### Step 0.2 — Freeze semantic fixtures

- [x] 在改动前运行当前功能测试，不运行 benchmark/resource-gate；保存 terminal corpus、
  snapshot/delta、history、driver、session、CLI 和 real-PTY 测试结果。
- [x] 将现有行为整理成 Zterm-owned fixture，比较 cell/style/cursor/screen/modes、exact
  DA/DSR/CPR、side-event classifications、history results 和 security sentinels，而不是比较
  `vt100` 私有类型。
- [x] 增补 whole、one-byte、fixed-size 和 deterministic-random chunking corpus；fixture 在
  新引擎通过后独立存在，随后删除旧 parser oracle。
- [x] 明确归一化规则：unwritten default blank 与 explicit default-styled space 视觉等价；
  styled blank、wide head/tail 和 bounded combining text 仍必须保留。

Rollback point: Phase 0 只增加测试/研究证据，不改变 runtime path。

## Phase 1 — Establish the Host-Only Dependency Boundary

### Step 1.1 — Pin the official engine

- [x] root workspace 增加精确 `alacritty_terminal = { version = "=0.26.0",
  default-features = false }` 和 `zterm-terminal` path dependency/member，刷新 `Cargo.lock`。
- [x] 核对 lockfile source/checksum、Apache-2.0 许可、advisory 和 duplicate report；任何新增
  deny exception必须最小化并说明原因。
- [x] 扩展 `tests/source-policy.sh`，拒绝 Ghostty/wrapper/fork/Git source、Zterm-owned unsafe、
  direct second `vte` dependency 和重新引入 `vt100`。
- [x] 增加 dependency-tree fixture：`vte` 只能位于锁定的 Alacritty path；版本升级不能靠
  宽松 semver 自动发生。

### Step 1.2 — Create `zterm-terminal`

- [x] 新增 `crates/terminal`（package `zterm-terminal`），继承 workspace package metadata和
  lints，建立`model`、`engine`、`ingress`、`projection`和`ansi` modules。
- [x] `zterm-core::terminal` 只保留 transport-neutral DTO、side events、snapshot/delta/history
  values、screen metadata constants、wire limiter 和 redacted Debug。
- [x] model/checkpoint/error ownership移动到 `zterm-terminal`；其 public signature只
  使用 core-owned types，Alacritty `Term/Grid/Cell/Event/Processor/TermMode` 全部保持 private。
- [x] daemon直接依赖core + terminal；proto保持core-only。CLI继续作为依赖daemon的host binary，
  但不直接依赖terminal，且其UI只使用core/proto-owned values。禁止core反向re-export host model。
- [x] 明确禁止产品代码调用 `alacritty_terminal::tty`、`event_loop` 或 spawning API。

### Step 1.3 — Prove graph isolation early

- [x] `cargo tree -p zterm-core` 和 `cargo tree -p zterm-proto` 不含 `zterm-terminal`、
  `alacritty_terminal` 或 `vte`。
- [x] CLI的完整transitive graph因其daemon依赖而预期包含engine；静态检查改为CLI没有direct
  engine dependency/upstream type import，daemon只通过`zterm-terminal`获得engine。
- [x] 在 `just ci-windows` 增加 `zterm-terminal --lib`，使 shared boundary在 Windows hosted CI
  显式编译/测试。

Rollback point: 这一阶段失败时删除未接线的新 crate/dependency，旧 runtime仍保持唯一。

## Phase 2 — Implement the Engine, Policy, and Resource Bounds

### Step 2.1 — Wrap `Term` and `Processor`

- [x] 用 private `EngineSize: Dimensions` checked转换 rows/columns；用 safe Rust创建并持有
  `Term<BoundedEventSink>` 与 Alacritty re-exported `vte::ansi::Processor`。
- [x] `Config` 设置 requested bounded scrollback、`osc52 = Disabled`、
  `kitty_keyboard = false`；不启用 renderer、selection、tty/event loop。
- [x] `BoundedEventSink` 只映射允许的无内容事件。它不得把 `Event::PtyWrite`、clipboard、
  color/title closure、upstream Debug 或任意 payload直接写回 PTY/日志。
- [x] model继续由现有 ordered model thread串行调用，不增加 actor、writer thread 或 custom
  `Send`/`Sync`。

### Step 2.2 — Implement `TerminalIngressPolicy`

- [x] 实现跨 chunk streaming framer：Ground/incremental UTF-8、ESC、CSI、OSC、DCS/APC/PM/SOS、
  cancellation、ST/BEL termination 和 discard-until-terminator。
- [x] 固定并测试：CSI/ESC 256 bytes、control string 1,024 bytes、reply output
  64 KiB/update、32 side events/update、title/icon source 256 bytes。
- [x] ordinary printable/grid/mode sequence才转给 Alacritty；overlong/unknown sequence只产生
  不含内容的 bounded classification，部分 payload不得重新成为 printable text。
- [x] policy在精确流位置实现 primary DA `CSI ?1;2c`、DSR `CSI 0n`、standard CPR 和 private
  CPR；其他 upstream query reply一律不透传。
- [x] 映射 BEL audible bell、`ESC g` visual bell、`CSI 8;rows;cols t` resize request、OSC 0/2
  title、OSC 1 icon、OSC 52 rejection；OSC 8/other OSC/DCS/APC/PM/SOS全部有界吞掉。
- [x] 拒绝 DEC synchronized-update 2026 和 Kitty keyboard controls；policy自己跟踪 legacy
  DECSET/DECRST 9，其他受支持 modes由 Alacritty维护。
- [x] 在交给 engine 前删除 hyperlink、underline-color和其他当前 wire无法表达的持久属性；
  不让 secret URI/clipboard payload进入 grid、reply、state、wire或 Debug/log capture。

### Step 2.3 — Bound cell extras

- [x] 新建 safe inline `InlineCellText`，初始 `MAX_CELL_TEXT_BYTES = 22`，只在完整 UTF-8 scalar
  可放入时追加。
- [x] 对 zero-width scalar在 engine ingest前查询目标 cell与预算：per-cell cap和全 session
  64 KiB retained combining payload任一将超限即丢弃，并产生去重的 bounded classification。
- [x] 全 session最多允许 4,096 个含 dynamic combining storage的 cell；main/alternate分别记账。
- [x] 使用保守增量记账；只在接近阈值、screen transition或resize时做有界 grid recount，回收
  已被覆盖/evict的额度。recount发现超限时用 public safe cell API重建超限 extra，不使用 unsafe。
- [x] adversarial tests覆盖单 cell combining flood、多 cell flood、scrollback eviction、screen
  switch、resize、hyperlink/underline-color flood和 malformed UTF-8。

### Step 2.4 — Remove aggregate memory admission and retain safety caps

- [x] 删除`TerminalResourceProjection`、`TerminalModel::project_resources/resource_projection`、
  `ResourceProjectionOverflow`和`ResourceLimits::aggregate_cell_projection_bytes`，不增加新的
  byte-estimate/high-water replacement。
- [x] session create/resize删除aggregate reserve/commit/rollback bookkeeping；仍先验证live-session
  count、viewport dimensions、scrollback policy和revision overflow，再触碰native PTY/model。
- [x] 保留8 live sessions、240x80 viewport、2,000 history rows，以及ingress/reply/event/wire/
  combining caps；取消memory admission不得导致PTY-controlled heap无界。
- [x] 单元测试覆盖零/最小/最大尺寸、checked `rows * columns` overflow、安全caps、resize失败
  顺序，以及Session关闭后model/checkpoints被drop；不计算或断言128/256 MiB。

Rollback point: Phase 2 保持新 engine未接入production reader；任何安全/资源契约失败都回到
isolated crate修正。

## Phase 3 — Build Zterm-Owned Projection and Wire Encoding

### Step 3.1 — Project the supported semantic surface

- [x] 从 active `Grid<Cell>`在 display offset zero构建 private `ProjectedScreen`：size、screen、
  rows/wrap、fixed cells、cursor、modes。
- [x] 颜色映射 Indexed/RGB/named 0..15/default；样式只映射 bold/dim/italic/any underline/
  inverse，明确忽略 strike/hidden/underline-color/hyperlink。
- [x] 映射 wide head/spacer和bounded zero-width text；cursor coordinates checked/clamped。
- [x] mode映射 application cursor/keypad、bracketed paste、focus、alternate scroll、mouse click/
  drag/motion、UTF-8/SGR encoding，加上 policy-owned X10 bit。

### Step 3.2 — Implement the allowlisted ANSI encoder

- [x] encoder只允许 printable UTF-8、current SGR subset、CUP、ED2、EL2、home、cursor show/hide、
  current input modes和Zterm screen metadata selector。
- [x] full snapshot先发 MAIN selector；active alternate时紧接 ALT selector；随后baseline reset、
  clear/home、rows、cursor/style/visibility/modes。
- [x] trailing visually empty default cells可裁剪；styled blank必须输出；wide continuation不重复。
- [x] test-only vocabulary validator拒绝OSC/DCS/APC/PM/SOS、arbitrary private modes、nested screen
  selectors、upstream formatter bytes和任何 secret sentinel。

### Step 3.3 — Implement checkpoint and delta

- [x] `TerminalCheckpoint`只持有format version、revision、size、active screen和一个 projected
  viewport；不持有engine、inactive screen或history，Debug不可泄漏cell contents。
- [x] compatible delta以 `CUP + SGR reset + EL2 + canonical row`重绘changed rows，最后恢复
  cursor/template/modes。
- [x] future revision、size/screen/format mismatch或delta bytes >= full bytes返回Resync；新revision
  但无visible change允许empty delta。
- [x] retained-capacity test更新为 `rows * columns`；lifecycle test证明controller + pending
  takeover稳态最多保留两个checkpoint，但不把它们换算成aggregate memory admission。

### Step 3.4 — Implement snapshot and history

- [x] snapshot继续输出 `recent_history_ansi` 再输出 `screen_ansi`；core 8 MiB limiter只删最旧
  complete history rows，不截screen。
- [x] main history用negative `Line`读取oldest-to-newest，绝不调用 `scroll_display` 或改变live
  state/revision/checkpoint。
- [x] alternate返回Changed；append-below-capacity保epoch；clear/decrease/resize/capacity eviction/
  identity ambiguity推进epoch并按现有contract返回Changed/Gap。
- [x] tests覆盖page bounds、80-row cap、append、eviction、clear、resize/reflow、alternate round-trip。

Rollback point: 新model通过全部direct tests后才允许daemon import切换。

## Phase 4 — Cut Over Driver, Session, and Hosted PTY Profile

### Step 4.1 — Switch daemon ownership

- [x] daemon从 `zterm-terminal`导入model/checkpoint/error，从core导入wire/domain types。
- [x] 保留现有ordered model thread、8 × 8 KiB no-drop queue、model mutex、latest-only revision
  watch、PtyIo mutex和独立 child interrupt；不引入第二条writer path。
- [x] query replies在其对应ingest完成后通过同一个PtyIo有序写回；reply-cap/write failure按现有
  terminal-fatal路径唤醒waiters。
- [x] resize仍为size/revision/session-limit preflight → native PTY resize → model resize → publish
  revision；expected failure不允许native/model尺寸分叉，不做memory preflight。

### Step 4.2 — Preserve session and attachment behavior

- [x] create failure、root exit、explicit close、daemon stop、reader EOF、reaper和drop都恰好释放
  child/PTY/model/checkpoints一次。
- [x] zero attachments持续排水；slow/detached client不产生revision backlog；reconnect/resync、
  controller lease和pending takeover维持当前语义。
- [x] local/remote session paths只传Zterm-owned snapshot/delta/history，不暴露engine类型或
  lifecycle。

### Step 4.3 — Virtualize child terminal identity

- [x] product login shell无条件设置`TERM=xterm-256color`和`COLORTERM=truecolor`；低层fixture
  若显式提供env仍遵守fixture contract。
- [x] real-PTY tests从parent `TERM=ghostty`、`xterm-kitty`、tmux-like和unset环境启动daemon，
  child观察到同一hosted capability profile和exact query replies。
- [x] CLI继续根据显式 `TerminalModes`管理outer mouse/focus/paste/key modes；外层终端只是
  Zterm ANSI消费者，不与daemon `Term`共享对象或PTY。

### Step 4.4 — Remove obsolete measurement owners

- [x] 删除现有`crates/core/benches/terminal_state.rs`、对应Cargo bench target和
  `tests/foundation/resource-gate.sh`；更新引用它们的spec/docs。
- [x] 不用空脚本、伪benchmark或普通test duration替代它们。未来如需profiling，另立任务并
  重新定义问题和workload。

Rollback point: daemon cutover可整体source-revert；不加入runtime feature switch。

## Phase 5 — Remove the Old Engine and Close Release Gates

### Step 5.1 — Complete compatibility/security evidence

- [x] 新engine通过冻结corpus的whole/one-byte/fixed/random chunking，并逐项完成
  `research/migration-differences.md`。
- [x] exact DA/DSR/CPR、screen/mode/cursor/style/wide/combining、side effects、snapshot/delta/
  history和all output-surface sentinel tests全绿。
- [x] driver/session/CLI/real-PTY tests覆盖non-reading child、query flood、detach/reconnect、
  recovery、resize、natural/explicit close、slow attachment和session isolation。
- [x] 未分类的must-preserve差异视为blocker；不能以“Alacritty行为不同”直接改写用户契约。

### Step 5.2 — Delete vt100 and temporary migration code

- [x] 删除workspace/core `vt100` dependency、old parser/model/helpers/callbacks和临时oracle。
- [x] `cargo tree --workspace`无`vt100`；`vte`只有Alacritty锁定dependency path；无Ghostty、
  wrapper、FFI或fork。
- [x] 删除任何temporary dual path/feature flag；保留Zterm-owned semantic/security fixtures。

### Step 5.3 — Cross-platform and release checks

- [ ] hosted macOS/Linux/Windows compile、Clippy和tests包含`zterm-terminal`；Windows只记录shared
  compile/test evidence，不扩大runtime承诺。
- [ ] 四个正式macOS/Linux native release-readiness jobs继续验证architecture、macOS 13、
  glibc 2.28、SBOM/license/source/dynamic dependencies；artifact不新增terminal dylib。
- [x] Android/iOS acceptance只检查core/proto这两个mobile-facing library graph隔离；host CLI
  不作为mobile library，不宣称mobile local engine、PTY或pixel renderer支持。

### Step 5.4 — Update executable project knowledge

- [x] 更新terminal-model、terminal-driver、pty-lifecycle、session-service、core-wire-domain、
  distribution-lifecycle和cross-platform specs，使其描述新crate、policy、安全caps和已取消的
  aggregate memory admission。
- [x] 文档明确`alacritty_terminal`是parser/state/grid而不是pixel renderer；outer terminal不是
  library nesting。
- [x] 验收报告明确：“未运行性能/RSS benchmark，不作迁移后性能保证”。
- [x] 执行最终API/dependency/content-redaction/user-change review，然后运行`trellis-check`。

Implementation self-review and independent `trellis-check` are complete. The
hosted matrix/release-readiness runs remain external acceptance evidence.

## Phase 6 — Continuous Scroll Viewport and CLI Scrollbar Follow-up

> Phase 0–5 record the already implemented and released engine migration. Every item below is new,
> remains unchecked during brainstorm, and requires the fresh post-summary user approval recorded at
> the end of this document before product-code work starts.

### Step 6.0 — Freeze the regressions and compatibility baseline

- [x] Record the follow-up start commit/dirty files and preserve all unrelated user/task changes.
- [x] Add a byte-level failing fixture proving an initial snapshot and an ordinary delta currently
  leave outer `1003/1006` disabled; cover resync, child mouse enable/disable and guard cleanup before
  changing renderer behavior.
- [x] Add a controller fixture proving the first live wheel-up currently ignores its three-line
  amount and cannot render `3 history + rows-3 live`; preserve existing PageUp, resume-input bound,
  history Changed/Gap and mixed-version page behavior as baseline evidence.
- [x] Freeze current wire kind/field/capability values through 314 / `HISTORY_PAGING` before adding
  anything. Do not reuse or renumber an existing value.

Rollback point: tests/research only; released runtime path is unchanged.

### Step 6.1 — Add renderer-neutral scroll domain and model projection

- [x] In `zterm-core::terminal`, add bounded/redacted `TerminalScrollMetrics`, signed
  `ScrollByLines`, absolute `ScrollToOffset`, viewport disposition/result/frame types and validation
  helpers. Define zero as live, positive signed delta as older/up and negative as newer/down.
- [x] Add live main-screen metrics to `TerminalSnapshot`/`TerminalDelta`; absent/invalid-on-alternate
  remains explicit rather than inventing a history extent.
- [x] In `zterm-terminal`, expose a non-mutating extent and full-height viewport projection. For
  offset `N`, read `Line(-N)..Line(rows-N-1)` under one model lock through the existing bounded row
  projector/encoder; never call `scroll_display` or modify `display_offset`.
- [x] Unit-test offsets 0/1/3/max/over-max, history shorter/longer than viewport, exact
  `3 history + live` composition, wide/styled/Unicode rows, alternate screen, resize/clear/eviction,
  unchanged revision/checkpoint/state and isolation between two callers.
- [x] Add per-attachment scroll baseline to `ActorAttachment`, not `TerminalModel` or
  `RemoteResumeCheckpoint`. Implement same-epoch output pinning, checked signed/absolute clamps,
  explicit `Rebased`, Live at offset zero and reset-on-detach/reconnect semantics.

Rollback point: new domain/model APIs remain unconnected to CLI input and do not affect live render.

### Step 6.2 — Carry semantic viewport through local and remote transport

- [x] Add proto v1 metrics/action/outcome/request/frame messages, optional live metrics on snapshot/
  delta, next unused terminal kinds after 314 and `Capabilities::TERMINAL_VIEWPORT` at the next free
  bit 19 (`AGENT_EVENTS` already owns bit 18).
  Preserve every existing field/kind/capability number.
- [x] Classify request as control and frame as content; validate attachment ID, action oneof,
  signed range, metrics invariants, row count/height, 1 MiB/8 MiB frame limits and content-redacted
  Debug output.
- [x] Thread the request/result through proto conversion, local IPC, typed operations driver,
  Session actor/driver, session wire, remote attachment bridge and connection broker advertisement.
  Keep at most one outstanding viewport request per attachment.
- [x] Coalesce pending wheel deltas into one bounded signed amount and pending drag into the latest
  absolute target. Live/resume supersedes queued scrolling; terminal outcome ordering still wins
  command-stream closure races.
- [x] Preserve `TerminalHistoryRequest/Page` unchanged as the fallback. A capability-less old peer
  must never receive a viewport kind; new↔old tests prove optional fields are ignored, legacy paging
  remains usable and no unsupported feature is falsely advertised.

Rollback point: new wire is capability-gated; old history paging and snapshot/delta remain valid.

### Step 6.3 — Fix capture and replace the CLI page jump with semantic actions

- [x] Refactor `TerminalRenderer` transactions so snapshot, applied delta, resync replacement and
  viewport-frame rendering finish with `HOST_INPUT_CAPTURE` followed by one flush. Remove the narrow
  `child_transition_disables_host_capture` dependency without altering authoritative child modes.
- [x] Replace the continuous-path `HistoryViewport` page/offset navigation with the returned semantic
  metrics/frame and new request actions. Retain a small explicit legacy pager only for peers lacking
  `TERMINAL_VIEWPORT`.
- [x] Implement ownership order: pinned Zterm history; visible Zterm gutter; live child mouse mode;
  live alternate+alternate-scroll; otherwise host main history. Never inspect process names, TERM,
  tmux state or screen text.
- [x] Keep one report on child mouse branches and change alternate-scroll from three cursor sequences
  to one. v0.1.11 kept the host CLI wheel constant at three lines and PageUp/PageDown at rows minus
  one; Phase 7.1 intentionally supersedes only that host-owned wheel constant.
- [x] Preserve `ResumePending`: ordinary input is bounded, latest live replacement and any geometry
  sync are flushed first, and the retained bytes are then sent exactly once. Background child mode
  changes cannot steal a pinned viewport.
- [x] Extend pseudo-terminal tests for Ghostty/kitty-compatible SGR reports, Default/UTF-8/SGR child
  encoding, Herdr/Pi-style modes, child mode exit, burst coalescing, delta gaps, cleanup, signals and
  panic restoration.

Rollback point: capture hotfix and semantic routing can be source-reverted without wire persistence.

### Step 6.4 — Implement stable gutter, scrollbar, and mode-driven geometry

- [x] Introduce a pure `ChromeLayout` from physical size, product limits, remote status-row presence
  and effective presentation screen. Use main assumption before initial snapshot, reserve one column
  only when usable width is greater than four, and expose exact child/gutter hit rectangles.
- [x] Make initial/physical/mode-driven resize use the same layout. Main history appearance must never
  resize; live Main→Alternate changes `N-1→N` once and Alternate→Main changes `N→N-1` once.
  `ResizeCoalescer::last_submitted` must suppress the resize-produced same-screen replacement.
- [x] Add pure overflow-safe scrollbar geometry: proportional min-one-row thumb, oldest/top and
  live/bottom mapping, track-click absolute target, drag grab offset and clamped pointer mapping.
- [x] Render a cleared reserved gutter with no history; render `▕` track and `▐` thumb only with valid
  metrics. Save/restore cursor/style, exclude the remote status row, clear stale chrome before
  alternate reclaim and repaint chrome after every relevant live/history/status transaction.
- [x] Intercept gutter wheel/press/drag/release without clamping it into child coordinates. Ignore
  events beyond usable layout, end drag on release/capture loss and ensure main-screen child mouse
  still receives events from all `N-1` child columns exactly once.
- [x] Unit/integration-test widths 1/4/5/max/over-max, heights with/without remote status, zero/one/max
  history, all thumb positions, clicks above/below/on thumb, drag bursts, initial alternate attach,
  pinned-background alternate, no geometry loop and eventual child redraw at the advertised width.

Rollback point: disabling/removing CLI chrome restores full-width main geometry; semantic viewport
wire remains independently valid for Android or a later UI.

### Step 6.5 — Close platform, compatibility, and knowledge gates

- [x] Run targeted core/terminal/proto/daemon/CLI tests, then fmt, Clippy, workspace tests, docs,
  cargo-deny, source policy and `just check`; do not run or add performance/RSS benchmarks.
- [ ] Add real PTY smoke scripts/fixtures for shell scroll, Herdr/Pi-style fullscreen enter/exit,
  mouse ownership, resize, detach/reconnect and local/direct/relay flows without requiring a named
  outer terminal for correctness.
- [ ] Execute and record macOS local/direct/relay smoke on macOS and Linux local/direct/relay smoke on
  Linux. Hosted Linux evidence must come from its CI/runner; local macOS cannot stand in for it.
- [x] Update terminal-model, terminal-driver, session-service, local-daemon-IPC, core-wire-domain,
  CLI/UI and cross-platform specs with executable scroll ownership, capability and geometry contracts.
- [ ] Run `trellis-check`; verify only intended task/product files changed, all follow-up acceptance
  items have direct evidence, and the report states that no throughput/latency/CPU/RSS benchmark was
  run and Android is still the next task.

Follow-up rollback is a source revert. Proto additions are additive and no viewport state is
persisted, so there is no schema/data migration to undo.

### Phase 6 verification evidence

- Follow-up baseline: released `main` commit
  `bce1d57d8bd91b5c0e58bcdf422d899cedcd7fac`; every dirty path was inventoried before product
  edits and the task-owned prior changes were preserved.
- Independent `trellis-check` reviewed the complete core -> terminal model -> Session -> proto ->
  local/remote bridge -> CLI path and fixed validation, old-peer, gutter-drag, background-resync,
  duplex input/history/viewport race, and epoch-loss correlation findings.
- Final `just check` passed after the additional caller-isolation and resize/eviction/clear model
  regressions. It includes source/version/dependency policy, format, workspace all-target Clippy with
  warnings denied, secret scans, all workspace tests/docs, cargo-deny, and relay static/upstream
  checks. The affected full suites include daemon `198/198`, CLI library `47 passed / 3 isolated
  helpers ignored`, and terminal model `19/19`.
- The real macOS local outer-PTY + daemon + production CLI scroll fixture passed five consecutive
  runs: a single SGR wheel report repainted the target exactly three rows above a 24-row live
  viewport, then detach and termios restoration completed. The automatic-resync pinned-baseline PTY
  regression also passed five consecutive runs.
- Pure/local/remote protocol tests cover Herdr/Pi-style terminal modes, one-report ownership,
  alternate entry/exit, capability-less fallback, one-outstanding/coalescing, background sync,
  reconnect, resize, redaction, and frame limits. The broader real-binary/direct/relay smoke item
  remains unchecked: actual Herdr/Pi fullscreen runs, macOS direct/relay, and all Linux
  local/direct/relay runtime paths are still external acceptance work.
- No throughput, latency, CPU, RSS, or candidate-comparison benchmark was run. Android remains the
  next task only after the outstanding native transport matrix is recorded.

## Validation Matrix

实现时先跑针对性门，再跑workspace门；命令按实际test target补充，但不得用不存在的target
占位成功。

```sh
sh tests/source-policy.sh
sh tests/workspace-version.sh
cargo +1.98.0 fmt --all -- --check
cargo +1.98.0 clippy --workspace --all-targets --all-features -- -D warnings
cargo +1.98.0 test -p zterm-terminal --all-features
cargo +1.98.0 test -p zterm-core --all-features
cargo +1.98.0 test -p zterm-proto --all-features
cargo +1.98.0 test -p zterm-daemon --lib --all-features
cargo +1.98.0 test -p zterm-daemon --test terminal_blackbox
cargo +1.98.0 test -p zterm-daemon --test terminal_drain
cargo +1.98.0 test -p zterm-daemon --test terminal_recovery
cargo +1.98.0 test -p zterm-daemon --test attachment_resync
cargo +1.98.0 test -p zterm-daemon --test session_limits
cargo +1.98.0 test -p zterm-platform --all-features
cargo +1.98.0 test -p zterm-cli --all-features
cargo +1.98.0 test --workspace --all-features
cargo +1.98.0 doc --workspace --no-deps
cargo +1.98.0 deny check
just ci-policy
just check
```

Dependency-isolation evidence:

```sh
cargo tree -p zterm-core
cargo tree -p zterm-proto
cargo tree -p zterm-daemon
cargo tree --workspace -i vte
```

`tests/source-policy.sh`负责断言workspace不存在`vt100`、core/proto不含engine dependency、CLI
没有direct engine dependency；不能把预期包含daemon传递依赖的CLI完整tree误判为失败。

Hosted-only evidence comes from the existing GitHub Actions matrix and release-readiness jobs; local
execution must report those as hosted-only, not silently mark them passed。

Explicitly forbidden in this task:

```text
cargo bench ...
sh tests/foundation/resource-gate.sh
vt100-vs-Alacritty / Ghostty-vs-Alacritty throughput, latency, CPU or RSS comparisons
```

## Final Completion Gate

- [ ] All PRD acceptance criteria have direct evidence.
- [x] `research/migration-differences.md` has no unresolved must-preserve item.
- [x] Product path contains exactly one terminal engine and one PTY per Session.
- [x] Zterm-owned code remains globally unsafe-forbidden.
- [x] No aggregate terminal-memory admission or stale128/256 MiB gate remains; count/dimension和
  untrusted-input safety caps仍生效。
- [x] No performance test was run or performance claim recorded.
- [x] Relevant specs/docs and task evidence are updated.
- [x] `trellis-check` passes and user-owned unrelated changes remain intact.

The user-owned modification to the parent task's `task.json` remains present
and was not edited by this implementation or the independent check. The
unchecked items are exactly the hosted-only evidence boundaries above.

## Scroll Follow-up Completion Gate

- [x] Host capture survives initial/full/delta/resync/history render and is removed on every exit.
- [x] Exact continuous viewport, signed/absolute actions, epoch rebase and per-attachment isolation
  pass model/session/local/remote tests.
- [x] One-report ownership and Herdr/Pi-style nested paths pass without application-specific logic.
- [x] Stable main gutter, interactive scrollbar and main/alternate single-resize behavior pass.
- [x] New capability and mixed-version legacy fallback pass bounds/correlation/redaction tests.
- [ ] Real macOS and Linux local/direct/relay smoke evidence is recorded before Android work starts.
- [x] Relevant specs/task evidence are current; `trellis-check` passes; no performance test was run.

## Pre-Start Approval

- [x] 用户已明确批准本 PRD、design和implementation plan（2026-09-02）。
- [x] 批准后已运行：

```sh
python3 .trellis/scripts/task.py start .trellis/tasks/09-02-migrate-alacritty-terminal
```

### Scroll follow-up authorization

- [x] 已向用户展示本次 scroll/gutter/wire 最终计划摘要。
- [x] 用户在该最终摘要之后再次明确批准开始修改产品代码（2026-09-02）。

## Phase 7 — Atomic Presentation and Client-owned Viewport Cache

### Step 7.0 — Freeze the reported failures and compatibility surface

- [x] Record `origin/main` / v0.1.11 as the implementation baseline and preserve the task-owned
  research additions plus unrelated worktree changes.
- [x] Add failing CLI byte-stream tests for one host report currently requesting three lines,
  request-time blank repaint, `EL2` before history content, missing outer DEC 2026 and drag release
  losing a throttled final target.
- [x] Freeze kinds through 316 and capabilities through bit 19. Add mixed-version fixtures proving
  315/316 and 312/313 remain unchanged before allocating 317/318 and bit 20.

Rollback point: tests/task research only; v0.1.11 behavior remains the product path.

### Step 7.1 — Fix desktop input and presentation first

- [x] Change only the host-owned CLI wheel amount to one line per complete SGR report. Preserve
  child mouse and alternate-scroll one-event semantics and PageUp/PageDown overlap.
- [x] Introduce one buffered host-presentation helper: DEC 2026 begin, complete content/chrome/capture
  and final cursor state, DEC 2026 end, one `write_all`, one `flush`. Add sync-end to every guard
  cleanup path.
- [x] Remove request-time unchanged/loading/returning repaints. Keep the last complete presentation
  until a replacement is ready; write history content before clearing its remaining line tail.
- [x] Add 33 ms drag pacing, target-change suppression and release-final delivery without replacing
  existing one-in-flight/latest-target coalescing.

Rollback point: presentation changes can revert independently; protocol still uses existing paths.

### Step 7.2 — Add the read-only window model and generic cache reducer

- [x] Add core anchor/request/frame/result values, `MAX_HISTORY_WINDOW_ROWS = 240`, structural
  validation and content-redacted Debug. Add a generic row cache/reducer free of ANSI, async,
  clocks and platform UI.
- [x] Add a terminal-model read-only projection for `[start,end)` using the approved live-top
  coordinate formula. Reuse the existing row projector/allowlisted encoder; do not call
  `scroll_display` or modify model/checkpoint/revision.
- [x] Implement same-epoch append anchoring, complete Rebased replacement, alternate Changed and
  invalid/future Gap. Test 0/1/mid/oldest targets, margins/caps, Unicode/wide/style and unchanged
  state across independent callers.
- [x] Unit-test cache hit rendering decisions, checked slice math, edge prefetch, one pending/latest
  target, stale response handling, append translation and all invalidation paths.

Rollback point: domain/model/cache code is not yet reachable over wire.

### Step 7.3 — Carry the additive window through local and remote paths

- [x] Allocate bit 20 and kinds 317/318 in core/proto/wire without changing any existing number.
  Add protobuf conversion/validation for anchor, signed coordinate, margins, row count, 1/8 MiB
  bounds and redacted Debug.
- [x] Thread the pure read through TerminalDriver, Session authorization/sync fence, session wire,
  local attachment driver, remote bridge and operations writer. It must not update
  `ActorAttachment.scroll` or remote resume state.
- [x] Enforce one outstanding window request and latest queued target. On stream loss, return one
  correlated content-free Gap, clear pending state, then reconnect.
- [x] Negotiate bit20 -> bit19 -> pager fallback. New code never sends 317/318 to a capability-less
  peer; old clients retain exact v0.1.11 behavior.

Rollback point: bit20 gates the complete path; legacy semantic viewport and paging remain usable.

### Step 7.4 — Make desktop the first cached-window client

- [x] Integrate the generic cache reducer into `ViewportController`. Opportunity-prefetch a bounded
  live window after activation; render wheel/Page/drag locally whenever the requested full slice is
  cached.
- [x] Request a window only for initial miss, edge low-water, absolute jump, stale identity or
  response that does not cover latest target. Preserve the currently displayed complete frame while
  pending and never render an intermediate target merely because its response arrived first.
- [x] Keep local offset/scrollbar metrics authoritative for presentation while cached. Translate
  coordinates on same-epoch append and invalidate on resize/reflow/extent decrease/live reset/true
  reconnect/takeover. Keep legacy 315/316 and pager controllers isolated behind capability fallback.
- [x] Add deterministic CLI/controller and local/remote fixtures proving cache hits emit no network
  request, prefetch is bounded, thumb release reaches the final target and nested TUI ownership is
  unchanged.

Rollback point: disable bit20/client selection to return to v0.1.11 semantic viewport behavior.

### Step 7.5 — Close quality, platform and knowledge gates

- [x] Run targeted core/terminal/proto/daemon/CLI tests, format, workspace Clippy/tests/docs,
  cargo-deny, source policy and one final `just check`; do not run performance/RSS benchmarks.
- [x] Dispatch an independent `trellis-check` across model -> Session -> proto -> local/remote -> CLI
  and fix every verified contract failure before delivery.
- [x] Update terminal-model, terminal-driver, core-wire-domain, session-service, local-daemon-IPC,
  transport-auth, and cross-platform specs with the final implemented window/cache/presentation
  contracts.
- [x] Record macOS local evidence available from this host and leave Linux plus real direct/relay
  evidence explicitly hosted/external when it cannot be executed here. Do not claim Android UI or
  semantic-cell wire completion.

Implementation evidence (2026-09-03, macOS host):

- Baseline: `origin/main` and `HEAD` were
  `be46c8f736148dab38a4854362b551bafe8a52fd` (v0.1.11) on
  `fix/local-viewport-cache`; task research/planning changes were preserved.
- `cargo +1.98.0 test -p zterm-core viewport_cache --all-features`: 10 passed.
- `cargo +1.98.0 test -p zterm-terminal history_window --all-features`: 3 passed.
- `cargo +1.98.0 test -p zterm-proto --all-features`: 26 passed.
- `cargo +1.98.0 test -p zterm-daemon --all-features --quiet`: passed (201 library tests plus
  every emitted integration group; platform/explicit-only skips remained reported as skips).
- `cargo +1.98.0 test -p zterm-cli terminal_ui --all-features -- --nocapture`: 40 passed,
  3 isolated helper tests ignored; `cargo +1.98.0 test -p zterm-cli --test daemon_autospawn
  --all-features -- --nocapture` exited successfully and exercised the real-PTY one-line wheel path.
- `cargo +1.98.0 fmt --all -- --check`, touched-package all-target/all-feature Clippy with
  `-D warnings`, and `sh tests/source-policy.sh` passed.
- Linux cross-UID, real direct/relay and the explicit terminal black-box gate were not available in
  this macOS implementation pass. Android UI, semantic-cell wire, performance and RSS claims remain
  out of scope. Full workspace tests/docs, cargo-deny, the single final `just check`, independent
  `trellis-check`, and final spec updates were completed in the closing review documented below.

Independent review evidence (2026-09-03, macOS host):

- The reviewer fixed request-bound response validation, stale-anchor monotonicity, complete-frame
  retention during resize/resume/content-free outcomes, resize-triggered window refill, styled row
  tail clearing, stream-loss/unsupported fallback distinction, and the complete bit20 -> bit19 ->
  pager fallback. Focused cache/model/local/remote/CLI/ANSI regressions passed.
- `cargo +1.98.0 fmt --all -- --check`, `cargo +1.98.0 check --workspace --all-targets
  --all-features`, and `cargo +1.98.0 clippy --workspace --all-targets --all-features -- -D warnings`
  passed on the final reviewed tree. `sh tests/source-policy.sh` and `git diff --check` passed; no
  product Rust `unsafe` was added.
- The one authorized `just check` invocation reached workspace tests and exited 101 because an
  existing `scroll_viewport_projects_full_rows_and_clamps_at_both_ends` fixture had accidentally
  replaced the asserted `one` row with the new styled-wide Unicode fixture. The reviewer restored
  that legacy fixture while retaining focused Unicode/wide/style coverage on `history_window` and,
  per the instruction to run the authoritative command exactly once, did not invoke `just check`
  again.
- After that fixture correction,
  `cargo +1.98.0 test -p zterm-terminal --lib --all-features` passed 23/23 and
  `cargo +1.98.0 test --workspace --all-features` passed. Workspace documentation, both cargo-deny
  manifests, relay-probe format/Clippy, relay shell syntax/publication/upstream checks, and the
  Docker-capable relay static check all passed when run as the remaining individual gate
  components.
- Hosted gaps remain explicit: Linux cross-UID was skipped on macOS, real Iroh loopback is Linux-CI
  owned, the terminal black-box gate is explicit-only, and real Linux plus local/direct/relay smoke
  was not run in this review. Android UI/semantic cells and performance/RSS work remain out of
  scope; no benchmark was run. Final `.trellis/spec/` synchronization was completed by the main
  session before the feature commits.

Hosted PR follow-up evidence (2026-09-03):

- PR #7's first matrix run passed dependency policy, portable policy, relay bundle, Linux x64,
  Windows shared-boundary, and macOS arm64. Linux arm64 exposed a `session_wire` fixture boundary:
  visible `history-11` did not prove its trailing CRLF had reached the model before the first local
  history page. A child marker emitted after the final CRLF now fences the comparison; the exact
  test passed 10/10 and daemon library tests passed 202/202 locally.
- The same run's macOS Intel job exposed a pre-existing identity-reset fixture boundary: a just-
  dropped Darwin Unix listener could briefly complete a connection and return EOF/`cancelled`
  before the intended shared deadline wait. The fixture now waits under an independent one-second
  bound for connect refusal before starting the unchanged 40-millisecond production deadline and
  still requires exactly `deadline_exceeded`. The exact test passed 50/50, daemon library tests
  passed 202/202, and daemon Clippy/fmt/diff checks passed locally.
- Neither correction changes product logic or weakens an assertion. Linux arm64 and macOS Intel
  remain pending until the pushed follow-up commit passes the hosted matrix.

### Phase 7 completion gate

- [x] One host report moves one row; child-owned paths still receive one event.
- [x] Every desktop repaint is complete, non-blank, synchronized when supported and cleanup-safe.
- [x] One bounded pure-read window and generic cache reducer pass owner-level tests.
- [x] Cache-covered desktop navigation performs no network request; miss/prefetch remains one-in-flight
  and latest-target correct.
- [x] 317/318 + bit20 and bit20 -> bit19 -> pager compatibility pass all local/remote tests.
- [x] Product Rust remains unsafe-forbidden, one Session remains one PTY/model, and no benchmark or
  Android-completion claim is introduced.

### Phase 7 authorization

- [x] 用户已批准 desktop/mobile 共用 client-owned viewport cache，并明确要求按该方案实施
  （2026-09-03）。

## Post-release follow-up: live-bottom gutter flicker (2026-09-03)

### 1. Root Cause Category

- **Category**: D — Test Coverage Gap, with B — Cross-Layer Contract.
- **Specific cause**: returning from history enters `ResumePending`. The old
  implementation returned no scroll metrics in that state, so the
  `SyncRequired` chrome transaction and replacement snapshot each cleared the
  gutter before `Active` redrew the live thumb. Legacy viewport coalescing also
  stored an intermediate response that was never painted, conflating received
  state with presentation authority.

### 2. Why the first anti-flicker fix was incomplete

1. It correctly kept history content unchanged while a replacement was pending,
   but tested `write_history` rather than every chrome frame in the event loop.
2. DEC 2026 made each individual write atomic, but could not make three separate
   transactions atomic as a group; the blank gutter transaction remained real.
3. The state model distinguished desired and cached offsets, but the legacy
   fallback still treated the latest received frame as if it had been rendered.

### 3. Prevention Mechanisms

| Priority | Mechanism | Specific action | Status |
| --- | --- | --- | --- |
| P0 | Architecture | Preserve validated last-painted metrics in `ResumePending`; switch to replacement live metrics only after snapshot observation | DONE |
| P0 | Architecture | Do not replace legacy presentation state with a coalesced intermediate frame that immediately schedules another request | DONE |
| P0 | Tests | Assert every frame in `History -> ResumePending -> SyncRequired -> Snapshot -> Active`, including nonblank old and offset-zero final chrome | DONE |
| P1 | Invalidation | Clear mismatched metrics on resize and clear both live/presented identity on true reconnect | DONE |
| P1 | Knowledge | Record received/desired/presented separation in local IPC spec and cross-layer checklist | DONE |

### 4. Systematic Expansion

- **Similar issues**: Android must not promote a fetched window to visible
  scroll state until the UI frame containing it commits; latest-wins transport
  state and presentation state remain distinct.
- **Design improvement**: model last-painted chrome explicitly at transition
  boundaries instead of deriving it from the newest response object.
- **Process improvement**: rendering regressions must enumerate intermediate
  output transactions, not merely compare the initial and final reducers.

### 5. Verification

- `cargo +1.98.0 test -p zterm-cli --all-features`: 56 passed, 3 ignored.
- `cargo +1.98.0 clippy -p zterm-cli --all-targets --all-features -- -D warnings`: passed.
- `cargo +1.98.0 fmt --all -- --check`, `sh tests/source-policy.sh`, and
  `git diff --check`: passed.
- Independent `trellis-check` found and fixed reconnect identity and unpainted
  legacy-coalescing gaps, then passed the complete CLI suite, format, Clippy,
  type-check, and source policy.
- Real Ghostty observation remains the user-owned acceptance step; no claim is
  made from byte-level tests that every outer-terminal implementation paints
  synchronized updates identically.

## Post-smoke follow-up: remote status-row flash and return-live pause (2026-09-03)

### 1. Root Cause Category

- **Category**: D — Test Coverage Gap, with B — Cross-Layer Contract and E —
  Implicit Assumption.
- **Specific cause**: the CLI treated every non-`Active` transport state as if
  the last observed connection path/RTT were invalid. Return-to-live is an
  in-epoch visual synchronization, however, so its
  `Synchronizing -> Snapshot -> Active` sequence painted `-- | --`, restored
  direct/relay + RTT, and then issued an unchanged Active repaint. That leaked
  internal coordination into the status row and added avoidable writes/flushes.
- **Discriminating evidence**: the prior probabilities were 55% intermediate
  transition frames, 25% a stale 16 ms cadence deadline, and 20% host-terminal
  composition. A deterministic writer trace reproduced the path as
  `direct -> -- -> direct` with extra transactions while showing the cadence
  timer was cancelled. This raises the intermediate-frame cause above 95%; a
  real Ghostty smoke remains necessary only for host-compositor confirmation.

### 2. Why the prior fixes were incomplete

1. The first return-live regression asserted content and scrollbar geometry,
   but did not assert the remote status row in every stdout transaction.
2. DEC 2026 makes one transaction atomic; it cannot hide a false status frame
   or an unchanged Active repaint emitted as separate transactions.
3. Transport synchronization and connection-observation validity were modeled
   implicitly through one `Active` comparison, so an in-epoch snapshot repair
   looked indistinguishable from a true reconnect to the status renderer.

### 3. Prevention Mechanisms

| Priority | Mechanism | Specific action | Status |
| --- | --- | --- | --- |
| P0 | Architecture | Preserve validated path/RTT through same-epoch synchronization; clear it only at a true reconnect epoch boundary | DONE |
| P0 | Presentation | Retain the complete history frame until one replacement Snapshot atomically paints live content, bottom thumb, and status | DONE |
| P0 | State transition | Let post-Snapshot Active complete input/resize state without repainting an unchanged visual result | DONE |
| P0 | Tests | Enumerate actual DEC-2026 transactions and assert content, gutter, status, writes, and flushes across the full resume sequence | DONE |
| P1 | Knowledge | Record the separate transport, connection-observation, and presentation authorities in code-spec and review checklist | DONE |

### 4. Systematic Expansion

- **Similar issues**: any future title/tab/badge, Android connection indicator,
  cursor overlay, or selection chrome can flicker if derived directly from a
  transient synchronization state instead of its own validity epoch.
- **Design improvement**: transition handlers decide separately whether state
  work, observation invalidation, and visual presentation are required. A
  visually complete replacement Snapshot is presentation authority even though
  the later Active event still owns input fencing and resize completion.
- **Process improvement**: transition tests must inspect the sequence and count
  of externally written frames, including every Zterm-owned chrome region,
  rather than checking only reducer endpoints or one component.

### 5. Knowledge Capture

- [x] Update `.trellis/spec/backend/local-daemon-ipc.md` with executable
  preservation/reset rules, matrix cases, tests, and wrong/correct examples.
- [x] Update the cross-layer terminal-presentation checklist to include
  connection-observation authority, the status row, and transaction count.
- [x] Add same-epoch direct/relay preservation and true-reconnect isolation
  regressions at the real composed-frame writer boundary.
- [x] Verify this product repository has no `src/templates/markdown/spec/`
  mirror, so there is no generated spec counterpart to synchronize.
- [ ] Confirm the reviewed debug binary in real Ghostty by returning to live
  repeatedly with wheel and scrollbar drag; this is user-owned acceptance.

### 6. Verification

- The regression first failed on the old `remote | -- | --` Synchronizing
  frame. The fixed sequence contains exactly the retained history transaction
  and one authoritative live Snapshot transaction; both retain direct/relay +
  RTT, DEC 2026 boundaries, host capture, one write, and one flush.
- Independent `trellis-check` found no production-code defect. It expanded the
  tests to Relay + RTT and to a true reconnect's full replacement Snapshot, so
  stale Direct/RTT cannot hide behind a chrome-only test.
- Focused tests and the complete `zterm-cli` suite passed with 64 library tests
  and 3 intentional isolated-process helpers ignored; main/integration tests,
  workspace all-target/all-feature check and Clippy with `-D warnings`, format,
  source policy, and `git diff --check` passed.
- The repository-level `just check` passed after the production, regression,
  task, and spec changes, including all workspace tests/docs, policy and secret
  checks, cargo-deny, and Relay static/upstream verification. The reviewed
  `target/debug/zterm` was then rebuilt with all CLI features.
- Real Ghostty/macOS/Linux composition remains the explicit smoke boundary;
  byte-level tests prove what Zterm emits, not when each host compositor paints.

## Phase 8 — Herdr-style Host Viewport Presentation Cadence (Planning)

### 1. Isolate timing state

- [x] Add a small desktop-CLI-owned pacer with a 16 ms minimum interval, one dirty flag, and at most
  one pending deadline. Keep it free of protocol, Alacritty, row, and terminal-writer ownership.
- [x] Provide deterministic mark-presented, mark-dirty, due, deadline, and cancel transitions that
  can be tested with supplied `Instant` values; do not introduce an always-running interval.

### 2. Separate state/request work from presentation

- [x] Refactor host-owned viewport effect handling so wheel/gutter/Page navigation updates desired
  state immediately and history-window request/prefetch effects remain immediate, while eligible
  cached rendering can be marked dirty instead of flushed per report.
- [x] Reduce every `HostInputEvent` from one stdin delivery before making the batch's presentation
  decision. Preserve one-row-per-wheel-report and exact checked final offset.
- [x] Leave child-owned mouse and alternate-scroll byte forwarding completely outside the pacer.
  Preserve the existing 33 ms scrollbar-drag request pacing and force the final release position to
  be presented when complete.

### 3. Drive and invalidate the pending frame

- [x] Add one guarded viewport-deadline branch to the active terminal `tokio::select!`; on expiry,
  present only the latest complete host-owned history slice in the existing DEC 2026 transaction.
- [x] Treat an immediate render containing the current viewport as satisfying pending work.
- [x] Cancel pending presentation before return-live/resume, authoritative snapshot/resync,
  resize/reflow, true reconnect, replacement transport state, detach, and cleanup. Do not cancel a
  compatible pinned-history frame solely because a background live delta advanced.

### 4. Regression tests

- [x] Prove three same-direction reports delivered together move exactly three rows and emit one
  history/chrome transaction; prove multiple deliveries inside 16 ms emit at most the latest due
  frame and retain the final offset.
- [x] Cover a burst crossing the cadence boundary, reverse direction, top/live clamp, cache miss with
  immediate request, and a deadline firing after the view has been invalidated.
- [x] Prove child-owned Herdr/PiAgent-style mouse modes still forward every report immediately and
  receive no host history repaint; prove final thumb release is not stranded.
- [x] Re-run existing resume/gutter/DEC-2026 byte-order, reconnect, resize, local-cache, fallback, and
  cleanup tests so cadence cannot reintroduce the fixed blank-gutter frame.

### 5. Validation and user smoke

- [x] Run focused CLI tests, full `zterm-cli` tests, format, all-target/all-feature CLI Clippy with
  `-D warnings`, source policy, and `git diff --check`.
- [x] Run the repository-prescribed broader gate once required by the active task, followed by an
  independent `trellis-check` review and spec synchronization.
- [x] Build the reviewed debug binary at `target/debug/zterm`.
- [ ] Complete real Ghostty verification: slow wheel, rapid wheel, trackpad, reverse direction,
  scrollbar drag/release, return-live, and nested Herdr/PiAgent fullscreen mode.

### Phase 8 completion gate

- [x] Desktop host-owned cached viewport presentation is event-driven and no faster than the chosen
  16 ms cadence, without a global PTY-output scheduler.
- [x] Every input report still affects the exact final offset; only intermediate presentation is
  coalesced, and requests remain immediate/latest-target correct.
- [x] Child-owned input, one-Session/one-PTY ownership, wire compatibility, Alacritty model, Android
  scope, and product-code unsafe prohibition are unchanged.
- [x] No stale timed frame can repaint after an authoritative state transition or cleanup.

### Phase 8 authorization

- [x] Final planning summary presented and subsequently approved by the user on 2026-09-03.

### Phase 8 implementation and review evidence (2026-09-03, macOS arm64)

- The CLI now reduces every host-owned event from one stdin delivery before a presentation
  decision, uses one event-driven/non-sliding 16 ms deadline, and keeps requests immediate. Normal
  PTY deltas and child-owned mouse/alternate-scroll do not enter the pacer.
- Independent review found that `ViewportCache` can advance its locally presentable offset before a
  paced stdout frame commits. The fix adds a CLI-only last-successfully-presented metrics baseline,
  advances it only after the complete outer transaction succeeds, and retains that baseline across
  cache miss/resume instead of preserving an unseen target. Review also fixed a sliding cold
  deadline and added same-epoch background-growth coverage.
- `cargo +1.98.0 test -p zterm-cli --all-features` passed 62 library tests with 3 intentional
  isolated-process helpers ignored; main and integration tests passed. CLI all-target/all-feature
  check and Clippy with `-D warnings`, format, source policy, and `git diff --check` passed.
- The single repository-level `just check` passed: source/version/dependency/release policy,
  workspace all-target/all-feature Clippy, secret scans, all workspace tests and docs, cargo-deny,
  and Relay static/upstream checks. Expected macOS skips remain Linux cross-UID, explicit-only
  terminal blackbox, and Linux-only real Iroh loopback; no performance/RSS benchmark was run.
- Real Ghostty perceived-cadence smoke remains the user-owned acceptance step. A burst that crosses
  a 16 ms boundary may intentionally produce two normally spaced frames; final offset must remain
  exact and no back-to-back per-report flush may return.

## Post-smoke follow-up: nested Herdr scrollbar erased (2026-09-03)

### 1. Root Cause Category

- **Category**: B — Cross-Layer Contract, with D — Test Coverage Gap and E —
  Implicit Assumption.
- **Specific cause**: a Main-to-Alternate transition transferred Zterm's final
  gutter column back to the child, but `write_scrollbar` still appended spaces
  for the former gutter after the authoritative child snapshot. Herdr rendered
  its own pane scrollbar in that rightmost column, so the later Zterm chrome
  bytes erased it inside the same otherwise-atomic DEC-2026 transaction.
- **Discriminating evidence**: the initial hypotheses were 55% stale host
  chrome, 25% terminal-model/rightmost-cell projection, and 20% Herdr mode or
  configuration. Herdr's pinned source confirmed it owns the outer alternate
  screen and draws pane chrome in the pane's final column. An exact-byte red
  regression then showed child `▐` followed by Zterm CUP-plus-space writes to
  the same column, raising stale chrome above 95%. Projection tests confirmed
  that the model and ANSI encoder preserve the rightmost child cell.

### 2. Why Earlier Scrollbar Tests Missed It

1. Layout tests proved that Alternate had no Zterm gutter and received the full
   width, but did not assert the final writer of the reclaimed physical column.
2. Clearing an old gutter is correct for Main-to-Main relocation, so that rule
   was incorrectly generalized to a transition where the region changed owner.
3. DEC 2026 prevents partial visual presentation of one transaction; it cannot
   repair a transaction whose final byte order itself overwrites child content.
4. The original stale state followed the most recently requested layout rather
   than the last successfully painted layout, leaving consecutive resize and
   failed-write cases underspecified.

### 3. Prevention Mechanisms

| Priority | Mechanism | Specific action | Status |
| --- | --- | --- | --- |
| P0 | Ownership | When Main transfers the gutter to Alternate or width `<=4`, never clear that reclaimed child column after child output | DONE |
| P0 | Presentation | Track the last successfully presented gutter independently from desired layout and commit it only after write plus flush succeed | DONE |
| P0 | Ordering | For Main-to-Main relocation, clear the committed old column first and draw the final desired gutter last to repair right-margin clamp | DONE |
| P0 | Tests | Assert exact bytes/write/flush for child-column preservation, grow/shrink, multiple layouts, removal, and write/flush failure retry | DONE |
| P1 | Knowledge | Record region ownership transfer and last-writer rules in design, local IPC spec, and cross-layer checklist | DONE |

### 4. Systematic Expansion

- **Similar issues**: tmux, PiAgent, editors, and any future nested TUI may draw
  a border or scrollbar in the final alternate-screen column. Process-name
  handling would hide this bug for only known applications, so routing remains
  mode/layout based.
- **Design improvement**: every status row, gutter, selection, title, overlay,
  and future Android chrome region needs an explicit owner per state plus an
  explicit final writer during ownership transfer.
- **Process improvement**: transition regressions must start from a committed
  prior frame and compare the full composed byte transaction, including output
  failures and retries, rather than infer presentation from desired layout.

### 5. Knowledge Capture

- [x] Update `.trellis/spec/backend/local-daemon-ipc.md` with gutter ownership,
  presentation authority, validation matrix, tests, and wrong/correct examples.
- [x] Update the cross-layer checklist with region ownership, final-writer, and
  failed-transaction rules.
- [x] Correct this task's design wording: the child snapshot or physical resize
  replaces/clips a reclaimed gutter; Zterm does not clear it afterward.
- [x] Verify that this product repository has no
  `src/templates/markdown/spec/` mirror to synchronize.
- [ ] Confirm the reviewed debug binary in real Ghostty with a Herdr pane that
  has enough retained output for its own scrollbar to be visible.

### 6. Verification

- The regression first failed with an exact transaction containing Herdr's
  rightmost `▐` followed by Zterm spaces at that same column. The corrected
  Alternate transaction contains child content, host capture, and DEC-2026 end
  with no reclaimed-column cleanup.
- Independent review found and fixed consecutive-layout and failed-transaction
  authority gaps. `zterm-cli` passed 69 library tests with 3 intentional
  isolated-process helpers ignored; CLI main/integration tests, check, Clippy
  with `-D warnings`, format, source policy, and `git diff --check` passed.
- The repository-level `just check` passed, including all workspace tests and
  docs, policy/secret checks, cargo-deny, and Relay verification.
- The pinned Herdr 0.8.2 terminal-model black box passed alternate-screen,
  resize, detached progress, resync, bounded pending work, and cleanup. It does
  not replace the remaining real Ghostty compositor smoke.
- The reviewed all-features debug binary was rebuilt at `target/debug/zterm`
  after the final repository gate.

## Phase 9 — Freeze the Presentation Failure Class

> The user confirmed one complete semantic-presentation migration with one release boundary. These
> ordered phases are internal checkpoints, not independently releasable feature slices. Product code
> remains frozen until the reconverged summary is followed by a new explicit implementation approval.
> The later direct-cutover decision supersedes every mixed-version/fallback item below: implementation
> resumes only after this plan removes the ANSI family, obsolete history protocols, and negotiation.

### Step 9.0 — Preserve baseline and classification

- [x] Record implementation-start commit, branch, dirty task/spec files, Rust/Cargo versions, current
  advertised capabilities, wire numbers through 318, and the focused terminal/daemon/CLI baseline.
- [x] Preserve all user and prior follow-up changes. Do not rewrite completed Phase 6–8 evidence or
  collapse unrelated worktree state into this migration.
- [x] Record the two-level diagnosis in the implementation log: split physical ownership is an
  architecture/boundary defect; unconditional full-width `EL0` is a local legacy-adapter defect.
- [x] Enumerate every active stdout/physical-mode writer and every current presentation baseline.
  The list becomes the final no-bypass audit; discovering another owner updates design before code.

Implementation-start evidence (2026-09-03, macOS arm64):

- Baseline commit `941877281135697b114692d4a643b593cbf632e5` on
  `fix/live-bottom-scrollbar-flicker`; Rust/Cargo are both 1.98.0. The dirty tree contains only the
  already reviewed task/spec changes listed by `git status --short`; no product source was dirty.
- The stable registry ends at history-window kinds 317/318 and capability bit 20. Production has no
  semantic-cell kind or advertised capability at this baseline.
- Focused baseline passed: core 43 unit + 10 pairing-vector tests; proto 10 unit + 16 compatibility
  tests; terminal 23 unit + 10 security + 5 corpus + 5 snapshot/delta tests; daemon 202 library
  tests; CLI terminal UI 58 passed with 3 intentional isolated helpers ignored.
- Root-cause classification is deliberately two-level. Multiple independent active writers and
  separately committed content/chrome baselines make the lost-cell/flicker family an
  **architecture / boundary defect**. The legacy row adapter's unconditional `EL0` after a
  full-width row is independently a **local implementation defect** against the existing
  replacement contract.
- The active raw-mode physical writers are: `TerminalGuard` entry/restoration; `TerminalRenderer`
  snapshot/delta output; `StatusRenderer`; history, scrollbar, and transport-state render helpers,
  all wrapped individually by `present_atomic`. Normal-mode completion/error output in `main.rs`
  remains outside the active presenter lifecycle. Current committed presentation authorities are
  split across `TerminalRenderer` revision/screen/modes, `StatusRenderer::previous_row`,
  `ViewportController` presented gutter and scroll metrics, and the viewport pacer's last-presented
  instant. This list is the Phase 14 no-bypass audit baseline.

### Step 9.1 — Add application-neutral failing contracts

- [x] Add a generic alternate-screen nested-TUI fixture that paints a styled final-column cell and
  updates it after a child-owned wheel report. Prove the Alacritty projection retains the cell while
  the current final physical transaction loses it in a strict pending-wrap/inclusive-erase oracle.
- [x] Add sibling regressions for status/gutter ownership transfer, history-to-live composition, and
  failed write/flush baseline. These should describe complete desired output, not Herdr glyph bytes.
- [x] Freeze wire-major-1 rejection fixtures and the wire-major-2 semantic kind registry before
  deleting old payloads. Old nodes must fail at readiness/ALPN/Hello rather than attachment rendering.
- [x] Add source/test guardrails rejecting application/process/title/theme/terminal-brand detection,
  a second CLI terminal parser, new unsafe code, and semantic messages containing ANSI payloads.

Rollback point: task evidence and red/contract tests only; resume implementation only after the
direct-cutover planning gate is approved.

## Phase 10 — Add the Semantic Core and Wire Contract

### Step 10.1 — Implement core semantic values

- [x] Add `TerminalSurfaceRow`, `TerminalSurface`, `TerminalSurfaceSnapshot`,
  `TerminalSurfaceRowPatch`, `TerminalSurfaceDelta`, and semantic history-window result values in
  `zterm-core`, reusing existing cell/style/cursor/mode/size/anchor values.
- [x] Centralize structural validation: exact rectangular shape, dimension limits, advancing
  revisions, sorted unique patch rows, cursor/metrics consistency, 22-byte control-free cell text,
  valid wide head/continuation pairs, and content-redacted `Debug`.
- [x] Remove presentation-only `TerminalState`/ANSI snapshot-delta DTOs once all semantic consumers
  compile. Keep only domain values with a current semantic/lifecycle consumer.
- [x] Instantiate the existing generic cache with semantic rows in focused tests; preserve all
  anchoring, slice, prefetch, invalidation, and maximum-240-row behavior.

### Step 10.2 — Define the wire-major-2 semantic protocol

- [x] Move protobuf sources/package and generated Rust module from `proto/zterm/v1` / `zterm.v1` /
  `zterm_proto::v1` directly to `proto/zterm/v2` / `zterm.v2` / `zterm_proto::v2`. Delete the v1
  generated module and do not compile dual schema generations. Preserve unchanged independently
  versioned persistent/ticket message shapes without retaining a wire-v1 transport.
- [x] Add protobuf color/style/cell/row/surface/cursor messages plus semantic snapshot, delta,
  full-row patch, and semantic history-window messages. Generated `Debug` for content-bearing values
  must be overridden/redacted exactly like current ANSI frames.
- [x] Increment product wire major and normal/pair ALPN identifiers. Reassign canonical content kinds
  301/302/318 to semantic snapshot/delta/history-window under the new major; keep request 317.
- [x] Delete bit 21, presentation encoding/preference, legacy payload messages, temporary 319..321,
  pager 312/313, viewport 315/316, and their generated/conversion/allowlist code.
- [x] Implement conversion/validation with exact row/cell counts, color/style ranges, wide structure,
  revision/anchor binding, control-free content, request correlation, max-size checks, and sentinel-
  redacted errors/debug output.
- [x] Prove a maximum legal viewport and maximum 240-row semantic window encode below 8 MiB. This is
  a correctness bound, not an RSS/performance benchmark or aggregate memory admission.

Rollback point: core/proto cutover commit; rollback requires reverting the whole wire-major slice.

## Phase 11 — Make Semantic Projection the Primary Model Output

### Step 11.1 — Project exact surfaces and full-row patches

- [x] Preserve private `ProjectedScreen`/inline cell storage, add exact conversion including wrapped
  rows, and produce semantic snapshots directly from one model lock/revision.
- [x] Compare checkpoint and latest projection once. Compatible updates emit complete metadata plus
  sorted full-row replacements; size/screen/format/future mismatch emits a semantic snapshot.
- [x] Preserve revision-only/no-visible-row transitions and current scroll metrics. Prove applying
  any accepted patch yields byte-for-byte equal semantic state to a fresh snapshot.
- [x] Add semantic history-window projection from the same row projector and existing coordinate
  formula. It must not mutate display offset, revision, checkpoint, history identity, or another
  attachment.

### Step 11.2 — Make the driver boundary semantic-only

- [x] Return semantic snapshot/delta/window types directly while retaining the semantic
  `TerminalCheckpoint`; remove presentation-family wrappers and selection state.
- [x] Delete `encode_full`, `encode_delta`, row/history ANSI, `recent_history_ansi`, their module/API,
  and all presentation-only byte tests; no recent-history stream exists in a snapshot.
- [x] Preserve the one model-owner thread, no-drop PTY queue, latest-only revision watch, reply order,
  resize transaction, final drain, and model/checkpoint lifecycle. Do not add an actor or parser.

### Step 11.3 — Delete the legacy model adapter

- [x] Remove the legacy ANSI encoder, old snapshots/deltas/history projections, extent-only fix, and
  strict compatibility oracle after semantic equivalence tests own all right-edge/wide/blank cases.
- [x] Prove no daemon/model source constructs terminal presentation ANSI and no CLI parser is added.

Rollback point: model semantic producer and ANSI deletion are one source slice.

## Phase 12 — Carry the Semantic Contract Through Session and Transport

### Step 12.1 — Use semantic values for the attachment lifecycle

- [x] Return semantic initial snapshot/resume delta directly. Use semantic values for acknowledgement
  replacement, next/final update, sync request, reconnect, takeover, and history-window response.
- [x] Preserve controller generation, pending takeover, resume view identity, latest-only update,
  input/resize fences, detach, zero-attachment drain, and one-Session/one-PTY behavior.
- [x] Permit exact checkpoint resume; incompatible revision/shape produces a complete semantic
  snapshot. Delete family-change and cross-encoding state.

### Step 12.2 — Enforce semantic local and remote paths

- [x] Local server/client accept only the wire-major-2 semantic content kinds; attach carries no
  presentation preference and no family state.
- [x] Remote connection establishment rejects the old ALPN/wire major. The bridge forwards semantic
  payloads without negotiation, translation, or ANSI synthesis.
- [x] Update local/direct/relay allowlists, decoder state machines, correlation, deadline,
  stream-loss Gap, redaction, and content/control size classification for canonical 301/302/318.

### Step 12.3 — Prove coordinated cutover and reconnect behavior

- [x] Test latest/latest semantic and explicit local/network rejection of the previous wire major/ALPN.
- [x] Cover initial full and resume delta, wrong semantic kind, wrong attachment/revision, ack mismatch,
  sync-required, final drained update, takeover, and true reconnect.
- [x] Add source/registry assertions that legacy payloads, 312/313, 315/316, 319..321, bit21,
  wire-v1 generated modules, presentation encoding, and downgrade branches are absent.

Rollback point: revert the whole wire-major transport slice; Session, PTY, trust, and persistent data
still require no migration.

## Phase 13 — Build the Client Surface and One Composed Frame

### Step 13.1 — Split explicit presentation owners

- [x] Extract focused private CLI modules for `AttachmentSurface`, composition/regions, and the
  semantic ANSI presenter. Keep event orchestration/input routing in `terminal_ui.rs`; do not
  introduce a new crate when a module has only a desktop consumer.
- [x] Make `AttachmentSurface` validate/install full snapshots and transactionally apply exact
  contiguous row patches. On gap/malformed input, retain the last complete frame, request sync, and
  never partially promote desired/received state to presented state.
- [x] Use one semantic viewport cache over the generic reducer. Reset only at documented
  identity/resize/reconnect boundaries.

### Step 13.2 — Compose live/history plus chrome before encoding

- [x] Refactor status and scrollbar from ANSI writers into pure cell contributors. Allocate child,
  gutter, and status regions through `ChromeLayout` before any painting and reject overlap/out-of-
  bounds composition.
- [x] Build a bounded sparse absolute-row `ComposedFrame` from a complete visible source. Bound its
  owned cells by product content plus one status row; never allocate the full arbitrary physical
  `u16 rows * columns` rectangle.
- [x] Live uses `AttachmentSurface` and its cursor. History uses one complete cached semantic slice,
  Main/gutter layout, and hidden cursor while the live surface keeps advancing in the background.
- [x] Preserve Main `N-1 + gutter`, Alternate full `N`, remote status final row, narrow widths,
  viewport caps, resize coalescing, pinned-history mode ownership, one-line wheel, 16 ms latest-frame
  cadence, and 33 ms drag request pacing.
- [x] Make reconnect/path/RTT a chrome state input. Remove semantic-mode standalone newline/loading/
  status writes; cache miss and sync keep the last complete composed frame visible.

### Step 13.3 — Verify ownership transitions without a backend

- [x] Test Main-to-Alternate, Alternate-to-Main, gutter move/removal, status row move/removal,
  physical grow/shrink, live-to-history/return-live, background screen switch while pinned, and
  chrome-only updates as complete desired-frame comparisons.
- [x] Test rightmost styled cells, default/styled blanks, combining and old/new wide-span ownership,
  cursor clipping/visibility, huge physical row numbers, maximum status width, and region errors.
- [x] Add a test-only ownership audit showing every active semantic visual/mode change enters the
  `ComposedFrame`; no contributor receives a stdout writer.

Rollback point: semantic surface/compositor remains disconnected from the CLI until presenter cutover.

## Phase 14 — Cut Over to the Sole Semantic Desktop Presenter

### Step 14.1 — Implement exact physical transitions

- [x] Retain the last successfully flushed `ComposedFrame`. Diff the union of old/new owned cells,
  expand changes through both wide spans, batch safe equal-style runs, and start every run with
  absolute `CUP`.
- [x] Emit explicit default-style spaces for removals. Do not use incremental `EL0`/`EL2`, saved-
  cursor side channels, relative cursor assumptions, or pending-wrap state; always restore final
  cursor with absolute positioning.
- [x] Derive application cursor/keypad, bracketed paste, and focus observation from child semantics;
  keep physical mouse in Zterm-owned `1003/1006` capture and use child mouse/alternate-scroll only in
  the existing one-owner input router.
- [x] Build one complete buffer with DEC 2026 begin/end, cell transition, cursor, and modes; perform
  exactly one `write_all` and one `flush`. Commit baseline only afterward; unchanged frames do no I/O.

### Step 14.2 — Define full resync and failure truth

- [x] On missing/unknown baseline, physical resize, screen/layout identity change, representation
  change, or prior I/O failure, reset/clear and repaint the complete composed frame.
- [x] On partial write or flush error, mark baseline unknown, best-effort end DEC 2026, preserve the
  original error, and prove the next successful transition is full. Never advance revision/frame/
  chrome authority speculatively.
- [x] Keep `TerminalGuard` as the only pre/post-active lifecycle writer. During active semantic mode,
  route snapshot, delta, history cadence, status, reconnect, gutter, cursor, and modes only through
  `DesktopPresenter`.

### Step 14.3 — Integrate and remove compatibility

- [x] Connect the CLI only to semantic snapshots/deltas/history. No preference, capability, family
  variant, or legacy renderer remains.
- [x] Delete old ANSI snapshot/delta/history, pager/viewport fallback, atomic chrome side writers,
  conversions, helpers, and tests once the sole presenter owns their behavior.
- [x] Remove every module, public alias, generated artifact, Cargo feature/dependency, error branch,
  and fixture made unreachable by the cutover. Use compiler/Clippy, inverse dependency inspection,
  and a source/registry audit; do not retain deprecated shims or commented fallback code.
- [x] Audit every event-loop branch and stdout helper. Delete semantic-mode independent status,
  reconnect, scrollbar, history, capture, and cursor writers; reject future bypass in tests/spec.
- [x] Run the generic nested-TUI flow across first/continuous/reverse child-owned wheel, resize,
  Alternate exit/re-entry, history pin/return-live, reconnect, and write/flush retry. No fixture or
  production branch may contain Herdr/PiAgent identity.

Rollback point: revert the complete presenter + wire-major slice before release. There is no runtime
fallback switch or interim release.

### Phase 9–14 implementation evidence (2026-09-03, macOS arm64)

- The coordinated cutover now has one schema and representation: product wire major 2,
  `zterm/2` / `zterm-pair/2`, protobuf package/module `zterm.v2` / `zterm_proto::v2`, and canonical
  semantic terminal kinds 301/302/317/318. The v1 source generation, ANSI terminal DTOs/encoders,
  representation selection, family/capability state, pager/viewport fallbacks, and retired kind
  numbers 312/313/315/316/319–321 are absent from product source.
- `zterm-terminal` projects exact semantic snapshots, full-row deltas, and history windows from the
  one Alacritty model. Rightmost cells, wide spans, combining contents, and styled blanks are owned
  by semantic projection/composition tests; no legacy ANSI oracle remains.
- Session, local IPC, and remote attachment paths carry only semantic values. During this cutover an
  existing takeover ordering race was isolated and fixed with epoch-local `takeover_ready` state:
  a takeover response may clear pending state before the current snapshot acknowledgement, but
  activation still requires both events in the current epoch. Response-before-ack,
  ack-before-response, and reconnect epoch invalidation regressions pass. This preserves the
  lifecycle gate rather than relaxing it.
- `TerminalAttachment::sync_changed` returns `None` for an equal checkpoint, a delta or complete
  resync for a behind checkpoint, and a mandatory resync for ahead/divergent state. Initial attach
  and reconnect still send a complete mandatory synchronization. This prevents a snapshot-ack
  notification loop without adding a compatibility branch.
- The desktop path is split into private `terminal_ui/surface.rs`, `composition.rs`, and
  `ansi_presenter.rs` owners. `AttachmentSurface` applies semantic updates transactionally;
  `ChromeLayout`/`ComposedFrame` own live/history rows, gutter, status, cursor, and modes; only
  `DesktopPresenter` encodes the active frame, with one DEC 2026 buffer, one `write_all`, one
  `flush`, and post-flush baseline commit. `TerminalGuard` remains the pre/post-active writer.
- The application-neutral nested-TUI regression routes child-owned wheel input without changing the
  host viewport, updates a styled rightmost Alternate-screen cell through the sole presenter, and
  rejects `EL0`/`EL2` cleanup. The cross-process autospawn regression observes termios, daemon
  revision, complete presentation transactions, and cleanup/process lifecycle rather than treating
  incremental ANSI bytes as a screen snapshot. Its history fixture waits for a bounded quiescent
  model/presentation boundary; the resize/scroll/detach sequence passed 10 consecutive isolated
  runs after that synchronization fix.
- Revision and history epoch identify an ordered update sequence; they are not content-equivalence
  hashes. Chunk/corpus equivalence therefore compares complete observable surfaces, replies, and
  events while lifecycle tests separately enforce revision/epoch ordering and invalidation.
- Focused suites pass for core, proto, terminal, daemon, and CLI. CLI has exactly three pre-existing
  ignored isolated-child helpers: `terminal_ui::unix::tests::panic_hook_child`,
  `terminal_ui::unix::tests::panic_restore_child`, and
  `terminal_ui::unix::tests::signal_restore_child`; their parent process tests execute each helper
  explicitly. No ignore was added for this migration.
- Source-policy and manual audits reject legacy presentation identifiers/kinds, a v1 protobuf tree,
  direct CLI engine/parser dependencies, application/terminal-brand detection, and Zterm-owned
  unsafe code. The only `zterm/1` references are explicit ALPN rejection tests; wire-major value 1
  remains only in explicit old-major rejection fixtures (protobuf field tag `= 1` is not a value).
- No throughput, latency, CPU, RSS, or candidate-comparison benchmark was run, and this evidence
  makes no migration performance claim. Linux/other-macOS-architecture/Windows hosted checks and
  real Ghostty/second-terminal/Linux connection smokes remain Phase 15 external evidence.

Final implementer handoff validation on the same macOS arm64 worktree passed:

- `sh tests/source-policy.sh`
- `cargo fmt --all -- --check`
- `cargo check --workspace --all-targets --all-features`
- `cargo clippy --workspace --all-targets --all-features -- -D warnings`
- `cargo test --workspace --all-targets --all-features`
- `cargo doc --workspace --no-deps`
- `just check` (including secret/release/workflow/shell policies, Cargo deny for the workspace and
  isolated relay probe, and the locally available relay static/publication/upstream checks)
- `git diff --check`
- `python3 .trellis/scripts/task.py validate .trellis/tasks/09-02-migrate-alacritty-terminal`

The task validator emitted only non-fatal context-injection size warnings for
`local-daemon-ipc.md` and `transport-auth.md`; both manifests otherwise validated. A final manual
source audit found zero product occurrences of the deleted DTO/family/encoding identifiers, retired
kind numbers, v1 protobuf files, direct core/proto/CLI terminal-engine dependencies, Rust `unsafe`
constructs, or application-name detection. Remaining text matches are intentional: the source-policy
guard contains the banned-name pattern, platform tests exercise terminal-environment pass-through,
Alacritty exposes a standard `kitty_keyboard` protocol mode, and old ALPN/wire-major values occur in
negative rejection fixtures. Active-mode stdout acquisitions all feed `ComposedFrame` into
`DesktopPresenter`; the other writers are pre/post-active `TerminalGuard`, normal-mode prompts and
diagnostics, command output, or test infrastructure.

Independent Trellis review and final automated closure on 2026-09-03 added/fixed the following:

- Proto Changed/Gap history decoding now rejects `current_epoch > current_revision` and any
  `current_revision` older than the saved query anchor; malformed content-free outcomes cannot
  bypass request identity.
- Presenter failure coverage includes partial-write-then-error as well as flush failure, proving the
  original error survives, DEC 2026 end is best-effort, the committed baseline is cleared, and the
  next successful attempt is a full repaint.
- The unused public `TerminalState` compatibility value and its self-only fixtures were deleted;
  source policy now bans its return with the other retired presentation DTOs.
- Current Trellis contracts were synchronized for terminal model/driver/Session, semantic wire v2,
  local IPC/remote structural bridge, transport ALPN, attachment cache/composition/presenter,
  cross-layer/cross-platform reasoning, coordinated distribution, and Relay ALPN. README now points
  at `proto/zterm/v2`; historical research explicitly marks the rejected mixed-version route as
  non-authoritative.
- The final post-spec `just check`, format, source policy, task-context validation, and
  `git diff --check` all pass. Cargo deny reports only allowed duplicate-dependency warnings; task
  validation reports only the known non-fatal context-injection size warnings for
  `local-daemon-ipc.md` and `transport-auth.md`.

Automated acceptance is complete on this macOS arm64 worktree. Real Ghostty plus a second terminal,
macOS direct/Relay, Linux local/direct/Relay, hosted Windows/other architectures, and formal release
artifact evidence remain external Phase 15.2 work. Android UI/touch rendering remains intentionally
deferred, and no performance/RSS claim was measured.

## Phase 15 — Close Quality, Platform, Knowledge, and Release Gates

### Step 15.1 — Focused and repository verification

- [x] Run focused core/terminal/proto/daemon/CLI tests for semantic validation, projection, transport,
  surface, cache, compositor, presenter, old-major rejection, input ownership, and all failure transitions.
- [x] Run format, all-target/all-feature check and Clippy with `-D warnings`, workspace tests/docs,
  source/secret/dependency policies, cargo-deny, relay checks, and the repository-prescribed final
  `just check`. Do not run performance/RSS benchmarks or use test durations as performance evidence.
- [x] Perform an independent Trellis review across model -> Session -> proto -> local/remote bridge ->
  client surface/cache -> compositor -> presenter. Fix every verified issue and repeat affected
  owner-level gates.
- [x] Run a final source/behavior audit proving product Rust remains unsafe-forbidden, no second
  parser/renderer or application detection exists, all semantic content is redacted/bounded, and
  only one active semantic presenter writes the physical terminal.

### Step 15.2 — Cross-platform and real-terminal acceptance

- [ ] On macOS, verify local/direct/relay with the reviewed binary in Ghostty and at least one other
  available terminal. Exercise shell history, generic nested TUI, Herdr entry/first/continuous/reverse
  wheel, resize, screen exit/re-entry, return-live, reconnect, detach, and cleanup.
- [ ] On Linux, separately verify local/direct/relay with the generic styled right-margin/wide-cell
  nested-TUI fixture, resize/screen/history/reconnect/detach/cleanup. Hosted Linux evidence cannot be
  inferred from macOS; unavailable external smoke remains a release blocker rather than a claim.
- [ ] Run existing hosted macOS/Linux/Windows shared-boundary and four native release-readiness jobs;
  record artifact architecture/floor/SBOM/license/source/dynamic-dependency results without expanding
  Windows or mobile runtime claims.

### Step 15.3 — Persist the implemented architecture

- [x] Use `trellis-update-spec` to update terminal-model, terminal-driver, session-service,
  core-wire-domain, local-daemon-IPC, transport-auth, cross-layer, cross-platform, distribution,
  development, and persistent-session knowledge with actual implemented contracts and tests.
- [x] Re-run task context validation and `git diff --check`; ensure no generated spec/template mirror
  is stale and task research records any design deviation with evidence.
- [x] Produce one final acceptance report distinguishing automated, macOS, Linux, hosted, deferred
  Android, and explicitly unmeasured performance evidence. Do not mark the task complete because the
  Herdr fixture alone passes.
- [ ] Only after every completion item passes may the reviewed commits proceed through the user's
  requested push/PR/CI/merge/release workflow. No Phase 9–14 checkpoint is an interim release.

### Semantic migration completion gate

- [x] Current live and history presentation is semantic end to end; no ANSI is constructed, parsed,
  or translated on that attachment before the sole desktop backend encodes the composed frame.
- [x] One retained attachment surface, one complete composed frame, and one successfully committed
  physical baseline cover terminal, history, status, gutter, cursor, and host modes.
- [ ] Generic nested TUI/right-margin, ownership transitions, reconnect, old-major rejection, and failed-
  output recovery pass the complete automated and macOS/Linux acceptance matrix.
- [x] Legacy presentation code and negotiation are absent; an old binary is rejected before attachment.
- [x] Product unsafe prohibition, one-Session/one-PTY/model, no-performance-test decision, bounds,
  specs, quality gates, and single-release boundary all remain satisfied.

### Requirement traceability

| Requirement / invariant | Owning implementation phases | Closing evidence |
| --- | --- | --- |
| R21 exact semantic surface | 10, 11 | core/proto/model shape, replay, bounds, redaction tests |
| R22 mandatory wire cutover | 10, 12, 14 | wire-major/ALPN rejection plus canonical semantic registry |
| R23 one model/semantic attachment | 11, 12 | checkpoint, resume, ack, final-drain, no-ANSI source tests |
| R24 complete client composition | 13 | surface transaction, cache, region, history/live transition tests |
| R25 sole presenter/commit truth | 14 | no-bypass audit, exact diff, one write/flush, failed-output retry tests |
| R26 obsolete-path deletion | 10–14 | source/registry audit and semantic-only integration tests |
| R27 quality/platform/release | 15 | repository gates plus distinct macOS/Linux evidence and final report |
| No application-specific patches | 9, 13, 15 | generic nested-TUI fixture plus source/ownership audit |
| Android-ready seam, not Android UI | 10, 13, 15 | core/proto graph and semantic-cache tests; explicit deferred list |

No phase may be skipped because a later row in this table appears green. In particular, passing a
Herdr smoke does not satisfy the semantic-only source, ownership, and wire cutover contracts.

### Semantic migration authorization

- [x] User confirmed the full semantic-presentation scope and one user-visible release boundary.
- [x] User explicitly removed mixed-version compatibility and will upgrade all nodes together.
- [x] PRD, design, research, implementation phases, and task context were reconverged for the direct cutover.
- [x] The revised final planning summary was presented and the user explicitly approved implementation
  afterward. Product-code work may resume under Phases 9–15.
