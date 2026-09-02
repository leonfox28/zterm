# 迁移终端状态引擎至 `alacritty_terminal` 及连续滚动 follow-up

## Decision State

- 2026-09-02，用户已决定从 `vt100 0.16.2` 直接迁移到 Alacritty 官方维护的
  `alacritty_terminal`，不再进行候选选型或性能比较。
- 精确目标为 crates.io `alacritty_terminal = "=0.26.0"`，关闭 default features；它对应
  Alacritty 官方仓库提交 `94e7c8874e526b1e67b349d9ba30ddf81669119e`。
- Zterm 自有 Rust crate 继续执行 workspace `unsafe_code = "forbid"`。不引入社区 wrapper、
  Ghostty/Zig/C FFI、bindings、fork 或 in-repo unsafe island。
- `portable-pty`、一个 Session 对应一个 PTY、现有 child/reader/writer ownership、当前
  protobuf/wire major、latest-only attachment 和 daemon-lifetime session 均保持不变。
- 本任务不运行或新增 throughput、latency、CPU、RSS 或候选对比 benchmark，也不对迁移后
  性能作保证。功能兼容、不可信输入驱动状态的确定性安全上限、安全过滤和跨平台构建仍是
  必须通过的正确性门。
- 2026-09-02，用户进一步决定取消跨Session的128 MiB terminal-memory admission。迁移后不
  估算Alacritty内部容量，也不因估算内存拒绝create/resize；8-session数量限制、viewport/
  scrollback上限和针对不可信PTY输出的buffer/content caps继续保留。
- 本文、`design.md` 与 `implement.md` 是替代旧 Ghostty 方案的唯一可执行计划；旧调研仅作
  决策历史。原迁移已在批准后进入实施；新增 scroll follow-up 受文末独立 planning gate 约束。

## Goal

用官方 Rust `alacritty_terminal` 替换 daemon 的 VT parser/grid/state 实现，同时保留 Zterm
自有领域模型、ANSI wire projection、断线恢复、history paging、输入安全边界和PTY生命周期；
建立一个host-only engine边界，使未来desktop/mobile UI可以继续演进而不
依赖具体 terminal-core 类型。

## User Value

- 获得 Alacritty 已维护的现代 VT、scrollback、resize/reflow、mouse/focus/paste modes 和
  Unicode grid 能力，减少继续扩展小型 `vt100` adapter 的长期成本。
- 用户从 Ghostty、kitty、Alacritty、tmux 或普通 shell 启动/使用 Zterm 时，外层终端与
  daemon 状态机仍是清晰隔离的两层，不共享 parser、PTY 或内存对象。
- remote client 继续消费稳定的 Zterm snapshot/delta/history，而不感知 Alacritty 私有类型；
  后续可在不更换 PTY/session 架构的前提下增加 GUI renderer、选择、搜索或 semantic wire。
- 保持 Rust 产品代码无 `unsafe`，不承担第三方 Ghostty wrapper/FFI 的 soundness 与构建链。

## Pre-migration Baseline (Historical)

- `zterm-core::terminal::TerminalModel` 私有持有 `vt100::Parser`，公开值均为 Zterm-owned。
- `TerminalDriver` 已有一个 ordered model thread、固定容量 no-drop PTY byte queue、共享 model
  mutex、latest revision watch、独立 child interruption 和每 attachment 一个 checkpoint。
- `portable-pty` 只负责 process/PTY input-output/resize/wait/close；没有理由随 parser 更换。
- snapshot 由 recent history + active screen ANSI 组成；delta 是一个 merged latest-state
  update；history cursor 绑定 epoch/revision/bounds。
- current parser 的 cell text 是约22-byte inline storage；默认仍限制2,000 history rows、
  240x80 viewport和8 sessions。现有128 MiB aggregate cell projection与历史256 MiB RSS门
  将随本迁移移除，不迁移成Alacritty容量估算。
- `zterm-core` 同时是未来客户端共享领域 crate。若直接在 core 中加入 Alacritty，mobile
  client 会无谓编译 terminal/tty dependency graph，因此需要拆出 host-only engine crate。

## Requirements

### R1. Official, Pinned, Safe Rust Dependency

