# 迁移终端状态引擎至 `alacritty_terminal` 及连续滚动 follow-up

> Sections R1–R20 retain completed migration/scroll history. Where those historical contracts
> mention ANSI wire, fixed wire major, capabilities, or fallback, the final R21–R27 direct-cutover
> section is authoritative.

## Decision State

- 2026-09-02，用户已决定从 `vt100 0.16.2` 直接迁移到 Alacritty 官方维护的
  `alacritty_terminal`，不再进行候选选型或性能比较。
- 精确目标为 crates.io `alacritty_terminal = "=0.26.0"`，关闭 default features；它对应
  Alacritty 官方仓库提交 `94e7c8874e526b1e67b349d9ba30ddf81669119e`。
- Zterm 自有 Rust crate 继续执行 workspace `unsafe_code = "forbid"`。不引入社区 wrapper、
  Ghostty/Zig/C FFI、bindings、fork 或 in-repo unsafe island。
- `portable-pty`、一个 Session 对应一个 PTY、现有 child/reader/writer ownership、latest-only
  attachment 和 daemon-lifetime session 均保持不变。最初迁移保持 wire major；最终 semantic
  direct cutover 按 R22 协调升级 wire major/ALPN。
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
自有领域模型、断线恢复、输入安全边界和PTY生命周期，并最终以 semantic surface/history
window 取代 ANSI wire projection 与兼容 paging；
建立一个host-only engine边界，使未来desktop/mobile UI可以继续演进而不
依赖具体 terminal-core 类型。

## User Value

- 获得 Alacritty 已维护的现代 VT、scrollback、resize/reflow、mouse/focus/paste modes 和
  Unicode grid 能力，减少继续扩展小型 `vt100` adapter 的长期成本。
- 用户从 Ghostty、kitty、Alacritty、tmux 或普通 shell 启动/使用 Zterm 时，外层终端与
  daemon 状态机仍是清晰隔离的两层，不共享 parser、PTY 或内存对象。
- remote client 继续消费稳定的 Zterm semantic snapshot/delta/history，而不感知 Alacritty 私有类型；
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
  side events、semantic surface/snapshot/delta/history-window 和 frame byte limiter。
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
  empty no-op、checked revision、same-size resize revision、semantic snapshot、checkpoint、merged
  semantic delta-or-resync、semantic state、history window。
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
- full snapshot 直接投影为 exact rectangular semantic surface；desktop ANSI 只在最终 compositor
  后由唯一 presenter 生成，不调用或转发 upstream formatter output。
- delta 比较 owned checkpoint 与最新 projected rows，产生 sorted full-row replacements 和完整
  cursor/modes/metrics；future revision、size/screen/format mismatch 一律 `Resync`。
- snapshot 不含 `recent_history_ansi`；8 MiB frame cap 在 proto semantic payload 处执行，历史由
  bounded history-window/cache 独立读取。
- main history 通过 Alacritty grid negative-line range oldest-to-newest读取，不修改
  `display_offset`、revision、checkpoint或 live viewport。alternate active时仍返回 Changed。
- append below capacity保留 epoch；resize、clear/shrink、capacity eviction/identity ambiguity
  推进 epoch并返回 Changed/Gap，不拼接无法证明连续的 window。

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
- persistent-state schema migration 或保留多 wire major 并行服务；R22 的一次性 wire-major/ALPN
  cutover 明确在 scope 内。
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
- semantic wire 让 native mobile UI 后续直接消费 surface/cache；pixel renderer、font/IME/touch
  仍不在本任务实现。
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
- Zterm-owned CLI wheel 每个完整 SGR report 固定映射 1 行；PageUp/PageDown 为
  `viewport_rows - 1`，至少 1 行。本次不增加配置项。该最终输入契约由 2026-09-03
  smooth-viewport follow-up 修正，取代 v0.1.11 的 3 行常量，但不改变 315/316 action wire 语义。
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
  产生 `ScrollByLines`；不能继承 CLI 按离散 SGR report 计行的输入设备假设。

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
- [ ] 首次 wheel-up 精确移动 1 行；连续 wheel、PageUp/PageDown、top/bottom 和 ordinary-input
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

## Smooth Client-owned Viewport Follow-up (Approved 2026-09-03)

### Decision State

