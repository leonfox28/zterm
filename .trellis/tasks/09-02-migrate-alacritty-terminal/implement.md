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

## Pre-Start Approval

- [x] 用户已明确批准本 PRD、design和implementation plan（2026-09-02）。
- [x] 批准后已运行：

```sh
python3 .trellis/scripts/task.py start .trellis/tasks/09-02-migrate-alacritty-terminal
```