- root workspace 精确固定 `alacritty_terminal = "=0.26.0"` 且
  `default-features = false`；Cargo.lock checksum 是实际解析权威。
- 只使用 Alacritty 官方 crates.io package，不使用 `blit-*`、`crow-*` 等 fork，不使用 Git
  branch/tag 或社区 wrapper。
- 新增 `zterm-terminal` crate并继承 workspace lints。所有 Zterm-owned code 保持
  `unsafe_code = "forbid"`；第三方 dependency 内部可能有 `unsafe`，但不得把 raw pointer、
  FFI 或 unsafe contract 暴露给 Zterm。
- 迁移后 workspace 不再直接或间接依赖 `vt100`。`vte` 只允许作为官方
  `alacritty_terminal` 的锁定 transitive dependency或其官方 re-export，不得另选版本形成
  第二套 VT state engine。
- `cargo deny`、许可证、advisory、duplicate/source policy 必须通过。升级 Alacritty 时必须
  单独重审 Term/Grid/Cell/Event/Processor/TermMode 和资源常量，不能自动漂移 minor version。

### R2. Host-Only Crate Boundary

- `crates/core/src/terminal.rs` 保留 Zterm-owned DTO：size、screen、cell/style/cursor、modes、
  side events、update、snapshot/delta/history、screen selector constants 和 frame byte limiter。
- 新增 `crates/terminal`，持有 `TerminalModel`、opaque checkpoint、terminal error、
  Alacritty adapter、ingress policy、projector和ANSI encoder。
- daemon 直接依赖 `zterm-core + zterm-terminal`；proto保持core-only。CLI作为同时承载本地
  daemon的host binary，会经daemon传递包含engine，但不得直接依赖`zterm-terminal`，其
  terminal UI/wire边界仍只使用core/proto-owned values。任何Alacritty/vte type都不得出现在
  core、proto、wire、CLI UI、session public API或Debug output。
- `cargo tree -p zterm-core` 与 `cargo tree -p zterm-proto` 必须不含 `zterm-terminal`、
  `alacritty_terminal` 和 `vte`。这是当前 mobile-facing library acceptance；本任务不把host
  CLI crate当作mobile library，也不在手机运行engine。
- 产品代码不得调用 `alacritty_terminal::tty`、`event_loop` 或 process spawning API；
  `portable-pty` 继续是唯一 PTY owner。

### R3. Preserve the Zterm Terminal Contract

- 保留 `TerminalModel` 的行为边界：checked construction/size validation、ordered ingest、
  empty no-op、checked revision、same-size resize revision、snapshot、checkpoint、merged
  delta-or-resync、semantic state、history page。
- 非空外部 ingest 无论内部怎样分段，只推进一次 revision；chunk whole/one-byte/fixed/
  deterministic-random 必须得到相同 semantic state、replies 和允许的 side events。
- 保持 main/alternate screen、cursor、indexed/RGB/default colors、bold/dim/italic/underline/
  inverse、wide/continuation、bounded combining text 和当前 input modes。
- Alacritty 多出来但当前 DTO 无法表达的 hyperlink、palette mutation、underline color/style、
  strike/hidden、Kitty keyboard 和 graphics 不得伪装为已支持能力。普通 underline variants
  可保守归一为 current boolean underline；其余另立版本化能力任务。
- Alacritty无法区分“未写入 blank”和“显式 default-styled space”。两者在当前 wire/UI 中
  视觉等价，semantic fixture 必须归一化此差异；styled blank 仍必须保留。

### R4. Zterm-Owned Ingress and Effect Policy

- 在 Alacritty processor 前放置有界、streaming、chunk-invariant 的
  `TerminalIngressPolicy`。它只维护 control sequence/string framing 与 Zterm policy，不保存
  screen/history，不是 runtime fallback 或第二个 terminal state engine。
- canonical query policy保持不变：primary DA `CSI ?1;2c`、DSR `CSI 0n`、standard CPR 和
  private `CSI ?row;columnR`。Alacritty自身 `CSI ?6c`、secondary DA、window/color/mode query
  reply 等不得直接写回 PTY。
- 保留 `ESC g` visual bell、BEL audible bell、`CSI 8;rows;columns t` resize request、OSC 0/2
  title和 OSC 1 icon-name的 Zterm events；title/icon只保留最多 256 source bytes。