- v0.1.11 真实使用暴露两个独立问题：外层 Ghostty 的一个离散 wheel tick 默认可产生三个
  SGR reports，而 CLI 又把每个 report 乘以三行，导致一次物理滚动约九行；历史帧和滚动条
  又通过 clear-before-draw 的全屏 repaint 显示给 host，滚轮和拖动均可见闪烁。
- 用户批准吸收 Herdr 可迁移的 presentation 原则：保留旧帧直到完整新帧就绪、内容与 chrome
  同帧提交、outer-host DEC 2026 synchronized output、无 clear-before-content、drag pacing、
  one-in-flight/latest-wins 和 release 最终位置。
- 用户进一步批准桌面与后续 Android 统一为 client-owned viewport/cache：daemon 继续唯一持有
  Alacritty 与完整 scrollback，客户端持有有界连续窗口、当前 offset 和交互动画；网络只用于
  初始预取、低水位补窗、绝对跳转、cache miss 和 identity rebase，不再位于每个 wheel/motion
  的关键路径。
- 既有 kind 315/316 的“一次 action -> 一屏 frame”语义和 312/313 history pager 保持不变，
  供 mixed-version fallback 使用。新窗口是 additive capability，不靠旧 peer 接收未知 kind
  来探测。
- 本阶段只实现 desktop CLI 的 canonical-ANSI row adapter 和 renderer-neutral cache reducer。
  Android UI、pixel/fling physics 及 semantic-cell wire encoding 仍属于下一任务；不得为了未来
  consumer 在本阶段预建半套移动 renderer 或第二个 terminal model。

### R16. Correct and Atomic Desktop Presentation

- 一个由 Zterm 拥有的完整 SGR wheel report 映射一行，而不是三行。外层 terminal 自己产生
  多少完整 reports 就移动多少行；child mouse 和 alternate-scroll 分支仍恰好转发一个事件。
- 每次 outer-host presentation 必须构成一个 byte transaction：DEC 2026 begin、必要的 cursor
  hide、child/history 内容、status/scrollbar、`HOST_INPUT_CAPTURE`、最终 cursor/mode 状态、
  DEC 2026 end，然后恰好一次 `write_all` 和一次 `flush`。cleanup 必须无条件发送 DEC 2026 end，
  防止 partial write/错误/panic 将支持该 mode 的 host 留在同步状态。
- viewport/window 请求发出时不得清除或重画未变化的旧画面，不显示空白 loading/returning
  中间帧。只有一份完整可呈现的新 frame 才替换当前画面。
- history row 不得 `EL2` 后再写内容。先覆盖目标内容，再清理行尾；不支持 DEC 2026 的 host
  仍不能看到先整行变空的阶段。scrollbar/status 与内容不得分开 flush。
- drag motion 仅在 target 改变且距上次发送至少 33 ms 时触发远端补窗；release 必须提交最新
  最终 target。现有 one-in-flight/latest target 合并继续保留。

### R17. Additive Read-only History Window

- `zterm-core` 新增 renderer-neutral window anchor/request/frame/result DTO。anchor 至少包含
  epoch、revision、max offset 和完整 viewport size；frame 包含 resolved target、当前 anchor、
  相对当前 live top 的首行坐标，以及 oldest-to-newest 的独立 canonical rows。
- 坐标以响应 revision 的 live screen top 为 `0`，history 为负数，live screen 为 `[0, R)`。
  target offset `O` 的完整 viewport 是 `[-O, R-O)`；retained 总范围是 `[-H, R)`。
- 请求携带 target `O` 与 older/newer margins。返回窗口为
  `start=max(-H,-O-older)`、`end=min(R,R-O+newer)`，并包含 `[start,end)` 的每一行。
  `MAX_HISTORY_WINDOW_ROWS = 240`，margins 总和不得超过 `2R`，最终 content 仍受 8 MiB gate。
- 该读取在一个 model lock/revision 下从 Alacritty grid 投影，不调用 `scroll_display`，不改变
  revision、checkpoint、PTY、attachment legacy scroll baseline 或其他客户端。
- 同 epoch/size 且 extent 单调增长时，将 anchor 坐标中的 target 增加 `H'-H` 以固定内容；
  epoch/size 改变或 extent 收缩时返回完整 `Rebased` window；alternate 返回 `Changed`；结构非法
  或 future anchor 返回 `Gap`。任何结果都不得拼接无法证明同 identity 的行。

