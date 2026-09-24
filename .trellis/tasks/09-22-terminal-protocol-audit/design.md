# 首轮终端协议兼容性设计

## Architecture and ownership

PTY → `terminal` 现有 ingress 策略 → 固定版本 Alacritty / Zterm 状态 → 语义投影 → `core` 领域模型 → `proto` → daemon/shared client → CLI / Android。继续由 Alacritty 独占网格，不新增 VT 解析器、网格镜像或原始 PTY 透传。

- `terminal`：控制序列准入、资源上限、引擎状态和快照投影。
- `core`：文字属性、光标呈现、共享链接值与应用标题的领域定义及验证。
- `proto`：字段与有界转换；沿用 v2 和同版本组件部署约束。
- daemon/shared client：沿用 revision、snapshot/delta、history、DEC 2026 发布边界，不引入独立元数据消息队列。
- CLI：唯一 ANSI presenter 输出规范化显示序列；Android：原生渲染和显式用户操作。

关键变更点：`crates/terminal/src/{ingress,engine,projection}.rs`、`crates/core/src/terminal.rs`、`proto/zterm/v2/terminal.proto`、`crates/proto/src/lib.rs`、`crates/client/`、`crates/cli/src/terminal_ui/`、`crates/android/src/{terminal,navigation}.rs`、Android `TerminalView`/`TerminalRowRenderer`/`TerminalFrames`/`TerminalScreen`。仅在实际字段消费者需要时调整 daemon/客户端代码。

## Shared contracts

| 领域变更 | Wire 计划（现有字段不重编号） | 发布/保留方式 |
| --- | --- | --- |
| `TerminalStyle` 增加 strike、conceal | style 新字段 9、10 | cell、cursor pen、snapshot、delta、history 全链路 |
| `TerminalCursor` 增加 shape 与 blinking，独立于文字 pen | cursor 字段 5 为 presentation message，形状 enum + bool | 全量及每个语义增量携带，默认值显式构造 |
| cell 可选共享 hyperlink，含 URI 与连接身份 | cell 字段 5 为字典引用；surface/delta/history 分别增加消息级字典 | 每条消息自包含；同一目标重复显示时不重复 URI 字节 |
| surface/delta 增加有界 application title | surface 字段 9；delta 字段 12 | 标题属于当前终端状态，不进入历史行或 session name |

拟用字典字段号：surface 10、delta 13、history-window 11。实现前检查 schema 无并行占用；不为未发布的中间状态建立兼容协议。字典 0 表示无链接，合法引用从 1 开始。decode 校验引用、数量、字符串长度与总字节，重用共享值；无状态地应用每个完整消息，不能依赖曾经收到过的链接登记消息。

所有新增字段进入相等比较、行指纹、快照编码和验证；内部 checkpoint 格式版本随实际变更更新。URI/id/title 的 Debug 与日志遵循现有敏感终端文本脱敏规则。保持 8 MiB 帧限制及现有历史窗口上限；不通过扩大帧限制掩盖重复编码。

## R1: basic controls and text effects

普通无前缀、无参数 `CSI u` 单独放行至上游 restore；Kitty 的 set/push/pop/query 继续走现有精确准入规则，不能宽泛放开所有以 `u` 结尾的 CSI。

REP 接受标准 `CSI Ps b`，省略或 0 为 1，每条最多重复 4,096 次，超限整条拒绝。不能直接放开上游 REP：其循环调用 `Handler::input` 会绕过 Zterm 组合字符预算。利用现有 ingress UTF-8 分帧记录最后一个实际准入的图形字符，通过同一个字符预算入口逐个重复；不从屏幕反推字符。没有可重复字符时不产生输出。RIS 清除记录；非法/被截断的 UTF-8 清除重复记录并保留原有普通输入处理，不把旧字符误当作新输入。对控制间隔、字符集、分块输入和宽/组合字符做差分验证。达到组合字符预算后采用现有字符拒绝语义。

将上游 STRIKEOUT/HIDDEN 投影为两个 bool；SGR 9/29、8/28、0 正确设置/清除。桌面输出规范 SGR；Android 原生绘制删除线，conceal 不绘制前景字形及其装饰，但保留背景与网格位置。conceal 不修改原文，不是信息保密机制；选择、复制及光标覆盖不能重新绘制隐藏字形。

## R2: cursor presentation

从上游 `cursor_style()` 读取 block/beam/underline 和 blinking；不要把 `TerminalCursor.style`（文字 pen）误用为形状。DECSCUSR 1/2 为闪烁/常亮块状，3/4 下划线，5/6 竖线；0 恢复引擎默认常亮块状。DEC 12 的设置及模式查询与实际状态一致，DEC 25 仍独立控制可见性。

CLI 无软件光标时输出规范 DECSCUSR；自定义颜色的软件光标路径也呈现形状与闪烁，使用现有 actor 的本地定时失效机制。离开时恢复 CLI 所拥有的外层终端状态。Android 光标作为动态 overlay 绘制，不因闪烁重建整页 RenderNode；不可见、后台、detach 时取消 tick。两端本地 blink 不创建 host revision 或网络流量；不承诺跨设备毫秒级同相闪烁。

## R3: OSC 8 web hyperlinks