- `Config::osc52 = Disabled`，且 policy 对 OSC 52 read/write产生不含 payload 的 rejected event。
- OSC 8、其他 OSC、DCS/APC/PM/SOS 和未批准 control payload 在进入 Alacritty前被有界消费；
  只可产生分类，不得进入 grid、reply、snapshot、delta、history、Debug 或日志。
- 所有 policy buffers 有固定 byte cap。超限 sequence 丢弃到自己的 terminator并产生一个
  bounded classification；不得把部分危险 sequence 当 printable text重新注入。
- 禁用/拒绝 DEC synchronized-update 2026 与 Kitty keyboard mode，防止当前 wire 无法表达的
  state 和延迟批量执行绕过资源策略。

### R5. Projection, Snapshot, Delta, and History

- `Term<EventListener>` 和 `vte::ansi::Processor` 是唯一权威 terminal state；projection 从
  active grid读取 Zterm-supported subset，不使用 Alacritty renderer、selection 或 dirty state
  作为多 attachment truth。
- private `ProjectedScreen` 使用 fixed/inline bounded cell text、row wrap、cursor、modes 和
  active screen；`TerminalCheckpoint` 只保存一个 latest active viewport，不保存 engine type、
  inactive screen 或 history。
- full snapshot 由 Zterm allowlisted ANSI encoder生成：单一 screen metadata selector、受控
  clear/home/CUP/EL、当前 SGR subset、printable UTF-8、cursor visibility 和明确 mode transitions。
  不调用或转发任意 upstream formatter output。
- delta比较 owned checkpoint与最新 projected rows，重画 changed rows并恢复 cursor/modes。
  future revision、size mismatch、active-screen mismatch、checkpoint format mismatch 或
  delta bytes >= full snapshot 一律 `Resync`。
- snapshot继续先应用 `recent_history_ansi`、再应用 `screen_ansi`。8 MiB limiter只删除最老的
  complete history lines，不截断 active screen。
- main history通过 Alacritty grid negative-line range oldest-to-newest读取，不修改
  `display_offset`、revision、checkpoint或 live viewport。alternate active时仍返回 Changed。
- append below capacity保留 epoch；resize、clear/shrink、capacity eviction/identity ambiguity
  推进 epoch并返回 Changed/Gap，不拼接无法证明连续的 page。

### R6. Bounded Untrusted State Without Aggregate Memory Admission

- 将当前 implicit cell text上限变为显式 `MAX_CELL_TEXT_BYTES`，至少覆盖现有兼容语料；
  per-cell和 per-session combining storage同时有固定 cap。
- OSC 8 和 unsupported underline-color extras不得在 grid中形成未计量 heap。达到 combining
  budget后丢弃新增 zero-width scalar并产生 bounded classification，不能无限增长。
- 删除`TerminalResourceProjection`、`aggregate_cell_projection_bytes`及其session aggregate
  accounting，不用新的`reserved_terminal_bytes`替代。Alacritty的grid、row cache、processor
  buffer与resize retention由library/allocator管理，不参与create/resize准入。
- 保留独立的产品/安全边界：8 live sessions、最大viewport、2,000-row scrollback、policy/reply/
  event/wire byte caps和combining caps；这些边界不得因取消aggregate memory admission而删除。
- 通过unit/adversarial tests验证size arithmetic、各安全cap、history eviction和session关闭后
  engine/checkpoint被drop。本任务不运行RSS/CPU benchmark，也不再维护128/256 MiB判定。

### R7. Preserve PTY, Driver, Session, and Nested-Terminal Semantics

- 保留现有 `TerminalDriver` model-owner thread、fixed no-drop byte queue、I/O mutex、child
  interrupt/reaper 和 latest-only revision path；安全 Rust Alacritty不要求新 actor或 unsafe
  `Send/Sync` workaround。
- query replies仍在对应 ordered ingest后写回同一个 hosted PTY；attachment、外层终端或
  remote transport从不回答 child query。
- resize继续preflight size/revision/session dimension limits -> native PTY resize -> model resize ->
  publish revision；可预期失败不允许native/model尺寸分叉，不增加memory-admission步骤。
