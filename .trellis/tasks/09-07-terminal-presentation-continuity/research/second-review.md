# 第二轮方案审视

日期：2026-09-07。基线仍为 `be66a16`。用户要求重新审视现有方案的缺陷、遗漏、优化和替代做法。
本轮为代码、协议及算例审查，没有修改产品实现，没有进行新的实机录屏或性能验收。

Convergence note (2026-09-08): the user accepted no additional completion-guessing delay for
unmarked output. The below-toolbar remainder layout is also resolved. Historical pending choices
and implementation alternatives below are superseded by [the converged PRD](../prd.md) and
[the design](../design.md); the original evidence and reasoning are retained as a review record.

## 结论与需要修正的表述

方向成立：保留正常远端 resize，提供本地连续过渡，支持 DEC 2026，按最终显示结果更新。
需要将重点从“每次尽可能少画字符”调整为“正确选择可展示状态、连续交接几何和输入、避免多余工作”。

- 已有差量不等于已经防闪；整帧绘制也不等于会闪。不能把 ED2 或 Canvas.drawColor 单独认定为根因。
- 新尺寸快照是终端模型正确性的证据，不是程序已完成 SIGWINCH 响应的证据。
- DEC 2026 只定义输出批次，不携带 resize 请求身份、目标尺寸或“已经完成新布局”的确认。
- “完整帧”须限定：标记正常闭合且没有触发恢复策略时，可保证不发布该批次内部状态；无标记输出只有合法、有序的状态，不能凭空获得应用帧边界。
- 超时/异常终止可能暴露未完成批次，这是保证继续前进的退化行为，不能同时承诺异常情况下完整且永不等待。
- 可以不展示每一个短暂中间状态，但最终真实清屏必须正确收敛，不能用“空白就是中间态”的启发式永久掩盖它。

## 1. 不应预先把闪烁归因于清屏或绘制面积

`crates/cli/src/terminal_ui/ansi_presenter.rs:185` 先输出 HOST_SYNC_BEGIN，再在 baseline
不可用时输出 ED2，并把正文与结束标记一起 write/flush。外层正确支持同步输出时，同一批次内的
清除与重画不必成为独立可见帧。移除 ED2 是可研究的效率/兼容改进，不是已经证实的唯一修复。

Android `TerminalView.kt:212` 在同一次 onDraw 中绘制背景和正文。Canvas 背景清除也不能
直接等同于显示了一幅空白画面。需要分辨语义中间帧、布局位移、光标/进度条变化和超出帧预算。

后续最小诊断应关联：IME/窗口几何、resize 发出、模型 resize revision、BSU/ESU、客户端应用
revision、实际提交的显示几何、输入是否就绪。使用合成内容录屏，不记录用户终端正文。

## 2. 高度缩小规则与宿主一致，但像素余量交接存在具体缺口

本地固定依赖 `alacritty_terminal-0.26.0/src/grid/resize.rs:78` 的 shrink_lines：

    scroll_rows = max(0, cursor_row + 1 - target_rows)

因此，在同宽、同字体、无并发输出的高度缩小场景，光标较高裁底部、较低最小上移，确实与宿主
引擎一致。这个规则有代码依据，不需要客户端引入第二个终端解析器或推测 Shell/TUI 程序名。

但 Android 的 `TerminalView.kt:288` 按像素平移；`gridBottomGap` 在新 viewport 被接收时
通过 `:182` 重算。旧余量和新余量不同，可以产生一次额外位移。

用当前公式执行的合成 Python 算例（2026-09-07）：

| 项目 | 数值 |
| --- | --- |
| 行高 | 17 px |
| 旧可用高度 / 行数 / 底部余量 | 1000 px / 58 / 14 px |
| 新可用高度 / 行数 / 底部余量 | 600 px / 35 / 5 px |
| 旧光标行，零基 | 39 |
| 旧帧临时平移 | -94 px |
| 宿主实际上滚 | 5 行，即 85 px |
| 旧帧平移后的光标顶部 | 569 px |
| 新尺寸帧的光标顶部 | 578 px |
| 交接额外位移 | 向下 9 px |

这是现有公式的可重复算术结果，不是实机复现报告。修正目标应包含行高、底部余量、裁剪和
平移的同一份几何；最终临时位置必须与宿主整数行变换对应。动画中不能简单逐帧取余导致阶梯抖动。

