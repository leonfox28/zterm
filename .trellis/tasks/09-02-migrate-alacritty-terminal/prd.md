# 迁移终端状态引擎至 `alacritty_terminal`

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
  决策历史。用户最终批准前不运行 `task.py start`，不修改产品代码。

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

## Current State

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
- font shaping、glyph atlas、GPU/CPU renderer、desktop/mobile UI、selection/search交互。
- Kitty graphics/keyboard protocol、OSC 8 hyperlink UI、advanced styles、palette/theme同步。
- protobuf semantic surface v2或 wire-major变化。
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

## Final Review Status

- Product scope：已收敛。
- Engine choice：已由用户确定。
- Performance decision：已由用户确定不测。
- PRD/design/implementation plan：已按官方Alacritty、禁止unsafe、不测性能、取消aggregate
  memory admission并保留安全caps的决定完成交叉检查；task context validation通过。
- Implementation authorization：已于 2026-09-02 获得；任务已通过 `task.py start`
  进入 `in_progress`。
