# Research: Herdr 的无闪烁滚动与滚动条渲染

- Query: Herdr 如何在终端 viewport 的滚轮滚动和滚动条拖动期间避免或缓解闪烁；哪些做法适合 Zterm 当前的远端 daemon / ANSI 架构。
- Scope: mixed（Herdr 固定提交与当前 upstream、Zterm 当前代码、任务设计与项目规范）
- Date: 2026-09-03
- Herdr baseline: `cc88b3b8e5bb9f7d9f23ed6ae85a52fd7b5b9ed6`
- Herdr upstream checked: `master` HEAD `94f6d9c0d9bb9cf9ffae99d8bbfb09e9bf2fc9e0`（2026-09-02 23:47:48Z）

## Findings

### 核心结论

Herdr 的低闪烁不是 Ratatui 自动提供的，而是 Herdr 自己实现的完整 presentation pipeline：

1. 终端内容与滚动条先画进同一个 Ratatui `Buffer`。
2. 保留上一帧，跳过完全相同的帧，正常更新只编码视觉上发生变化的 cell。
3. Herdr 自有 `BlitEncoder` 显式用 `CSI ? 2026 h` / `CSI ? 2026 l` 包住整次输出，绘制前隐藏 cursor，绘制完恢复最终 cursor，再结束同步输出并 flush。
4. 正常滚动与拖动不会先整屏 clear。只有没有任何 baseline 的首帧会发 `ED2`（`CSI 2 J`）；尺寸变化或 forced repaint 即使全量写 cell，只要仍有旧 baseline，也不会先 clear。
5. 拖动事件还经过约 30 Hz 的节流、每 pane 最多一个 in-flight 请求和一个 latest queued target，以及约 60 Hz 的全局 render cadence。慢客户端的 render slot 只有一个，满时丢弃中间帧并安排一次恢复帧，不积累视觉延迟。

因此 Herdr 的防闪烁关键是“完整目标帧 + diff + DEC 2026 原子呈现 + 合并过密更新”，而不是“在 clear 之后更快地重画”。

### 证据链：输入到 host terminal

#### 1. 鼠标输入与滚动状态

- `src/client/shell/mouse.rs:37-49` 将滚动条 row 和 thumb grab offset 映射为绝对 `offset_from_bottom`。
- `src/client/shell/mouse.rs:991-1029` 的 pane scrollbar drag 只有 target 改变且距离上次发送至少 33 ms 才发送，约为 30 Hz。
- `src/client/shell/mouse.rs:1209-1230` 在 release 时补发尚未发送的最终 target，节流不会丢掉最终落点。
- `src/client/shell/mouse.rs:52-64,97-137` 每个 pane 只保留一个 in-flight scroll RPC；期间的新 target 覆盖为 latest queued target，完成后只发送最新值。
- `src/app/api/panes.rs:170-187` 接收 `PaneScroll`，调用 terminal runtime 的绝对 offset API；`src/pane/terminal.rs:1693-1732,2815-2829` 最终将它落到 Ghostty viewport，并从 terminal core 读取 scrollbar metrics。