扩高并非简单反向操作：同一依赖 `:40` 的 grow_lines 会从有效历史补行，缺少历史时补底部。
客户端只有 historyMaximum 不等于持有那些历史行，缺失内容不能合成。宽度/字体变化则需独立处理。

### 2.1 后续用户提案：将余量放到终端内容区之外

用户指出，终端有效高度可以严格等于整数行高度，剩余像素由下面工具栏补齐。此方案可行，
作为布局方向记录；之前的 9 px 是现有余量处理导致的差异，不是屏幕或字体的不可避免限制。

在至少能容纳一行且未触发行数上限的常规空间中，令 H 为扣除固定 chrome 后的可用物理像素高度：

    rows = floor(H / cell_height)
    grid_height = rows * cell_height
    remainder = H - grid_height

用户进一步明确：补白应放在工具栏下面。内容区顶部固定，终端直接接固定高度的工具栏按钮区，
余量放在工具栏底部 padding；按钮区与键盘/系统导航区域的距离随余量微调，不在终端与按钮之间留空。
系统手势/导航区域自身的高度由系统决定，应用保留相应安全 inset，在它上方增加自己的补白。
键盘展开时，同一补白位于工具栏与键盘之间；沿用 inset consumption 规则，避免重复相加导航与
IME 重叠区域。补白与工具栏底部背景统一，布局计算仍在同一物理像素坐标中进行。
无需拉伸字体或改变每行间距来填满容器。当前 measureGrid 已经 floor 行数，修改重点是所有
几何消费者统一使用 grid_height，有效裁剪不能继续使用包含 remainder 的外部容器高度。
也可以在外部 View 中定义精确的内部网格矩形，不要求一定重新调整整个 View 的测量架构。

原算例现在变为旧网格 58×17=986 px、外部余量 14 px；新网格 35×17=595 px、外部余量 5 px。
旧帧仅按宿主所需的 5 行平移，即 -85 px；旧光标 y=39×17-85=578 px，新光标 y=34×17=578 px。
两端点一致。正常动画的中间帧仍按系统进度做平滑像素平移/裁剪，不能把每帧位置吸附到整行，
也不因此逐帧发送远端 resize。整数行约束用于稳定布局和交接终点。

所有尺寸在同一物理像素坐标中计算，避免分别转换 dp 后出现重复舍入。资源行数上限与极小窗口
需沿用产品限制单独处理；超过上限的大块空间不能伪装成不足一行的 rounding remainder。

## 3. ResizeApplied 与应用重绘完成必须分开

`crates/daemon/src/terminal_driver.rs:480` 将原生 PTY resize、模型 resize 和 revision
发布串行提交；`crates/terminal/src/model.rs:205` 立即形成合法新尺寸模型。子进程何时重绘
是之后的独立事件；同一 read 中也可能混有 resize 前已经排队的输出。

    旧显示 → 宿主 resize / 合法新尺寸快照 → 子程序处理 SIGWINCH → BSU / 重绘 / ESU

因此不能规定“第一张新尺寸快照就是最终 TUI 布局”，也不能规定“resize 后第一个 ESU 必然
属于新布局”。2026 没有携带尺寸 generation。不能为了跳过旧布局而丢弃已到达的 PTY 字节。

客户端仍立即执行本地避让，并按明确显示边界交接。没有程序完成信号时，必须允许后续真实布局
变化正确显示；不能无限保留旧画面等待一个也许永远不会出现的重绘。

当前 Session `next_update` 在 AwaitingSnapshot 时返回 None（`crates/daemon/src/session.rs:3735`）。
如果客户端故意扣住合法 resize snapshot 的 ACK，等下一幅重绘画面才 ACK，会形成等待环：
宿主等待 ACK 才发送后续更新，客户端等待后续更新才 ACK。协议应用/确认与展示必须明确区分。
桌面现有 ACK 与成功呈现绑定，Android 则在 native actor 安装后 ACK；不得把假设直接跨平台套用。

## 4. DEC 2026 需要可展示状态边界，而不只是暂停通知

已同意支持这个能力；以下是技术实现必须解决的事项，不是再次询问是否支持。

### 4.1 所有读画面的路径都必须有一致语义