### R18. Renderer-neutral Bounded Client Cache

- `zterm-core` 提供无 async、ANSI、手势和平台依赖的 generic row cache/reducer；CLI 仅适配
  `Vec<u8>` canonical ANSI row，未来 Android 可复用相同 anchor/range/state transition。
- client 是 presentation offset 的权威 owner。完整 slice 已被 cache 覆盖时，wheel/Page/drag
  只更新本地 desired offset 并立即 render；不得发送 315 或新 window request。
- Active 且有 main history 时可机会式预取；首个 miss 拉取至多三屏。距任一可滚动 cache edge
  小于 `R/2` 时后台预取相邻窗口。每个 view 同时最多一个 window request；pending 时只保留
  latest desired target，不积累 pages/events。
- cache miss、跳转或 pending response 未覆盖最新 target 时保留最后完整画面。response 覆盖
  latest target 才允许替换；否则不呈现该中间 target，并立即为 latest target 请求一次窗口。
- same-epoch append 按 extent 增量平移 cache 坐标和 pinned offset。resize/reflow、epoch 变化、
  extent 收缩、true reconnect、takeover 和 explicit live reset 清除或完整替换 cache；不得把
  cached rows 存入 `RemoteResumeCheckpoint`、Session model、磁盘或全量 transcript。
- 一个客户端最多持有 `MAX_HISTORY_WINDOW_ROWS` 行；这是交互工作集上限，不是恢复已取消的
  per-session/aggregate terminal-memory admission。

### R19. Wire Compatibility and Mobile Seam

- proto v1 使用 next free kinds 317/318 和 capability bit 20 增加纯读 history-window request/frame；
  kind 315/316、bit 19 与其 validators 均保持原语义和编号。
- request 是 control，response 是 content；attachment/request correlation、deadline、8 MiB limit、
  redacted Debug 和 local/remote authorization 沿用现有 terminal read paths。旧 remote peer 无
  bit 20 时，新 CLI 回退至 315/316，再回退到 312/313，不发送 317/318。
- 首版 row payload 是当前 allowlisted、独立的 canonical ANSI rows。window metadata、cache reducer
  和失效语义不得依赖 ANSI；Android 后续通过独立 capability/encoding 增加 semantic cells，
  不修改本阶段稳定的坐标/anchor/cache contract。
- daemon 新 window path 是无 presentation state 的纯读请求。旧 315/316 所需的
  `ActorAttachment.scroll` 只为 mixed-version fallback 保留，不能成为新 cache 的真相源。

### R20. Verification and Scope Boundary

- core/model tests覆盖 window 公式、0/live、1、mid、oldest、clamp、same-epoch append、rebase、
  invalid/future anchor、alternate、Unicode/wide/style、240-row cap 和 model immutability。
- proto/session/local/remote tests覆盖 kind/capability 稳定、bounds、redaction、correlation、
  authorization、one outstanding/latest target、epoch loss、reconnect 和两级 fallback。
- CLI tests覆盖一个 host report 一行、child one-report、初始预取、本地连续滚动不发请求、
  edge prefetch、jump miss、stale response、drag 33 ms/release、无中间 blank、DEC 2026/cleanup、
  no-clear-before-content 以及 status/gutter/capture 单事务。
- 不新增或运行 throughput、latency、CPU、RSS benchmark。macOS/Linux local/direct/relay 的真实
  连接证据仍由各自环境提供；本地 macOS 不能替代 Linux 或 hosted release evidence。

### Follow-up Acceptance Criteria

- [ ] 外层 Ghostty 的一次默认离散 wheel tick 不再被 Zterm 二次乘三；每个 host-owned report
  精确移动一行，nested TUI 路径仍只有一个 owner/一个事件。
- [ ] history/scrollbar/status 的所有替换均无 blank/returning 中间帧，使用安全 cleanup 的
  DEC 2026 transaction、一次 write/flush 和 content-before-tail-clear。
- [ ] 新 window API 在一个 revision 下返回最多 240 个连续 rows；append/rebase/gap/alternate
  与所有 bounds/correlation/redaction 条件有直接测试。
