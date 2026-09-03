# Implementation Plan

> 本文只在用户批准本任务最终计划后执行。批准前不得运行 `task.py start`，不得修改产品
> 代码。本计划完全取代旧 Ghostty wrapper/FFI 方案。

## Guardrails

- 只使用 Alacritty 官方 crates.io `alacritty_terminal = "=0.26.0"`，并关闭 default
  features；不使用 fork、Git revision、社区 wrapper、Ghostty、Zig 或 C FFI。
- 所有 Zterm crate 继续继承 `unsafe_code = "forbid"`；不得增加 unsafe island、raw binding、
  `unsafe impl Send/Sync` 或绕过 workspace lint。
- `portable-pty` 仍是唯一 PTY/process owner；一个 Session 仍对应一个 PTY、一个 root child、
  一个 authoritative model。
- 保留当前 protobuf/wire major、attachment/session 生命周期和 no-drop PTY drain；任何
  Alacritty type 都不得穿过 `zterm-terminal` 私有边界。
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
