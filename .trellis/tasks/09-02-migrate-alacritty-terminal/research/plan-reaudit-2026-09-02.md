# libghostty-vt 迁移计划重新审计（2026-09-02）

## 结论

原计划的主方向仍成立，但原版本**不应直接进入结构迁移**。重新按当前仓库契约、精确
上游源码、社区 wrapper 最新状态和 Herdr 实现逐项核验后，必须先增加一个独立的
Qualification Gate（Gate A）。Gate A 通过前，不新增正式 workspace 依赖、不切换
`TerminalModel`，也不删除 `vt100`。

保持不变的决定：

- 宿主端使用 `libghostty-vt` 作为唯一权威 VT 状态机；
- Zterm 自有 crate 继续继承 `unsafe_code = "forbid"`；
- 只从 host-only `zterm-terminal` crate 依赖安全 wrapper，core/proto 保持纯 Rust；
- 一个 `Session` 对应一个 PTY，workspace/tab/pane 布局由未来 UI 组织；
- `portable-pty` 继续拥有 child、PTY、resize、wait/close；
- wire 继续是 Zterm-owned snapshot/delta/history/modes，不暴露 Ghostty handle 或私有
  snapshot。

需要纠正的关键设计：

1. `5988a0b...` 只能作为已验证的**候选探针 revision**，不能在当前状态直接批准为
   production cutover revision；
2. Ghostty 是 parser/state authority，但 Ghostty 通用 VT formatter 不能直接成为
   Zterm wire codec；
3. terminal actor 不能同步写可能阻塞的 PTY writer；
4. history epoch、资源 reservation、TERM profile 和 offline source bundle 必须从模糊
   约束改成可执行契约。

## 审计基线

| 对象 | 精确输入 |
| --- | --- |
| Zterm workspace | 2026-09-02 当前工作树；`vt100 = 0.16.2`；workspace `unsafe_code = "forbid"` |
| 社区 wrapper | `Uzaaft/libghostty-rs@5988a0b78b4aa804d1c12e66bbfe662bd97d81c0` |
| wrapper 内嵌 Ghostty | `ghostty@22d13172cde98a0a4dda05d3d6a3fcb0dd8ed018` |
| Zig | `0.16.0` |
| Herdr | `herdrdev/herdr@cc88b3b8e5bb9f7d9f23ed6ae85a52fd7b5b9ed6`（0.8.2） |

本次重新阅读了 Zterm terminal model、driver、PTY lifecycle、CLI screen-selector/mouse
virtualization、安全 corpus、resource admission 和 shell launch 环境；同时复核 wrapper
的 render/grid/selection/callback/build APIs，以及精确 Ghostty pin 的 formatter 实现。

## 发现 1：候选 wrapper 尚未满足 cutover soundness 门

候选 revision `5988a0b...` 合并了 clipboard callback 路径的两处可达 soundness 修复，
包括 nullable zero-length buffer 与二进制 clipboard data 的处理。这说明选择 Git HEAD
而不是旧 crates.io 0.2.1 是必要的。