- [ ] 桌面在 cache 覆盖范围内滚轮、Page 和 drag 不产生网络 viewport request；edge/jump 只
  产生一个 in-flight request 并最终呈现 latest target。
- [ ] 新旧 daemon/client 组合按 bit 20 -> bit 19 -> history paging 顺序安全降级，未知 kind
  永不发送给未协商 peer。
- [ ] core/proto 仍无 host engine 依赖，产品代码继续禁止 unsafe，Android UI/semantic renderer
  未被本阶段虚假宣称完成。

### Explicitly Out of Scope

- Android UI、touch velocity/fling、pixel renderer、font shaping、glyph atlas 和 device validation。
- semantic-cell live snapshot/delta 或 history-window wire；其 encoding 在 Android 任务中独立协商。
- 完整 scrollback 镜像、持久化 transcript、第二个 client terminal parser/model、Ratatui 引入。
- Herdr 的全 cell diff、全局固定 60 Hz scheduler、Kitty graphics 拼接或应用名特判。

### Authorization

- [x] 用户在讨论 desktop/mobile 统一缓存、兼容边界和分阶段实现后明确回复“好 按你说的做”
  （2026-09-03）。该回复授权本 follow-up 的 planning amendment 与产品代码实施。

## Wheel Burst Presentation Pacing Amendment (Planning, 2026-09-03)

### Goal and confirmed facts

- 实机已确认一个 Ghostty wheel burst 的总位移为三行，修复后的“一份完整 SGR report = 一行”
  语义正确；不得重新乘以三。
- 当前 CLI 对 burst 中每份 host-owned report 都立即更新缓存并各自 `write_all`/flush 一帧，
  因而 host 有时只显示最终三行状态，有时显示一行/两行的中间状态。总 offset 正确，但视觉
  cadence 不稳定。
- SGR mouse report 不携带物理 gesture/notch ID，不能无启发式地证明哪三份报告属于一次动作。
- Herdr 的可迁移机制是 16 ms render/presentation cadence、dirty 合并、latest complete frame 和
  单 render slot；不是它的默认三行 input multiplier。

### In scope and requirements

- 仅对 Zterm-owned cached viewport repaint 增加有界 cadence；每份 report 仍立即、checked 地
  累计 desired offset，最多每 16 ms 呈现一次最新完整 slice。
- child-owned mouse/alternate-scroll、网络 prefetch/request、普通键盘输入和 authoritative
  snapshot/resync 不等待 wheel cadence。后者必须取消或吸收 pending repaint，避免旧历史帧
  在状态切换后补画。
- 不引入全局 PTY-output 60 Hz scheduler，不改变 wire、daemon、Alacritty model、每-report
  行数、PageUp、scrollbar drag 的 33 ms 网络 pacing 或 Android semantic-cell scope。

### Acceptance criteria

- 一个同方向三-report cached burst 总位移仍精确为三行，且 16 ms 窗口内最多一次 history/
  scrollbar transaction；不出现三次背靠背 flush。
- burst 跨 cadence 边界时每个可见 frame 间隔受控且最终 offset 不丢失；反向 wheel、边界 clamp
  和 cache miss/prefetch 保持 latest-target 正确。
- child-owned三份 report 仍逐份原样/等价转发，不经 host repaint cadence；嵌套 Herdr/PiAgent
  行为不变。
- return-live、normal input、snapshot/resync、resize、reconnect、detach 和 cleanup 不会在取消后
  补画过期 viewport；DEC 2026、host capture 与单事务约束保持不变。

### Key decision and accepted trade-offs

- 用户于 2026-09-03 选择与 Herdr 相同的 event-driven、16 ms minimum presentation
  interval：dirty 状态合并为 latest complete frame，空闲时不运行固定 tick。
- 16 ms 是 desktop Zterm-owned cached viewport 的最大呈现频率，不是与显示器 vsync 对齐的
  精确 60 Hz。输入到可见结果会增加最多约一帧的等待；120/144 Hz 屏幕也不会因此获得
  120/144 个 Zterm history frame。
- 该策略不识别物理滚轮 gesture。常见的同一 stdin batch 内三份 report 必须先累计再呈现，
  但跨过 cadence boundary 或被外层终端拆成多个 batch 时仍可能显示两步；两步之间不得再是
  无界的背靠背 flush，最终 offset 必须精确。