- zero attachments持续排水；slow client不建立 per-revision backlog；detach不结束 PTY；root
  exit、explicit close和 daemon stop仍是唯一生命周期终点。
- 产品 login shell显式获得稳定 capability profile：`TERM=xterm-256color` 与
  `COLORTERM=truecolor`，不继承启动 daemon/CLI 的 Ghostty/kitty/tmux identity。outer terminal
  只重放 Zterm allowlisted ANSI，不与 daemon engine嵌套共享对象。
- 保持一个 Session、一个 root child、一个 PTY、一个 authoritative terminal model；不引入
  Herdr/tmux式 workspace/tab/pane tree。

### R8. Cross-Platform and Release Boundary

- 现有 hosted CI 的 macOS arm64/x86_64、Linux arm64/x86_64 和 Windows shared boundary都
  编译并测试 `zterm-terminal`；`just ci-windows` 显式包含新 crate。
- 正式四个 macOS/Linux native release产物继续通过现有 architecture、macOS 13、glibc 2.28、
  SBOM、license、source和动态依赖检查。Alacritty作为 Rust rlib链接，不增加随包 dylib。
- Windows只宣称当前 hosted compile/test边界；Windows local login PTY runtime仍由既有 roadmap
  决定，不能因为 Alacritty可编译而宣称完整 runtime支持。
- Android/iOS remote clients通过 dependency isolation受益；本任务不声称
  `alacritty_terminal` 官方支持 mobile，也不实现 mobile local PTY或 pixel renderer。

### R9. Direct Cutover and Rollback

- 开发期间可在测试中用冻结的 Zterm semantic fixtures验证新 model，但 production path不
  同时运行两个 engine，也不加入 feature fallback。
- 最终删除 workspace/core 的 `vt100` dependency、callbacks、formatter和 screen clone；
  tests不得保留 `vt100` 作为 oracle。
- 差异按 visible semantic compatibility、approved normalization、security improvement或
  blocker分类；未分类的 must-preserve差异阻塞 cutover。
- 回滚方式是 source revert到迁移前提交。wire无持久化 terminal state，因此没有数据迁移或
  rollback schema。

## Acceptance Criteria

- [x] workspace精确锁定官方 `alacritty_terminal 0.26.0`、关闭 default features；无 Ghostty、
  wrapper/fork、FFI/bindings或 Zterm-owned unsafe。
- [x] 新 `zterm-terminal` 是唯一 engine owner；core/proto dependency graph不含engine；CLI
  没有direct engine dependency或upstream type泄漏；daemon仍只用`portable-pty`管理PTY。
- [x] workspace/Cargo.lock无 `vt100`；`vte` 只来自锁定的 Alacritty dependency path。
- [x] 原 terminal corpus在 whole/one-byte/fixed/random chunking下保持 main/alternate、Unicode、
  styles、cursor、modes和 exact DA/DSR/CPR语义；显式 default space差异已归一并记录。
- [x] OSC/title/icon/clipboard/unknown string、visual/audible bell、resize request和事件数量/byte
  caps通过安全语料，任何 secret sentinel均不出现在 state/wire/Debug/log capture。
- [x] combining flood、OSC 8 URI flood、sync-update、query flood和screen-switch adversarial tests
  证明所有PTY-input-controlled persistent buffers有界；不包含aggregate memory admission断言。
- [x] snapshot/history replay、small changed-row delta、screen/size/future/large-delta resync、history
  append/eviction/resize/alternate语义全部通过；checkpoint不持有 history或 engine type。
- [x] TerminalDriver/session/local+remote attachment tests证明 no-drop drain、reply ordering、resize
  transaction、detach/reconnect、controller takeover、natural/explicit close均未回归。
- [ ] hosted macOS/Linux/Windows checks和四个 release-readiness build通过；正式 artifact无新增
  未分发 dynamic terminal library。
- [x] product login shell在不同 parent `TERM/COLORTERM` 下始终看到固定 Zterm capability profile。
- [x] `TerminalResourceProjection`、`aggregate_cell_projection_bytes`、旧terminal benchmark/RSS
  resource gate均已删除；create/resize不因Alacritty内部容量估算被拒绝。