但截至 2026-09-02，wrapper 仍有未关闭的
[`RenderStateRowCells::graphemes_buf` soundness issue #70](https://github.com/Uzaaft/libghostty-rs/issues/70)：
一个 safe `pub fn` 只把 buffer pointer 传给 C API，没有传容量；safe caller 可以传过小
slice，C 侧无法阻止越界写。`graphemes()` 又建立在这条路径上。

`!Send + !Sync` 排除了 issue 最初讨论中的跨线程 TOCTOU 场景，但不修复“safe caller
传过小 slice”这一基本问题。因此：

- Gate A 可以继续用 `5988a0b...` 做受控 probe；
- 最终 cutover pin 必须是修复 #70 的精确上游 successor revision，且重新固定其 Ghostty/
  Zig tuple；
- 如果上游没有修复，停止在 Gate A 并重新评审，不在 Zterm 产品代码内调用 sys API、
  写 unsafe workaround 或以“我们暂时不调用”为理由把已知 unsound safe surface 当作
  已通过；
- Gate A 还要审计实际使用的 `graphemes_utf8`/grid-ref、callback、nullable string、
  allocator 和 drop 路径，而不是只搜索 Zterm 源码中的 `unsafe`。

官方 C header 本身也明确标记 API incomplete/WIP、未来会发生 breaking changes；精确
静态 pin 解决的是漂移与分发，不等于解决 soundness 或语义适配。

## 发现 2：Ghostty 通用 formatter 不符合 Zterm ANSI wire 边界

原计划把 changed row 设计成 `CUP + reset/clear-row + Ghostty-formatted row`，并把
Ghostty formatter 视为权威 ANSI codec。精确源码证明这个假设不成立：

- `TerminalFormatter` 的默认 style extras 会发出 256 项 OSC 4 palette；
- `.modes` 会发出包括 main/alternate screen 在内的通用 terminal modes，而 Zterm CLI
  只允许一个由 Zterm 定义的 screen-selector metadata prefix，并拒绝 nested selector；
- full extras 还可能发出 OSC 7 PWD、tab stops、scrolling region、keyboard state、
  protection、charset 和 cursor hyperlink URI；
- page content 的 VT formatter 当前不发出 cell OSC 8 hyperlink，只在 HTML 路径保留
  hyperlink，因此原计划声称的 hyperlink fidelity 实际无法实现；
- formatter 不负责把任意接收端先归一到 Zterm 可证明的 baseline。

这既是兼容问题，也是安全问题：Zterm 现有 corpus 要求 OSC 52 和 unknown OSC/DCS/APC
sentinel 不出现在 state、event、reply、snapshot、delta、log 或 Debug。不能把更宽的
通用 formatter 输出直接写到外层 Ghostty/kitty/其他终端，再寄希望于客户端过滤。

修订后的最小机制是：

```text
PTY bytes
   -> Ghostty terminal（唯一 parser/state authority）
   -> actor-owned Zterm SurfaceProjection（可见 cells/style/cursor/modes）
   -> Zterm allowlisted AnsiSurfaceEncoder（wire snapshot/delta/history）
   -> CLI/未来客户端的外层 terminal parser
```

`AnsiSurfaceEncoder` 不是第二个 parser。它只编码已投影的可信 DTO，允许的输出 vocabulary
固定为：

- printable UTF-8 grapheme；
- 与当前 `TerminalStyle` 一致的 SGR subset（default/indexed/RGB、bold/dim/italic/
  boolean underline/inverse）；
- `CUP`、`EL 2`、full clear/home、cursor visibility；
- Zterm 自己的单一 screen metadata selector；
- 经明确表列的 input-mode transition constants；不能透传 formatter 的 arbitrary modes。

编码器不输出 OSC、DCS、APC、palette mutation、PWD、hyperlink URI、graphics、clipboard、
protection、charset 或当前 DTO 无法表示的 style。OSC 8、underline variants、strike、
overline、blink/invisible 等能力扩展另行版本化；本迁移只保持当前 Zterm 语义，不把新引擎
能力伪装成已有 wire。

可见 viewport 使用唯一 actor-owned `RenderState` 更新一份 canonical projection；不能
把 Ghostty dirty rows 当作 per-attachment baseline，因为 render update 会消费 terminal/
screen dirty state，而每个 attachment 的确认水位不同。attachment checkpoint 比较自己
确认过的 Zterm rows/fingerprints 与最新 canonical projection。

history 使用 bounded `Point::History`/`GridRef` 读取逻辑窗口，再走同一 row encoder。
selection formatter 可以作为上游对照 probe，但不进入 correctness 或 wire path。

## 发现 3：terminal actor 必须与可能阻塞的 PTY writer 解耦

当前 `PtyIo::write_input` 是同步 `write_all + flush`。原计划让 Ghostty callback 先累积
reply，再由 terminal owner thread 同步写同一 `PtyIo`。如果 child 暂停读取 PTY，reply
写入可能阻塞，随之阻塞 VT ingest、snapshot/history/control，甚至破坏持续 drain 的
核心保证。

Herdr 的 `PtyIoActor` 提供了值得借鉴的分层：PTY readiness、user input、terminal
responses 和 resize 在独立 IO owner 中排序，terminal processing 不直接执行阻塞 write。
Herdr 自己的部分 response/pending 容器并不满足 Zterm 的严格上限，因此只借鉴所有权
分离，不复制容量策略。

Zterm 修订为两个 session-local owner：

```text
TerminalStateActor                         PtyWriterActor
  owns all !Send Ghostty handles             owns PTY writer
  ingests ordered PTY read chunks             serializes user input + replies
  produces bounded replies   --bounded-->      readiness/write/flush
  serves projection/history                   reports write failure asynchronously

Pty resize/control 与 writer actor 串行；child interrupt/wait owner 始终独立可用。
```

callback 只复制到本次 ingest 的有界 accumulator。terminal actor 用 bounded try-enqueue
把 replies 交给 writer，绝不等待 kernel write/flush 完成；mailbox full、
reply bytes 超限或 writer failure 均 fail closed，并唤醒所有等待方。user input 和 automatic
reply 必须在一个明确的 ordered write domain 中，测试 query reply 与并发 user input 的
因果顺序。child interrupt/wait 不依赖 writer 健康，因此 non-reading child 仍可关闭。

resize 需要一个书面 transaction：验证尺寸后，由 PTY IO/control owner 应用 native resize
并回执，再由 terminal actor 应用 Ghostty resize和发布 revision；任一步意外失败都进入
terminal-fatal，不能继续提供尺寸分叉的状态。不得让 terminal actor 在持有 Ghostty
borrow/callback 时等待 writer。

## 发现 4：history identity 可以用 tracked reference 做成可证明契约

原计划只写“oldest identity 淘汰时推进 epoch”，但没有定义如何检测。Ghostty line/byte
limit 又按 page 粒度近似回收，内部实际 rows 可能超过 Zterm 对外逻辑窗口。

修订方案在 terminal actor 内持有一个 owner-local `TrackedGridRef`，锚定当前逻辑 oldest
main-screen row：

1. 每次 mutation 后计算 `logical_oldest = actual_history_rows - logical_total`；
2. 把 anchor 转回 `PointSpace::History`；
3. anchor 无效、坐标不等于新的 logical oldest、resize/reflow 或 screen identity 无法证明
   连续时，推进 `history_epoch` 并重建 anchor；
4. public cursor 继续以逻辑窗口 oldest=0 计数，不暴露 Ghostty page 尾差。

这样既能检测 Ghostty 真正 prune，也能检测 Zterm logical clamp 窗口前移。tracked handle
只留在 owner thread，不进入 checkpoint/wire。Gate A 必须用写满、line/byte cap、reflow、
alternate screen 和连续分页 fixture 验证这一假设。

## 发现 5：resource projection 需要 reservation 公式，不是经验因子

当前 admission 依赖非分配的 checked projection。用 `size_of::<vt100::Cell>()` 替换成一个
“Ghostty 保守因子”仍不可审计。新的 reservation 至少分别计入：

- main + alternate visible viewport 的 checked budget；
- 显式 Ghostty scrollback byte cap；
- 精确 pin 文档/实测得到的一个 page-granularity slack；
- actor-owned canonical projection 与 encoder scratch 上限；
- controller + pending takeover 的 per-attachment checkpoint 上限；
- PTY byte、terminal control、writer、reply/event mailboxes 的 byte/capacity 上限。

配置的 engine reservation 与 Foundation 实测 RSS 是两个门：前者决定 admission，后者
验证公式没有系统性低估。Gate A 先测出公式参数，后续实现再考虑把当前
`estimated_cell_storage_bytes` 改名为真实含义的 `reserved_engine_bytes`。

## 发现 6：构建离线、TERM 与 mobile 描述需要收紧

### Reproducible/offline build

`cargo --offline` 只有在 Git dependency 已进入 Cargo cache 时才成立，cache 不是 release
source authority。受控准备至少包含 `cargo fetch --locked` 与 checksum/revision 验证；
正式 release source bundle 应 `cargo vendor`（含 Git dependency）或携带精确 wrapper
checkout，同时携带/校验 Ghostty source 与 Zig system package inputs。四个 native target
必须在禁网条件下从 bundle 重建。

### Child capability identity

当前 shell launch 只显式设置 HOME/SHELL，`TERM` 会从 daemon 环境继承；用户从 Ghostty
运行时可能把 `xterm-ghostty` 泄漏给 child。MVP 固定一个
`TerminalCapabilityProfile`：

```text
TERM=xterm-256color
COLORTERM=truecolor
DA/DSR/CPR = Zterm 当前声明的精确 replies
palette/default-color/theme query = silent ignore，保持当前行为且不询问外层 terminal
```

长期可评审 `zterm-256color` terminfo，但不阻塞本次迁移。Ghostty、kitty、tmux 或 SSH
外层身份不能改变 child profile。

### Mobile boundary

core/proto 不依赖 Ghostty，只说明移动端无需链接 C/Zig host engine；当前 wire 仍是 ANSI，
因此 native mobile UI 若要直接渲染，仍需要一个客户端 ANSI parser/widget。只有未来
semantic surface capability 才能让移动端完全不再解析 ANSI。计划不得把这两件事混为
一谈。

## Gate A：进入结构迁移前的资格门

Gate A 是第一个独立、可失败而不污染产品架构的交付物：

- [ ] 选定修复 #70 的精确 wrapper revision；记录 wrapper/Ghostty/Zig tuple 和差异审计；
- [ ] 对所有实际使用的 safe APIs 做 source review 与 create/use/drop/thread-affinity probe；
- [ ] 用 RenderState 构造 canonical Zterm projection，禁止调用已知 unsound API；
- [ ] 用 allowlisted encoder 完成 full/row-patch/history round-trip corpus；输出扫描证明没有
      OSC/DCS/APC、nested screen selector 或未表列 mode；
- [ ] 证明 OSC 8 URI、OSC 52、unknown payload 和 PWD 不进入任何输出面；
- [ ] 验证 bounded callback reply -> PTY writer handoff 在 non-reading child/query flood 下
      不阻塞 terminal actor，且 failure 可终止/唤醒；
- [ ] 用 tracked oldest anchor 验证 logical history epoch；
- [ ] 定出 line+byte cap、page slack、projection/checkpoint/mailbox reservation 公式并跑 RSS；
- [ ] 从审计过的 source bundle 在 macOS arm64/x86_64、glibc Linux arm64/x86_64 与 Windows
      hosted compile 边界验证 exact Zig/static/offline build；
- [ ] 固定 `TERM=xterm-256color`/`COLORTERM=truecolor` 与 query policy fixture。

任一项失败时，Gate A 的结果是“停止并重新评审 wrapper/codec”，不是允许 raw FFI、
`unsafe impl Send`、floating revision、runtime fallback 或放宽安全 corpus。

## 最终架构评价

在 Gate A 通过的前提下，libghostty-vt 仍优于继续扩展 vt100：它提供更完整的现代终端
状态、reflow/scrollback/modes/selection，并已有 Herdr 这类后台 multiplexer 实证。它也
优于直接嵌入 Ghostty/kitty 应用：Zterm 只需要 terminal state engine，不需要窗口、GPU、
字体 shaping 或 pane UI。

用户从 Ghostty 或其他终端运行 Zterm 没有“同一 libghostty 对象嵌套”问题。实际是两个
进程/层级中相互独立的 parser：daemon 解析 child PTY output，外层 terminal 解析 Zterm
投影 ANSI。真正需要控制的是中间 codec、mode virtualization 和 TERM identity；本次
修订正是把这些从隐含假设变成明确边界。

## 主要上游证据

- [Ghostty exact C header（WIP/API unstable）](https://github.com/ghostty-org/ghostty/blob/22d13172cde98a0a4dda05d3d6a3fcb0dd8ed018/include/ghostty/vt.h)
- [Ghostty exact formatter implementation](https://github.com/ghostty-org/ghostty/blob/22d13172cde98a0a4dda05d3d6a3fcb0dd8ed018/src/terminal/formatter.zig)
- [libghostty-rs candidate revision](https://github.com/Uzaaft/libghostty-rs/commit/5988a0b78b4aa804d1c12e66bbfe662bd97d81c0)
- [open wrapper soundness issue #70](https://github.com/Uzaaft/libghostty-rs/issues/70)
- [safe wrapper thread-affinity documentation](https://github.com/Uzaaft/libghostty-rs/blob/5988a0b78b4aa804d1c12e66bbfe662bd97d81c0/crates/libghostty-vt/src/lib.rs)
- [Herdr PTY actor](https://github.com/herdrdev/herdr/blob/cc88b3b8e5bb9f7d9f23ed6ae85a52fd7b5b9ed6/src/pty/actor/unix.rs)
- [Herdr terminal projection](https://github.com/herdrdev/herdr/blob/cc88b3b8e5bb9f7d9f23ed6ae85a52fd7b5b9ed6/src/pane/terminal.rs)