通过现有有界 OSC 分帧解析 `8;params;URI`。空 URI 关闭；支持 `id` 参数，保留链接身份。只接受可解析且包含有效 host 的绝对 HTTP/HTTPS URL；拒绝控制字符、`file:`、自定义 app scheme 和不合法地址。使用标准 URL 解析并输出规范目标，不拼接 shell。忽略未知参数；非法 opener 关闭当前链接，避免后续文本错误继承旧目标。普通文本继续显示。

保持现有单条 OSC 总长度 1,024 字节，外部 id 最多 128 字节；每个终端两屏合计最多 1,024 个不同链接身份，URI/id 数据合计最多 1 MiB。链接值共享，禁止按每个 cell 深拷贝 URI。资源超限时清除当前链接、记录既有有界 unsupported 事件，后续文字正常显示。

配额回收与现有 combining budget 的屏幕生命周期一致：统计当前网格（含 scrollback）、当前/保存 cursor template 及另一个屏幕的保留用量；切换前后、reset、必要时配额压力下重算活动屏幕。未公开的 inactive grid 不绕过上游封装读取，允许在下次激活前保守计数；不能永久累加 session 曾经出现过的全部 URI。以 `(id, URI)` 值身份计数，并保证自动身份有界。利用 Alacritty 自带 cell hyperlink 完成滚动、清除、宽字符和保存光标语义；投影至领域共享链接值。

Wire 按消息去重；Android 缓存按共享分配计入保留字节（同页去重，跨页可保守重复计算），仍遵守现有 16 MiB 页面预算及选择预留。历史裁剪、resize、重连后不得残留旧目标；带链接的 cursor template 也参与配额，不只统计可见 cell。

CLI presenter 给文字 run 加规范 OSC 8 开/关，在 run/row/界面 chrome 边界关闭，退出时也清理，防止链接扩散到会话外。打开行为由外层终端原有链接手势负责，其支持程度是桌面端的运行环境条件。

Android 长按链接走现有选择与系统 ActionMode，增加“打开链接”；仅对该次长按命中的有效链接显示操作，后续选择跨越多个目标时不猜测。用用户实际看到的 immutable frame source、坐标和来源身份查找 URI；内容替换、source 失效或重连时取消旧动作。点击操作再校验 scheme 后通过 `ACTION_VIEW` 打开系统浏览器；无接收应用时给出轻量失败反馈。普通 tap/TUI mouse 不被链接劫持，Copy 保持现有行为，不自动读取目标或写剪贴板。

## R4: application title

OSC 0/2 写入 terminal 所有的 application title；OSC 0 本轮仅实现 title 部分，不增加 icon 行为。标题长度保持 256 UTF-8 字节，按字符边界截断；去除控制字符后才进入领域模型。空值、RIS 恢复无应用标题，主/备用屏幕切换不清空。保留已有诊断 side event 如有消费者，但客户端不依赖它恢复状态。

标题变更属于 revisioned surface/delta 元数据；title-only 输出会发布，DEC 2026 hold 期间不泄漏尚未发布的标题，重连 snapshot 恢复当前已发布值。

CLI 首次接管时使用外层终端 title stack 保存标题，输出规范 OSC 2 更新；清空时使用客户端默认 `zterm` 标签，退出/错误清理时 pop。不查询或记录外层原始标题；不支持 title stack 的外层终端无法保证恢复原标签，应在运行验证中如实记录该限制。此栈是客户端管理自身外层显示，不意味着放开子应用任意窗口操作。

Android 现有会话页主标题显示“会话名 · 应用标题”，重复或空的应用标题省略，单行省略号处理，保留原有高度和连接状态副标题。会话列表、手工重命名及持久化 session name 均继续使用原字段；不增加标题编辑、通知或后台列表更新功能。

## R5: query replies

OSC 21 未知字段回复为 `unknown=<字段名的无填充 Base64>`，按请求字段顺序回复；已知字段行为保持原合同，修改旧测试与 `terminal-colors.md` 中的旧 `=?` 合同。遵守现有每次 update 回复字节预算。

仅新增无多余参数的 `CSI 18t`，返回 `CSI 8;rows;columns t`，从 `engine.size()` 获取当时权威字符尺寸。resize 后使用新尺寸；不采信应用请求 resize 的未执行 side event，不发像素、屏幕尺寸或外层终端猜测值。

## Compatibility, evidence and rollback

沿用同版本 CLI/host/Android 配套更新策略，所有 wire/model/生成绑定变更随同一功能交付，不提供混合版本迁移或双协议旁路。生成 Kotlin 使用现有构建脚本，不能手改生成物。回退时整组回退相应语义字段、生产者、消费者及测试；不要只回退一端。无数据库迁移。

基础依据：[原始研究](research/protocol-support.md)、[xterm controls](https://invisible-island.net/xterm/ctlseqs/ctlseqs.html)、[Kitty color queries](https://sw.kovidgoyal.net/kitty/color-stack/#querying-current-color-values)、[OSC 8](https://iterm2.com/documentation-escape-codes.html#anchor-osc-8)、[Android browser intent](https://developer.android.com/guide/components/intents-common#Browser)。设备验收、外层终端对链接/标题栈的支持是运行验证项目，不能从解析器测试推断完成。