固定提交链接：[mouse.rs](https://github.com/herdrdev/herdr/blob/cc88b3b8e5bb9f7d9f23ed6ae85a52fd7b5b9ed6/src/client/shell/mouse.rs#L37-L137)、[drag throttle](https://github.com/herdrdev/herdr/blob/cc88b3b8e5bb9f7d9f23ed6ae85a52fd7b5b9ed6/src/client/shell/mouse.rs#L991-L1029)、[final drag target](https://github.com/herdrdev/herdr/blob/cc88b3b8e5bb9f7d9f23ed6ae85a52fd7b5b9ed6/src/client/shell/mouse.rs#L1209-L1230)。

#### 2. Render scheduling 与背压

- `src/render_signal.rs:14-26,37-56,100-107` 把多次 generic / PTY render 请求折叠进一个 pending signal 和 source set。
- `src/app/mod.rs:36` 定义 `MIN_RENDER_INTERVAL = 16 ms`；`src/app/runtime.rs:74-95,135-168` 同时限制 render / presentation cadence 并计算下一次 deadline。
- `src/server/headless.rs:533-594` 到 cadence 后才消费合并的 dirty signal；可走 retained-surface patch，否则做 full virtual render。
- `src/server/client_transport.rs:117-124,247-304` 的每客户端 render queue 只有一个 `Option<Vec<u8>>` slot；slot 满时 `try_send_render` 返回 `Full`。
- `src/server/headless/render.rs:493-529,577-596` 跳过 identical frame；只有 render 成功入队后才 commit 新 baseline。slot 满则 `defer_full_render()`，避免把未发送帧当成客户端 baseline，也避免慢客户端累计旧帧。

固定提交链接：[RenderSignal](https://github.com/herdrdev/herdr/blob/cc88b3b8e5bb9f7d9f23ed6ae85a52fd7b5b9ed6/src/render_signal.rs#L14-L107)、[16 ms cadence](https://github.com/herdrdev/herdr/blob/cc88b3b8e5bb9f7d9f23ed6ae85a52fd7b5b9ed6/src/app/runtime.rs#L74-L95)、[single render slot](https://github.com/herdrdev/herdr/blob/cc88b3b8e5bb9f7d9f23ed6ae85a52fd7b5b9ed6/src/server/client_transport.rs#L247-L304)、[enqueue/commit/defer](https://github.com/herdrdev/herdr/blob/cc88b3b8e5bb9f7d9f23ed6ae85a52fd7b5b9ed6/src/server/headless/render.rs#L493-L596)。

#### 3. Ratatui buffer 与滚动条

- `src/ui/panes.rs:341-366` 先让 terminal runtime render 到 `Frame`，随即把 pane scrollbar render 到同一个 frame，最后才进入编码/输出阶段。
- `src/ui/scrollbar.rs:136-162,186-204` 直接修改 `frame.buffer_mut()` 中的一列 cell；它不是另一次 stdout repaint。
- `src/ui/panes.rs:34-47,175-196` main screen 使用稳定的一列 gutter，避免 scrollbar 出现/消失时反复 resize；child alternate screen 活跃时不占 gutter，也不画 host scrollbar。

固定提交链接：[pane + scrollbar same frame](https://github.com/herdrdev/herdr/blob/cc88b3b8e5bb9f7d9f23ed6ae85a52fd7b5b9ed6/src/ui/panes.rs#L341-L366)、[scrollbar buffer paint](https://github.com/herdrdev/herdr/blob/cc88b3b8e5bb9f7d9f23ed6ae85a52fd7b5b9ed6/src/ui/scrollbar.rs#L136-L204)、[stable gutter / alternate screen](https://github.com/herdrdev/herdr/blob/cc88b3b8e5bb9f7d9f23ed6ae85a52fd7b5b9ed6/src/ui/panes.rs#L34-L47)。

#### 4. Diff、clear 顺序、DEC 2026 与 flush

- `src/protocol/render_ansi.rs:1-27` 明确描述 Herdr 自己的策略：首帧全量、后续 cell diff、每帧 synchronized output、绘制前隐藏 cursor、最后恢复 cursor。
- `src/protocol/render_ansi.rs:57-63,70-108` 的 `BlitEncoder` 保留 `last_frame`。`clear_before_full_redraw` 仅在 `previous_frame.is_none()` 时为真。
- `src/protocol/render_ansi.rs:651-715` 的全帧路径顺序为：DEC 2026 begin → hide cursor → full/diff cells → final cursor → DEC 2026 end → flush。`ED2` 仅在“需要 full redraw 且无旧 baseline”时发出。
- `src/protocol/render_ansi.rs:586-649` 的 retained patch 路径也使用同一 DEC 2026 transaction。
- `src/protocol/render_ansi.rs:977-1030` 正常 diff 逐 cell 比较，仅写变化内容，不做 clear-before-draw。测试 `src/protocol/render_ansi.rs:1659` 验证 diff 无 `ED2`，`1721-1748` 验证 resize / forced repaint 在已有 baseline 时也无 `ED2`。
- `src/server/render_stream.rs:14-27,67-104` 为每个 Terminal-ANSI client 保留 encoder；semantic shell client 则在 `src/client/mod.rs:218-283` 的最后 host presentation 边界使用自己的 encoder。
- `src/client/mod.rs:1317-1324` 对已编码 Terminal-ANSI frame 做一次 `write_all` + `flush`；`src/client/frame_output.rs:9-25` 甚至把 Kitty graphics 插到最终 DEC 2026 end 之前，保持同一原子呈现块。

固定提交链接：[BlitEncoder policy](https://github.com/herdrdev/herdr/blob/cc88b3b8e5bb9f7d9f23ed6ae85a52fd7b5b9ed6/src/protocol/render_ansi.rs#L57-L108)、[full/diff synchronized write](https://github.com/herdrdev/herdr/blob/cc88b3b8e5bb9f7d9f23ed6ae85a52fd7b5b9ed6/src/protocol/render_ansi.rs#L651-L715)、[changed cells](https://github.com/herdrdev/herdr/blob/cc88b3b8e5bb9f7d9f23ed6ae85a52fd7b5b9ed6/src/protocol/render_ansi.rs#L961-L1030)、[client host write](https://github.com/herdrdev/herdr/blob/cc88b3b8e5bb9f7d9f23ed6ae85a52fd7b5b9ed6/src/client/mod.rs#L218-L283)。

这里有两个不同的 DEC 2026 状态，不能混为一谈：Herdr 会读取 pane 内 child 是否处于 synchronized-output mode（`src/pane/terminal.rs:1904-1913`），但防 host 闪烁的是上述 `BlitEncoder` 在最外层 host terminal 输出时主动包的 DEC 2026。它不是 Ratatui backend 隐式添加的。

#### 5. Alternate screen

- Herdr client setup 在 `src/client/terminal_setup.rs:39-44` 调用 Ratatui 0.30 的 `ratatui::init()`，退出在 `:320-346` 调用 `ratatui::try_restore()`；即 Herdr 自己占用 host alternate screen。
- pane 内 child alternate screen 是另一层状态；Herdr 此时取消该 pane 的 host scrollbar gutter，而不是退出 Herdr 的 host alternate screen。
- Zterm 当前也在 `crates/cli/src/terminal_ui.rs:71-98,1513-1541` 用 `?1049h/l` 管理自己的 host alternate screen，并在 `:1211-1229` 对 child alternate screen 取消 gutter。因此“外层 alternate screen + 内层 child alternate screen”本身不是这里的闪烁根因。

### 与 Zterm 当前实现的差异

Zterm 已有一些正确的基础：滚动条 absolute target、每次只有一个 `viewport_pending` 请求，以及 pending 时只保留/合并 queued action（`crates/cli/src/terminal_ui.rs:2459-2467,2571-2621,2624-2653`）。内容、status、scrollbar 最终也在同一次 stdout lock 生命周期中写出并 flush。

但当前 history repaint 路径会制造肉眼可见的中间状态：

- `crates/cli/src/terminal_ui.rs:3419-3443` 每次历史帧逐行执行 `CUP + SGR reset + EL2`，即先把整行清空，再写该行内容；每次滚动都对可视区所有行重复这一过程。
- `crates/cli/src/terminal_ui.rs:3352-3382` 随后又逐行重画整列 scrollbar。
- `crates/cli/src/terminal_ui.rs:3406-3416` 最后才 flush，但 `flush` 或持有 stdout lock 并不等于外层 terminal 原子呈现：terminal 可以边接收、边显示上述 clear/CUP/write 序列。
- 当前文件没有 host presentation 用的 DEC 2026 transaction。
- `crates/cli/src/terminal_ui.rs:3479-3482` 对每个真正发出的 `RequestViewport`，会在 RPC 前先调用 `render_view_stdout`。如果现有 frame 没变，这会额外清空并重画一次旧画面；response 到达后又重画新画面。
- `crates/cli/src/terminal_ui.rs:2350-2376` 对每个 drag motion 都计算 target；虽然 in-flight/latest 机制限制了并发，但没有 Herdr 的 33 ms 输入节流。低延迟连接上 response 很快时，仍可能产生高频全屏 repaint。

这与用户观察到的“滚动和拖动都闪”一致。最直接的原因是 Zterm 的 clear-before-draw 全帧输出暴露给了 host，而不是 Alacritty terminal model 的 viewport 状态更新有问题。

### 对 Zterm 的优先建议

#### P0：先修 presentation transaction

1. 在 CLI host-output 边界把一次 history 内容、scrollbar、status、最终 cursor/mode 恢复组合成一个 byte buffer。
2. 在 buffer 最外层加入 `CSI ? 2026 h`，绘制前隐藏 cursor，全部画完后恢复最终 cursor，再发 `CSI ? 2026 l`；随后一次 `write_all` + 一次 `flush`。
3. cleanup / `TerminalGuard::restore` 也应补发 `CSI ? 2026 l`，避免 partial write 或异常退出把支持该模式的 host 留在同步状态。

这项可以安全迁移到现架构：DEC 2026 只由 Zterm CLI 写给用户实际使用的最外层 Ghostty/Kitty/其他 host terminal；不要把它放进 PTY child ingress，也不要让 daemon 的 Alacritty model 把它当 child 状态。未知该 mode 的 terminal 会忽略私有 mode，支持者则原子呈现整帧。

#### P0：停止无变化的预请求重画，避免 clear-before-content

- `RequestViewport` 发 RPC 前，若 display state 未变化，不应无条件重画旧 frame；只有首次从 live 进入 loading notice 等实际 UI 变化才需要 present。
- 不再对每行先发 `EL2`。短期可先在 DEC 2026 块内覆盖完整目标行；进一步可使用“写目标内容后清理行尾”或固定宽度 padding，避免不支持 DEC 2026 的 host 看见空白阶段。
- scrollbar 至少与内容留在同一 presentation transaction；随后可只写 thumb/track 真正变化的 cell，而不是每次重画整列。

#### P1：采用 Herdr 的拖动 pacing，但保留 Zterm 已有 coalescing

- 为 drag motion 增加约 33 ms 的发送下限，仅在 target 改变时发；release 强制补发最终 target。
- 保留现有 one-in-flight + latest queued 机制。两者解决不同问题：前者限制低 RTT 下的 repaint 频率，后者限制高 RTT 下的积压。
- 若 daemon 仍可被其他高频事件触发，再考虑约 16 ms 的 presentation cadence；不要无差别延迟实时 PTY delta。

#### P2：再评估 retained visual baseline / diff

Herdr 的完整 cell diff 值得长期借鉴，但不能直接复制进 Zterm：Herdr 的最终 presenter 拥有 `FrameData` / Ratatui cell buffer；Zterm CLI 当前收到的是 daemon-authored canonical ANSI rows。可以先做 row-level retained baseline，或将协议演进为 semantic cells / patches，再做可靠的 style-aware cell diff。对于滚动导致几乎每个同位置 cell 都变化的帧，DEC 2026 比 diff 更直接地解决闪烁，故不应为了本次修复先扩张 wire protocol。

### Follow-up: 三个 wheel reports 的视觉批次为何不稳定

2026-09-03 的实机反馈确认总位移已经稳定为三行，但三份 host SGR wheel report 仍可能被看成
一次三行跳转，也可能被看成很快的一行加两行。Zterm 的 `HostInputCodec::feed` 会从一个
4 KiB stdin chunk 解出多份 `HostInputEvent::Mouse`，主循环随后逐个调用 `navigate(..., 1)` 和
`render_view_stdout`。因此三份报告会产生三次独立 DEC-2026 transaction / `write_all` / flush；
DEC 2026 只能原子化每一帧，不能把三帧合成一帧。host 的刷新时机决定用户是否看见中间帧。

Herdr 没有可依赖的“物理滚轮动作 ID”。它仍按单个 Crossterm `MouseEvent` 处理输入，并把
配置的 `mouse_scroll_lines`（默认三行）随事件传给 pane。它解决视觉批次的层次在 renderer：

- `RenderSignal` 将重复 dirty 请求折叠为一个 pending signal 和 source set；
- `MIN_RENDER_INTERVAL = 16 ms` 同时限制 render/presentation cadence；
- 每客户端只有一个 render slot；slot 满时不排队中间帧，而是安排最新 full recovery；
- 只有成功入队的帧才成为新的 presentation baseline。

所以可借鉴的是“状态立即累计、呈现最多约 60 Hz、只画最新完整状态”，不是照搬 Herdr 的
三行步长。Zterm 必须继续保持一份完整 SGR report 等于一逻辑行，否则 Ghostty 当前的一次
三-report burst 会重新放大成九行。

对 Zterm 的最小安全方案是只给 host-owned cached viewport repaint 加 16 ms cadence：每份报告
立即更新本地 desired offset，网络 prefetch/child-owned input 仍立即处理，timer 只提交当时
最新的完整缓存切片。返回 live、snapshot/resync、resize/reconnect 和普通输入必须取消或吸收
pending repaint。不要在本轮给所有 PTY delta 引入全局 60 Hz scheduler。

该方案消除无上限的快速微帧，但由于 SGR mouse protocol 没有 gesture boundary，一个 burst
恰好跨过 16 ms 边界时仍可能形成两张节奏正常的帧。若产品要求“一次物理动作永远只见一次
三行跳转”，只能增加短 trailing debounce；这会延迟首帧，并使连续触控板滚动更黏滞。

产品决定（2026-09-03）：desktop 采用 Herdr-style 16 ms、event-driven latest-frame cadence，
接受高刷新率未充分利用、最多一帧额外等待和极少数跨边界两步显示；不采用 trailing
debounce。Android 后续复用 coalescing 原则，但由 native vsync 决定实际刷新节奏。

### Current upstream drift

[固定提交到当前 HEAD 的比较](https://github.com/herdrdev/herdr/compare/cc88b3b8e5bb9f7d9f23ed6ae85a52fd7b5b9ed6...94f6d9c0d9bb9cf9ffae99d8bbfb09e9bf2fc9e0)显示：

- `src/protocol/render_ansi.rs`、`src/ui/scrollbar.rs`、`src/client/shell/mouse.rs`、`src/render_signal.rs`、`src/app/runtime.rs`、`src/client/frame_output.rs`、`src/client/terminal_setup.rs` 未发生相关修改。
- `src/server/render_stream.rs`、`src/server/headless/render.rs`、`src/ui/panes.rs` 的变化主要是 per-client tab/shell target 与多客户端投影；没有改变上述 DEC 2026、diff、clear policy、drag throttle 或 same-buffer scrollbar 机制。

所以固定提交上的结论对 2026-09-03 当前 upstream 仍成立。

## Files Found

- `Herdr src/protocol/render_ansi.rs` — 自有 diff encoder、DEC 2026 transaction、clear policy 和最终 cursor/flush。
- `Herdr src/ui/scrollbar.rs` — scrollbar geometry、drag row mapping 和 Ratatui buffer paint。
- `Herdr src/ui/panes.rs` — terminal 与 scrollbar 的同帧组合、stable gutter、child alternate-screen policy。
- `Herdr src/client/shell/mouse.rs` — drag 33 ms throttle、release final target、one-in-flight/latest coalescing。
- `Herdr src/render_signal.rs`, `src/app/runtime.rs`, `src/server/headless.rs` — dirty 合并和 16 ms render/presentation cadence。
- `Herdr src/server/client_transport.rs`, `src/server/headless/render.rs`, `src/server/render_stream.rs` — 单 render slot、identical skip、成功发送后 commit baseline、满载恢复。
- `Herdr src/client/mod.rs`, `src/client/frame_output.rs`, `src/client/terminal_setup.rs` — 最终 host write/flush、graphics 插入同步块和 alternate-screen 生命周期。
- `crates/cli/src/terminal_ui.rs` — Zterm 当前 history clear-before-draw、scrollbar repaint、viewport request/coalescing、host alternate-screen 输出路径。

## Related Specs

- `.trellis/tasks/09-02-migrate-alacritty-terminal/prd.md` — semantic viewport / scrollbar 用户需求。
- `.trellis/tasks/09-02-migrate-alacritty-terminal/design.md` — host-vs-child mouse ownership、固定 gutter 与 viewport protocol 设计。
- `.trellis/tasks/09-02-migrate-alacritty-terminal/implement.md` — 当前已实现范围和验证要求。
- `.trellis/spec/backend/terminal-model.md` — terminal model / ANSI 投影边界与 model-owned state。
- `.trellis/spec/backend/local-daemon-ipc.md` — daemon/CLI wire、revision 与背压约束。
- `.trellis/spec/guides/cross-layer-thinking-guide.md` — input → transport → state → presentation 的跨层审查要求。
- `.trellis/spec/guides/cross-platform-thinking-guide.md` — Linux/macOS/后续 Android 的平台边界。

## Caveats / Not Found

- DEC 2026 只能在支持 synchronized output 的 host terminal 上保证原子呈现；不支持者需要依靠“不先 clear”、减少写入和合理 pacing 来降低闪烁。
- Herdr 的 source 表明它“缓解/避免中间帧暴露”，不能据此保证所有 terminal emulator、SSH 链路、IME 和 graphics 组合上绝对无闪烁。
- 本报告未做 Zterm 代码修改，也未在 Ghostty/Kitty/macOS/Linux 上做动态抓包或视频验证；建议实现后分别验证支持/不支持 DEC 2026 的 host，并人为注入慢写以确认没有逐行空白。
- “一次物理滚轮移动九行”是输入报告数量与每报告 line step 的独立问题，不由本报告的 presentation 修复解释；不要用降低渲染频率掩盖该输入语义问题。