- 快速滚动时允许跳过不可见的中间 offset，只呈现最新完整状态；不得丢失 report、反向移动或
  最终位置。未来 Android 复用 latest-frame/coalescing 原则，但应跟随 native display vsync，
  不把 desktop 的 16 ms 常量写进跨平台 core。
- trailing debounce 被否决：它更容易把三份 report 固定成一次跳转，但会让每次滚动起步都等待
  quiet window，并使连续触控板输入更黏滞。

### Authorization

- [x] 最终 planning summary 已提交；用户随后于 2026-09-03 明确回复“批准实施”。本次授权只
  覆盖上述 desktop host-owned viewport cadence，不扩大到全局 PTY scheduler 或 Android。

## Semantic Presentation and Single-Presenter Direct Cutover (Planning Reconverged 2026-09-03)

> Supersession note: R19/R20 and Phase 7 correctly deferred semantic-cell wire for that already
> completed smooth-viewport release. The user has now explicitly brought that work into this new
> R21–R27 scope; those historical deferrals must not be read as current exclusions.
> The later direct-cutover decision also supersedes every mixed-version/fallback requirement in the
> historical sections above: all nodes will be upgraded together and only the semantic presentation
> protocol remains in the product.

### Decision state and root-cause classification

- 用户确认直接完成一次完整 semantic-presentation migration，而不是先发布 extent-only 修复、
  之后再迁移架构。实现可拆为可验证、可回滚的内部阶段和 commits，但只有一个 user-visible
  completion/release boundary，不发布中间补丁版本。
- **架构/边界缺陷**：daemon-authored terminal ANSI、attachment-local history/status/gutter、capture
  mode 与 cursor restoration 没有进入同一个 desired frame，也没有共享一个只在 write + flush
  后推进的 physical baseline。此前的 gutter、status、return-live 与 nested-TUI right-margin 问题是
  同一缺失不变量的多个表现。
- **已删除的局部缺陷路径**：legacy changed-row encoder 的 inclusive `EL0` 是已确认的局部错误，
  但 direct cutover 后该 encoder 不再有产品消费者，因此删除整条 ANSI presentation path，而不是
  继续维护一个修正后的 compatibility adapter。
- Herdr、PiAgent、Ghostty 只是 regression/smoke fixtures。产品逻辑不得识别 application、process、
  title、theme、glyph 或 terminal brand。
- 项目级诊断顺序已写入
  `.trellis/spec/guides/root-cause-and-architecture-thinking-guide.md` 并加入当前任务 implement/check
  上下文：先分类 local / architecture / undetermined；确属局部违反既有契约时直接局部修复，不为
  重构而重构。
- 用户随后明确撤销 mixed-version compatibility 目标，并承诺发布后升级全部节点。本节 PRD、
  design、implement、research 与 manifests 因此再次收敛；产品代码冻结在当前部分实施状态，等待
  本次 direct-cutover summary 之后的新的明确批准。

### Goal

把当前“daemon 发送 ANSI、CLI 再追加 chrome”的 split-composition path 迁移为：daemon 发送有界、
版本化的 Zterm semantic terminal surface；client 保留完整 live/history state，先完成显式 region
layout 与 composition，再由唯一 desktop presenter 从 last successfully committed frame 原子过渡到
next frame。该边界必须让未知 nested TUI 自然正确，并让下一阶段 Android 无需解析 desktop ANSI 或
链接 Alacritty。

### R21. Exact renderer-neutral semantic surface

- `zterm-core` 新增 full-width `TerminalSurfaceRow`、`TerminalSurface`、revision-bound snapshot、
  full-row delta patch 与 semantic history-window result；复用现有 cell/style/color/cursor/modes/size/
  screen/anchor/cache domain types，不暴露 Alacritty type。
- 每个 surface 必须有恰好 `rows` 个 row、每个 row 恰好 `columns` 个 cell，并保留 wrapped、wide
  head/continuation、bounded cell text、style、cursor、child input modes 与 main scroll metrics。
- delta 只接受 exact revision + size + active-screen baseline；row index 唯一、递增且有界。size、
  screen、format 或 baseline 不兼容时返回完整 semantic snapshot，不创造 terminal-command
  pseudo-protocol。