- [x] specs/docs/dependency policy更新为 Alacritty事实；旧 Ghostty方案明确只作历史。
- [x] 验收报告明确写“未运行性能/RSS benchmark，不作迁移后性能保证”，不得把普通测试耗时
  当成性能证据。

Hosted checks above await the next GitHub Actions run and are not claimed from
the local macOS arm64 implementation environment.

## Out of Scope

- Ghostty、`libghostty-vt`、社区 Rust wrapper、Zig/C rewrite或 unsafe FFI adapter。
- 替换 `portable-pty`，新增 PTY/session/pane层级，或改变 session persistence。
- font shaping、glyph atlas、GPU/CPU renderer、Android/iOS UI、selection/search交互；本次 follow-up
  明确批准的 CLI 右侧字符滚动条除外。
- Kitty graphics/keyboard protocol、OSC 8 hyperlink UI、advanced styles、palette/theme同步。
- protobuf semantic surface v2或 wire-major变化；本次 follow-up 允许在 v1 中增加有 capability
  保护的 viewport request/frame 和可选 scroll metrics 字段。
- Android/iOS local shell、mobile engine runtime/device/App Store/NDK验收。
- candidate comparison、throughput/latency/CPU/RSS benchmark或性能优化承诺。
- 对Alacritty内部grid/cache/processor capacity建立per-session或aggregate memory quota。

## Risks and Deferred Work

- `alacritty_terminal` 是 pre-1.0 library；即使精确锁定，未来升级也可能需要 adapter重写。
- 它没有 upstream snapshot/diff wire codec；Zterm-owned encoder/policy成为必须长期测试的代码。
- 迁移后的吞吐、latency和真实 RSS没有在本任务测量；如果用户体验出现问题，另立 profiling
  task，以实际 session workload测量，而不是恢复双引擎。
- 取消aggregate memory admission意味着8个合法Session的实际内存总量不再有128 MiB产品
  保证；Alacritty缓存可能在Session缩小后继续保留，通常到Session关闭才整体释放。若未来
  需要host级保护，应另立基于进程/OS压力的策略，而不是恢复不准确的`size_of<Cell>`估算。
- 当前 ANSI wire意味着 native mobile UI仍需要自己的安全 terminal widget/parser；未来
  semantic surface可以复用本次 `ProjectedScreen` seam，但不在本任务实现。
- 固定 `TERM=xterm-256color` 需要目标宿主具备对应 terminfo；正式 native runner和 real PTY
  fixture必须验证，缺失时作为 blocker处理，不能回退继承外层 TERM。

## Original Migration Review Status

- Product scope：已收敛。
- Engine choice：已由用户确定。
- Performance decision：已由用户确定不测。
- PRD/design/implementation plan：已按官方Alacritty、禁止unsafe、不测性能、取消aggregate
  memory admission并保留安全caps的决定完成交叉检查；task context validation通过。
- Original migration implementation authorization：已于 2026-09-02 获得；任务已通过
  `task.py start` 进入 `in_progress`。

## Post-release Scroll Follow-up (Planning Reopened 2026-09-02)

### Decision State

- 2026-09-02，用户确认 wheel ownership：child 未声明接管时，由 Zterm 浏览
  attachment-local history；child 通过标准 terminal mode 声明接管后，Zterm 不改变
  自己的 viewport，只向 child 精确转发一次事件。
- “转发”仍经过 Zterm。外层 Ghostty/kitty 向 Zterm 报告 SGR mouse，Zterm 根据 daemon
  投影的权威 child modes 决定由哪一层消费；physical host capture 与 child-requested
  modes 是两份独立状态。
- 2026-09-02，用户批准 CLI 默认采用 Herdr 式右侧单列滚动条：main screen 稳定预留，
  有历史才绘制并允许点击/拖拽，alternate screen 隐藏并收回该列。
- 用户明确后续路线：本功能完成并验证 Linux/macOS 连接后，下一阶段是 Android App。
  因而 scroll state/action/metrics 必须 renderer-agnostic；CLI 字符、颜色和 gutter 布局
  不得进入共享协议。