`TerminalAttachment::sync_latest/sync_changed/latest_snapshot/history_window`
（`crates/daemon/src/terminal_driver.rs:744/:755/:777/:783`）直接读当前 model。
只延迟 revision watch，主动 sync、新 attachment、历史窗口等仍可能捕获批次中间态。

建议让一个宿主所有者管理“持续变化的模型”与“最新可展示投影”的界限。保留一份有界已完成
可见状态即可，不引入第二个解析器或逐帧队列。需明确 live cells、颜色、光标、模式、尺寸、
scroll metrics 和 revision 属于同一投影；不能把旧正文与新历史锚点拼成一个貌似有效的帧。

History API 不是只读 scrollback，也可能覆盖可变 live rows（`model.rs:322`）。不能通过复制
全部历史来草率解决；应设计只读取可靠历史部分、使用对应已发布视口或可取消的有界挂起策略。
挂起不能占住 Session actor、commit/model 锁，不能阻塞负责闭合批次的输入/超时处理。

### 4.2 输出批次不能依赖 PTY read 的分片

必须覆盖这样的一次输入：

    BSU A ... ESU A BSU B ... [B 尚未结束]

只在 ingest 返回时查看“是否仍同步中”，可能把 A 的完整状态与 B 的部分状态混在一起，或在 B
超时之前连已经完成的 A 也无法提供。反过来，ESU 一到只发送通知、之后从可变 model 抓取画面，
也可能已经读到 B。

需要在真实解析边界保存可展示状态，或提供等价的有界机制；慢客户端可以跳过完整帧，但不能读到
半帧。当前“每个非空 ingest 恰好推进一次 revision”的契约（`terminal-model.md:70`）与内部
批次边界之间需明确设计，不能让一个公开 revision 对应两种不同内容。
还需避免重复标记触发不必要的逐次整屏复制；有界内存不等于低 CPU 成本。合并/惰性捕获仍须
保留最后可展示状态，不能以减少复制为由重新引入 B 半帧。

### 4.3 超时必须真的能触发

当前 model thread 在 `ByteQueue::pop()` 等下一段字节（`terminal_driver.rs:344`）。如果程序
发了 BSU 后停止输出，仅在下一次 ingest 检查时间不会恢复显示。需要独立的可唤醒 deadline。
持续输出、重复 BSU、resize、reset、EOF/退出都不能让恢复饥饿。连续开启模式不是必须配对计数
的嵌套事务；按模式语义处理重复 set/reset，并给最长不可见时间一个明确界限。

正常解析、资源检查和 PTY 查询回复继续进行。真实断线/接管/退出通知也不能被绘制批次扣住。
能力/状态查询以及跨片、C1、组合私有模式都应由既有解析边界处理，不能在原始字节里搜索字符串。

优先在宿主完成聚合，保留现有自包含快照/差量协议。但是否完全不调整 wire 字段，须由版本、
历史和 ACK 契约验证后决定；这仍是技术未知，不能提前承诺“只改一个过滤分支”。

## 5. 输入连续性是体验目标的一部分

当前证据：

- Android `terminal.rs:594` resize 调用 navigation.disconnected、增加 input_epoch、进入同步。
- `TerminalView.kt:184` epoch 变化清除组合文字/修饰键并 restartInput；`:468` 旧连接拒绝输入。
- `AppRepository.kt:304` inputReady=false 时直接不入队；`crates/android/src/terminal.rs:957` 的 native 投影隐藏非 Active 光标。
- `TerminalScreen.kt:85` 普通同步也显示进度条，快捷键随 inputReady 禁用。
- 宿主 `session.rs:4192` 已允许曾经 Active 的相同控制者在 visual-sync 窗口写入；首连和接管仍被阻挡。
  所以“resize 必须等 RTT 后才能处理任何输入”不是未经核对即可认定的宿主限制。

建议分清三件事：连接/控制权是否有效、坐标与当前几何是否匹配、输入法本地组合状态是否仍有效。
几何变化必须阻止错误坐标，但不应未经分析就把本地尚未提交的组合文字当作断线输入清掉。
普通按键也依赖终端输入模式，不能仅因“不含鼠标坐标”就宣称全部安全直通。

需要评估同一健康 attachment 的本地 preedit 保留、输入 readiness 原因分离，以及 barrier 期间
已提交输入的处理。这是提案，不代表已经批准输入缓存/重放或删除 epoch 保护。真正断线、
Session 切换、lease loss 仍不得重放旧输入。用户最终提交的一次文字不能静默漏掉或发送两次。