- validator 在任何 backend 输出 text 前拒绝 control/ESC、超过现有 22-byte cell cap、orphan/非法
  wide pair、越界 cursor、错误 row shape、非法 metrics/revision 与超过 8 MiB 的 content frame；
  Debug 必须 content-redacted。
- semantic snapshot 不重放 `recent_history_ansi`。现有 bounded client-owned history window/cache
  是 desktop/mobile 共同的 scroll truth，不依赖 outer terminal physical scrollback。

### R22. Mandatory semantic wire-major cutover

- 将 product wire major 与 normal/pair ALPN 一次性升级到下一 major；旧 binary 必须在 local
  readiness 或 authenticated handshake 阶段明确失败，不能进入 attachment 后再静默降级。
- 将 protobuf source/package 与 Rust generated module 从 `proto/zterm/v1` / `zterm.v1` / `v1`
  直接迁移到 `proto/zterm/v2` / `zterm.v2` / `v2`；删除产品中的 v1 generated module，不并行
  编译两代 schema。`PairTicketV1`、持久 route cache 等独立数据格式若 wire shape 未变可保留其
  自身版本名，不能因此保留 wire-v1 transport。
- terminal attach 不再携带 presentation preference；semantic snapshot/delta/history-window 是唯一
  表示，因此删除 capability bit 21、encoding enum、epoch family negotiation 与 cross-family state。
- 在新 wire major 内让 semantic snapshot/delta 使用 terminal content 的主 kind 301/302，semantic
  history-window response 使用 318；317 request 保持 renderer-neutral。旧 ANSI 301/302/318 payload、
  312/313 pager、315/316 stateful viewport 与相应 protobuf/domain/validator/allowlist 全部删除，不复用
  为 fallback。
- local 与 remote attachment 只接受 semantic kinds；remote bridge 可以结构化解码 terminal cells，
  用于验证 shape/content bounds、revision、correlation 与 request identity，改写私有 attachment ID
  后重新编码转发；它不得解释应用内容、转换表示、合成 UI、构造 ANSI 或执行 presentation。
- trusted-device/store/session 数据不迁移；升级全部 binary 后现有身份与 Session 生命周期继续使用。
  发布前 rollback 只能回滚整次 wire-major release，不能通过重新开启 legacy adapter 完成。

### R23. One model and semantic attachment boundary

- `ProjectedScreen` 与 checkpoint 继续由 `zterm-terminal` 私有持有；每个 Session 仍只有一个 PTY、
  一个 root child、一个 Alacritty model。`portable-pty`、ordered no-drop drain、reply ordering、
  controller/takeover 与 detach/reconnect lifecycle 不变。
- attachment 在 initial snapshot/resume delta、latest update、final drain、sync-required 与 history
  response 的完整生命周期中只使用 semantic values，不再存储或分派 presentation encoding。
- model 必须直接从同一 projection 产出 surface/row patch/history rows；删除 ANSI full/delta/history
  encoder、ANSI snapshot/delta domain values 与 CLI ANSI/VT compatibility renderer。
- existing checkpoint 是 semantic baseline，因此 exact revision reconnect 可以直接 resume；不再存在
  encoding change 或 cross-family replacement 分支。

### R24. Attachment surface, cache, and explicit composition

- CLI semantic path 新增一个 `AttachmentSurface`：只安装完整 snapshot、只应用 contiguous full-row
  patch；gap 保留 last complete presentation 并请求 full sync，不能先修改 visible/committed state。
- 使用唯一 `ViewportCache<TerminalSurfaceRow>`。same-epoch anchoring、one-in-flight/latest-wins、16 ms desktop
  presentation cadence、33 ms drag request pacing 与 one-report/one-line input ownership不变。
- history pinned 时，后台 live surface 可以继续前进，但 visible source 仍是 complete cached main
  slice、cursor hidden；return-live 只把最新 complete surface 合成并提交一次。
- `ChromeLayout` 在 composition 前分配不重叠 regions：Main = child + optional one-column gutter；
  Alternate = full child width/no gutter；remote status 独占 final physical row。region transfer 不通过
  repaint ordering、glyph 或 application detection 修复。