- fullscreen TUI 本身会 redraw；这不构成拒绝 gutter 的理由。需要验证的是 mode switch
  带来的至多一次额外 PTY resize/SIGWINCH、远程同步期间的短暂旧几何，以及退出后的
  main-screen reflow。

### Goal

修复普通 shell 中滚轮失效，并把现有整页 history browser 升级为 attachment-local、
可逐行浏览的连续 viewport；提供默认可见且可交互的 CLI 滚动条，同时保证 Herdr、Pi
等 nested TUI 每个 wheel report 始终只有一个 owner，并为随后 Android 原生 overlay
复用语义 metrics/actions。

### User Value

- shell 用户可以像普通终端一样逐行/逐页查看历史，并从 thumb 看到当前位置或快速跳转。
- fullscreen TUI 接管鼠标时不发生 Zterm 与 child 双重滚动，退出后无需应用特判即可恢复。
- Linux/macOS 先验证同一协议和 ownership；Android 随后复用语义状态，不复制 CLI 字符布局。

### Defect Evidence

- `crates/cli/src/terminal_ui.rs:69-70` 的 `TerminalGuard` 请求 physical SGR mouse；
  `crates/terminal/src/ansi.rs:133-143` 的受控 reset 又关闭 `1003/1006`；
  `crates/cli/src/terminal_ui.rs:2601-2643` 的 snapshot 不恢复 capture，delta 只在狭窄
  child-mode transition 后恢复。故障属于 CLI host-input integration，不是 Alacritty
  scrollback 数据丢失。
- `crates/cli/src/terminal_ui.rs:2038-2185` 的首次 `navigate(older, amount)` 请求 newest page
  时没有保存 `amount`，而返回值只有 history rows；它无法表达“3 行 history + 其余 live
  screen”，所以 capture hotfix 与连续 viewport 是两个独立验收项。

### R10. Separate Physical Capture from Child Modes

- 所有成功写入 physical terminal 的 snapshot、delta、resync replacement 和 history viewport
  frame 都必须把 `HOST_INPUT_CAPTURE` 作为该次 render transaction 的最后一个 mode write，
  然后 flush；普通 delta 不得依赖 child mode 是否刚好发生变化。
- `TerminalRenderer` 继续保存 daemon-authored child modes，用它决定输入路由；不得因为对外层
  重新声明 `1003/1006`，就在 child state 中伪造 mouse reporting。
- detach、正常退出、signal、错误和 panic guard 仍通过唯一 `TerminalGuard` cleanup 恢复外层
  terminal，不得遗留 raw mode 或 host mouse capture。

### R11. Renderer-neutral Continuous Viewport

- `zterm-core` 新增 Zterm-owned `TerminalScrollMetrics`：history epoch、model revision、
  `offset_from_bottom`、`max_offset_from_bottom` 和 `viewport_rows`；offset `0` 明确定义为 live。
- `zterm-core` 新增 `ScrollByLines(i32)` 和 `ScrollToOffset(u64)` 两个语义 action。
  `ScrollByLines` 正数向 older/up，负数向 newer/down；所有 arithmetic checked/clamped。
- scroll state 属于一个 attachment，不属于 Session 的 authoritative terminal state。
  daemon attachment 保存 action baseline；CLI/未来 Android 只保留最后一次返回的 metrics/frame。
  detach/reconnect 从 live 开始，不持久化 presentation offset。
- `zterm-terminal` 从同一个 Alacritty grid 直接读取 logical lines，绝不调用或暂时修改共享
  `display_offset`。offset 为 `N` 时，完整 viewport 依次投影 `Line(-N)..Line(rows-N-1)`；
  因此从 live 首次上滚 3 行恰好得到 3 行 history 加 `rows-3` 行 live screen。
- 一个 viewport frame 必须来自单个 model lock/revision，包含恰好 `viewport_rows` 个 canonical
  ANSI rows；它不改变 revision、checkpoint、child PTY、Alacritty state 或其他 attachment。
- 同一 history epoch 内有新 rows 进入 scrollback 时，下一次相对 action 先按
  `new_max - previous_max` 提升 offset，以保持原内容锚定，再应用用户 delta。epoch 改变、
  resize、clear 或 eviction 使精确锚点不可证明时，server 返回 `Rebased` 并在当前 retained
  bounds 内取最近合法 offset，不静默拼接两个 epoch。