## 6. 桌面移除 ED2 时，未知格子不能当作空白

当前 presenter 在 baseline 不兼容时把 before 当作空行，再 normalize 成空白参与 diff。
其正确性依赖前面的 ED2 清除了真实旧内容。若只删除 ED2：旧格子是 X、新格子为空格，算法
可能比较出“空白等于空白”而不输出，留下 X。

未知 physical baseline 应触发目标区域完整覆盖，包括空白、宽字符旧半格、废弃 chrome 和边缘。
已知且同几何时继续精确 diff。外层终端自行 reflow 的限制保留；外层不支持 2026 时也应正确收敛。

## 7. 优化方向：先合并无效工作，再考虑细粒度绘制缓存

Android `terminal.rs:709` 在每轮命令/事件后调用 project_navigation，`:967` 的 project_rows
重建行/字符串/颜色；不改正文的命令也可能走完整投影。Kotlin 最后按 vsync 合并 draw，并不能
追回前面已经发生的投影和 FFI 成本。这是源代码可见的冗余候选，尚无测量证明它是当前瓶颈。

优先候选：相同正文复用已有投影；区分元数据/光标/选区变化；只为下一个真实展示机会准备最新
所需内容。保持必要状态/epoch/lifecycle 即时传播，不把所有事件一概 debounce。只有测量仍显示
绘制瓶颈时，再评估 Android retained row/RenderNode 缓存。缓存需包括颜色/字体/选择等失效依赖。

无标记输出仍可以在现有自然刷新机会合并；不需要“每个网络包展示一遍”。额外猜测结束的固定
等待仍是待定产品取舍。不能在宿主、网络和客户端各加一段等待，再把总延迟当成一个小间隔。

## 8. 可选优化：让远端 resize 与键盘动画重叠

当前 TerminalView 在动画结束后的 preDraw 才 measureGrid/发送 resize。这样最终等待可能串行包含
键盘动画时间和远端响应时间。Android 文档说明 onStart 时终态布局可用于取得动画结束位置。

可选实验：可靠得到最终可用网格时提前提交目标尺寸，本地继续跟随动画；最终校验并仅在目标改变
时纠正。需要确认 Compose/Activity 回调实际能提供包含 chrome 的终态网格，不能猜键盘固定高度。
收益是重叠等待；代价是动画反转/目标变化可能多一次有效 resize，远端新帧也可能在动画中途到达。
首版先修交接正确性；这个方案作为后续独立候选，尚未替换“动画结束提交最终尺寸”的当前策略。

## 9. 修订后的优先级与验收

1. 用最小录屏/时间线区分空白、回跳、输入中断、真正布局变化和 CPU/GPU 卡顿。
2. 修正 Android 几何余量交接，评估普通 resize 的输入法/状态提示连续性；桌面验证已知/未知
   baseline 的正确覆盖策略，避免把去掉 ED2 当作孤立修复。
3. 实现宿主 DEC 2026 可展示状态边界，先证明读路径一致、分片不敏感、deadline/ACK 不死锁。
4. 利用现有刷新机会合并候选；按证据减少重复投影/复制。额外无标记等待和提前 resize 分别评估。

新增必要场景：非整行高度（含上述 17 px 算例）、宽度变化、A 完整/B 半帧同 read、新 attach/主动
snapshot/history 恰逢批次、BSU 后零输出、持续 BSU、实际 clear 最终停住、resize ACK 等待环、
组合中文过程中键盘高度改变、快速 A→B→A 尺寸、未知 baseline 下 X→空格、退出/接管期间未闭批次。
视觉正确性和输入正确性要分别验收；仅通过快照测试不能宣称“无感”。

## 资料

- 同步输出语义、能力查询、超时讨论：
  https://github.com/contour-terminal/vt-extensions/blob/master/synchronized-output.md
- Android 键盘动画阶段与终态布局：
  https://developer.android.com/develop/ui/views/layout/sw-keyboard
- Compose inset padding 与消费规则：
  https://developer.android.com/develop/ui/compose/system/insets-ui
- 当前固定依赖位置：
  `/Users/huyuanzhe/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/alacritty_terminal-0.26.0/src/grid/resize.rs`
  `/Users/huyuanzhe/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/vte-0.15.0/src/ansi.rs`