- compositor 以 complete visible terminal/history rows、status、gutter、cursor 与 host-mode policy
  生成一个 renderer-neutral `ComposedFrame`。使用 bounded sparse absolute rows，避免按任意
  `u16 physical_rows * physical_columns` 分配；overlap/out-of-bounds 是明确 internal error。

### R25. Sole semantic desktop presenter

- active semantic attachment 期间，只有一个 presenter 可以写 terminal cells、status、gutter、
  cursor、outer keyboard/paste/focus modes、host mouse capture 与 DEC 2026 transaction。生命周期
  guard 只负责进入 UI 前的 alternate/raw setup 与退出后的 unconditional restore。
- incremental presentation 比较 previous committed 与 next composed frame；changed set 必须扩张到
  old/new wide span，按 absolute `CUP` + style runs 输出，并用 literal default blanks 删除旧 cells。
  incremental path 禁止 `EL0`/`EL2` row-tail shortcut，rightmost cell 后也不依赖 pending-wrap cursor。
- child mouse/alternate-scroll modes 只决定 input router；physical host 始终由 Zterm 以 SGR any-motion
  capture，禁止把 child mouse mode 当作第二套 outer mode owner。其他必要 keyboard/paste/focus policy
  由 presenter 从 child semantics 统一派生。
- 每个 transition 完整构造后使用一个 DEC 2026 begin/end、一次 `write_all`、一次 `flush`；只有两者
  都成功才推进 committed frame。write/flush failure 将 baseline 标为 unknown，best-effort 结束
  DEC 2026，下一次只允许 full clear + complete repaint。
- resize、screen/layout change 或 missing baseline 走 full resync。status 与
  reconnect notice 是 compositor input；不得再用 standalone newline 或独立 chrome repaint 修改屏幕。

### R26. Remove superseded presentation paths

- 删除 core/model/driver/session/proto/local IPC/remote bridge/operations/CLI 中的 legacy ANSI
  snapshot、delta、recent-history、row encoder、family enum/variants、preference/capability 与 fallback。
- 删除只为逐代兼容存在的 312/313 history pager、315/316 stateful viewport、legacy window 318 与
  CLI pager/viewport/cache/render branches；滚轮、PageUp、拖动和 return-live 全部归一到 semantic
  history-window + one cache reducer。
- 删除不再可达的测试 helper、wire conversions、error variants、feature probes、legacy fixtures 和
  corrected-EL0 临时实现，并清理只被这些路径引用的 module、public alias、Cargo dependency/feature
  和 generated artifact。旧数字在 wire-major-2 registry 中由新的唯一 semantic payload 占用或标为
  reserved；禁止保留 deprecated shim、dual-schema module 或死分支“以防将来兼容”。
- 仅保留仍有独立当前用途的通用机制，例如 snapshot acknowledgement、sync request/required、
  input/resize/detach、controller lifecycle、history coordinate/cache primitives 和 terminal guard。

### R27. Verification and one release boundary

- core/proto/model tests覆盖 exact shape/roundtrip、malformed/untrusted payload、revision/row patch、
  Unicode/wide/wrapped/style/cursor/modes、maximum dimensions、history coordinates、redaction 与
  semantic snapshot/patch replay equivalence；必须证明当前 semantic path 未构造 ANSI。
- session/local/remote tests覆盖 initial full、resume delta、ack、gap/resync、reconnect、takeover、
  final drain、local/direct/relay 与旧 wire-major 的明确拒绝；不存在 negotiation/fallback matrix。
- compositor/presenter tests覆盖 Main/Alternate/status/gutter ownership transfer、live/history、resize、
  rightmost/wide/styled blank、cursor/modes/capture、chrome-only update、一个 transaction/write/flush、
  partial failure 后 full retry，以及 active semantic mode 没有旁路 writer。
- normative automated fixture 是无 application identity 的 nested alternate-screen TUI；Herdr 在真实
  Ghostty/macOS 中验证 entry、first/continuous/reverse wheel、resize、exit/re-entry 和 return-live。
  Linux 使用通用 right-margin/nested-TUI fixture，并单独记录 local/direct/relay evidence。
- 完成 focused tests、workspace fmt/check/Clippy/tests/docs、source policy、cargo-deny、`just check`、
  independent Trellis review 与 owning specs 更新后，才允许合并和发布。没有 interim patch release。