- 普通键盘/paste/prefix input 在 history 中不会直接到 child：先进入 bounded
  `ResumePending`，取得并 flush live sync（以及必要的 geometry resize），再精确转发一次。

### R12. One Wheel Report, One Owner

- Zterm 已显示自己的 history viewport 时，wheel/PageUp/PageDown 继续由 Zterm 拥有，直到
  offset 回到 `0` 或普通 input 完成 live resume；后台 child mode 变化不得抢走 ownership。
- live 时路由优先级为：child mouse-reporting → alternate screen + alternate-scroll →
  Zterm main-screen history。只使用 `ActiveScreen` 与 `TerminalModes`，禁止 process name、
  `TERM`、tmux marker 或 Herdr/Pi 专用分支。
- child mouse-reporting 分支按声明 encoding 只写一个 mouse report；alternate-scroll 分支
  只写一个 application/normal cursor-key sequence。两者都保持 Zterm offset 为 `0`。
- Zterm-owned CLI wheel 每个完整 SGR report 固定映射 3 行；PageUp/PageDown 为
  `viewport_rows - 1`，至少 1 行。本次不增加配置项。
- 唯一例外是用户直接操作可见的 Zterm gutter：main screen 的 scrollbar column 位于 child
  PTY rectangle 之外，其 wheel/press/drag/release 由 Zterm chrome 拥有，永不伪造为 child
  最后一列事件。其他 child rectangle 内的事件遵循上述 mode ownership。

### R13. Herdr-style CLI Scrollbar and Geometry

- CLI 在 main screen 且受支持布局宽度大于 4 列时始终预留最右一列：child PTY 使用 `N-1`
  列，gutter 使用第 `N` 列。history 从 0 变为非 0 时不得触发 resize；无 history 时 gutter
  保持空白。
- 有 history 时用单格 track/thumb（计划采用 `▕`/`▐`）显示 metrics。thumb 至少一行；长度按
  `viewport_rows / (viewport_rows + max_offset)` 比例计算，位置把 oldest 映射到顶部、live
  映射到底部，并使用 overflow-safe integer arithmetic。
- track click 映射为绝对 `ScrollToOffset`；thumb drag 保留 grab offset，并把最新 pointer
  位置 coalesce 为一个绝对 target。wheel burst/drag 在已有 request 时只保留有界累计或
  最新 target，不建立 per-event network backlog。
- remote connection status row 不属于 scrollbar track；超过产品最大 viewport 的 physical
  rows/columns 也不进入 child/gutter coordinates。gutter 外的 mouse event 被丢弃。
- live alternate screen 隐藏并清除 gutter，将 `N` 列全部归还 child；回到 live main 时恢复
  `N-1 + 1`。每次 mode transition 最多提交一次新 size，same-size snapshot/delta 不得产生
  resize loop。
- 若 Zterm 已在 history 中，presentation screen 暂时保持 main/gutter；后台 child 切换
  alternate 只更新待恢复的 authoritative state。用户回 live 后才按最新 screen 进行一次
  geometry reconciliation。

### R14. Additive Wire Compatibility and Android Boundary

- proto v1 新增 `TerminalViewportRequest/Frame`、metrics/action/outcome message、新 wire kinds
  和 `TERMINAL_VIEWPORT` capability；不改变 wire major。request 是 control frame，frame 是
  content frame，继续受 1 MiB/8 MiB bounds、attachment identity、deadline 和 correlation约束。
- snapshot/delta 以新增可选字段携带 live scroll metrics，使 scrollbar 在尚未滚动时也能
  知道 history extent。旧 client 忽略未知字段；新 client 对未声明 capability/缺少 metrics
  的旧 remote peer 使用现有 bounded history paging，隐藏不可用的 thumb，不发送未知 kind。
- semantic viewport peer 每个 attachment 同时最多一个请求。`Ok` 返回一致 frame；`Rebased`
  返回当前 epoch 的完整替代 frame；offset 到 `0` 返回 `Live` 并走既有 sync；alternate/
  invalid state 返回明确 `Changed/Gap`，不能把错误 page 当作 current screen。
- core/proto/wire 中不得出现 `▕`、`▐`、颜色、gutter column 或 pixel。下一阶段 Android 可用
  同一 metrics/actions 绘制不占 PTY cell 的 native overlay；Android renderer、touch physics、
  本地 PTY 和 device validation 不在本次实现。
- Android 的后续 gesture adapter 应按 pixel delta / cell height 并保存 fractional remainder
  产生 `ScrollByLines`；不能继承 CLI“每个 SGR report 3 行”的输入设备假设。

### R15. Verification and Release Boundary

- pseudo-TTY/byte-level tests覆盖初始 snapshot、普通 delta、resync、child enable/disable mouse、
  history frame之后仍保留 host capture，以及所有 cleanup paths。
- model/proto/session/local+remote tests覆盖 3-history+live composition、top/bottom clamp、absolute
  jump、epoch rebase、one-outstanding/coalescing、malformed/oversized frame 和 mixed-version fallback。
- CLI tests覆盖 routing matrix、one-report invariant、PageUp overlap、scrollbar math、click/drag、
  width 4/5、remote status row、main↔alternate single resize、无 resize loop、history-pinned mode
  change和 resume input exactly once。
- 在进入 Android 阶段前，macOS 与 Linux 的 local/direct/relay 连接至少各完成一次真实 PTY
  smoke；进入/退出 Herdr 或 Pi-style fullscreen 后 shell、resize、wheel、detach/reconnect 正常。
  hosted-only evidence 必须明确来自对应 CI/runner，不得由本地 macOS 结果代替。
- 延续用户决定：不运行 throughput、latency、CPU 或 RSS benchmark，也不据普通测试耗时作
  性能结论。

### Follow-up Acceptance Criteria

- [ ] 普通 shell 在 snapshot、任意 delta 和 resync 后均能收到 wheel；退出后 outer terminal
  raw/mouse state 完整恢复。
- [ ] 首次 wheel-up 精确移动 3 行；连续 wheel、PageUp/PageDown、top/bottom 和 ordinary-input
  resume 均符合 R11/R12，且一个 attachment 不改变另一个 attachment 或 authoritative model。
- [ ] Herdr/Pi-style mouse modes 收到恰好一个 report；alternate-scroll 收到恰好一个 cursor key；
  child 退出 mode 后 wheel ownership 自动回到 Zterm。
- [ ] main-screen gutter 稳定预留、无 history 时不画、有 history 时可 wheel/click/drag；alternate
  screen 收回整列，mode-driven resize 无 loop、输入丢失或不可恢复 stale frame。
- [ ] 新 viewport wire/capability、optional metrics、frame bounds、redacted Debug、mixed-version
  fallback 和 local/direct/relay paths 都有自动化证据；旧 history paging contract 保留。
- [ ] macOS 与 Linux 的真实 local/direct/relay smoke 通过后，才将本 follow-up 标为完成并进入
  Android App；未运行性能/RSS benchmark，未宣称 Android 已实现。

### Out of Scope and Risks

- 本次不实现 Android UI、pixel/inertial touch physics、CLI scroll-speed 配置、selection/search、
  horizontal scrollbar 或 multipane compositor。
- main screen 永久少一列是用户批准的 UX 成本；main↔alternate 会多一次 PTY resize/SIGWINCH，
  最坏出现 child 自身 redraw 后再按新宽度 redraw。它必须被验证，但不是正确性 blocker。
- outer terminal 可把一次物理触控板手势拆成多个 SGR reports；3 行是“每个 report”，不是
  “每个物理手势”。当前 ANSI input 无法可靠恢复 pixel delta。
- history capacity eviction 或 resize 后旧锚点可能已不存在；`Rebased` 只保证返回当前 retained
  state 中最近的合法 viewport，不保证展示已经被淘汰的内容。
- mixed-version remote peer 只能得到既有 page-based fallback，不能获得新 continuous frame 或
  interactive thumb；升级两端后自动启用，不通过未知 wire kind 探测。

### Planning Gate

本 follow-up 的用户 UX 决策已经收敛。原迁移批准不覆盖这次新增的 wire、gutter 和 geometry
范围；只有在 PRD、design、implement manifests 全部校验完成并展示最终摘要后，用户再次明确
批准，才开始修改产品代码。