- 延续用户决定：不运行 throughput/latency/CPU/RSS benchmark，不作性能结论；所有产品 Rust 继续
  `unsafe_code = "forbid"`。

### Acceptance criteria

- [ ] 最新 client + 最新 local/remote daemon 的 live snapshot、delta 与 history window 全程是 semantic
  cells；CLI 不解析 daemon ANSI，model/bridge 也不为该 attachment 构造或翻译 ANSI。
- [ ] 一个 complete attachment surface 与一个 complete composed frame 分别拥有 semantic/physical
  baseline；所有 active semantic output 只能经过 sole presenter，baseline 只在 write + flush 成功后
  提交。
- [ ] generic nested TUI 的 right-edge cell 在 initial/first delta/continuous/reverse wheel、resize、
  screen switch、exit/re-entry 与 return-live 后保持正确；short rows 与旧 wide cells 被精确 blank，
  没有 application/terminal-brand 特判。
- [ ] Main gutter、Alternate child full width、remote status 与 cursor/mode/capture 在所有 ownership
  transitions 中由 layout/compositor 决定；不存在 post-child cleanup、standalone status/reconnect write
  或失败后 speculative baseline。
- [ ] wire major/ALPN cutover 在 local/direct/relay 上通过；旧 binary 在握手阶段明确失败。产品 source
  中不存在 wire-v1 generated module、presentation preference、bit21、legacy kinds/payload、family
  switch 或 downgrade。
- [ ] ANSI snapshot/delta/history encoder 与 CLI compatibility renderer 已删除；rightmost、short/empty、
  styled blank、wide/combining 的正确性只由 semantic compositor/presenter oracle 覆盖。
- [ ] macOS 与 Linux 的自动化和真实连接证据均完成，所有质量门通过，specs 与任务证据同步后才
  merge/release；Android 可以消费 core/proto semantic surface/cache，而无需 Alacritty/ANSI parser。

### Explicitly out of scope

- Android UI、font shaping/glyph atlas、IME、selection/search、pixel/inertial touch/fling、native vsync、
  device/Play Store release；这些在本 migration 后进入下一任务。
- 新增 Ratatui/其他 renderer、第二个 terminal parser/model、Alacritty renderer/tty/event loop、
  pane tree、multiplexer 或改变 one-Session/one-PTY。
- application/process/title/theme/glyph/`TERM`/terminal-brand detection，forced periodic repaint，
  debounce-based correctness，或把 child DECAWM/DEC 2026 直接透传给 outer terminal。
- Kitty graphics/keyboard、OSC 8 UI、advanced styles/palette、performance benchmark 或恢复 aggregate
  terminal-memory admission。

### Risks and deferred work

- 最大风险是 wire-major coordinated rollout、reconnect/resume 一致性、wide-cell physical invalidation、以及把当前
  large `terminal_ui.rs` 中所有 active writer 收束到 presenter。每项都有独立红测、internal rollback
  point 与 final no-bypass audit。
- Unicode width policy 在 Alacritty 与 outer emulator 间可能有差异。source wide flags 作为 domain
  truth，presenter 用 absolute positioning 隔离 cursor drift；更完整 grapheme/font parity 属于 native
  renderer 工作，不扩大本次 terminal semantics。
- direct cutover 意味着未升级节点不可连接；这是用户明确接受的部署约束。实现必须让这种失败发生
  在版本/ALPN握手边界并给出稳定诊断，而不是保留运行时 fallback 或在 terminal frame 处偶然报错。
- 不新建 speculative shared presentation crate。当前可证实的移动复用边界是 core/proto semantic
  surface、history coordinates 与 cache reducer；desktop cell compositor 留在 CLI，Android 后续按 native
  layout/rendering policy 消费相同 surface。如果实施证据表明确有跨平台稳定代码边界，必须先重新收敛
  design，而不是临时搬层。

### Planning gate

- [x] 用户已确认完整 migration、one-release boundary，并明确撤销 mixed-version compatibility。
- [x] PRD、research、technical design、implementation phases 与 task contexts 已按 direct-cutover scope
  重写并校验。
- [x] 已向用户展示本次 reconverged final planning summary；用户随后明确“批准实施”。不得把
  extent-only patch 发布为过渡版本。
