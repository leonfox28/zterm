# 终端协议盘点与首轮兼容性补齐

## Goal

让常用命令行应用在桌面和 Android 上正确显示文字、光标、网页链接和应用标题，并修正已确认的基础控制及查询回复缺口。以端到端可观察行为验收，不能把解析器识别等同于客户端支持。

## Background and confirmed facts

- 原始请求为协议盘点，用户于 2026-09-22 同意创建 Trellis 任务；该研究交付已经完成。
- 基线为 `main` 的 `535e2fc`，workspace `0.1.35`，固定 `alacritty_terminal = 0.26.0`。完整 OSC/CSI/DCS/APC 盘点、源码定位、客户端差异、主动限制和官方参考见 [research/protocol-support.md](research/protocol-support.md)。保留该报告作为变更前证据。
- 基线 `cargo test -p zterm-terminal --all-features` 共 54 项通过；临时模型探针确认了 REP、CSI u、链接、标题、样式与查询缺口。客户端链路经过源码检查，尚未进行本轮设备验收。
- Android 已有原生选择、系统 Copy 和剪贴板写入；host-driven OSC 52 在此前设计中明确延期。先前将它列为首轮补齐项的建议已撤回。
- 用户随后认可下面五项首轮范围，并于 2026-09-24 要求把全部现有改动带到新分支继续工作。已切换至 `feat/terminal-protocol-compatibility`；切换前只有本任务的未跟踪文档，没有其他未提交代码。

## Requirements and task map

| ID | 要求与用户可见结果 | 子任务 |
| --- | --- | --- |
| R1 | 有界 REP 正确重复字符；普通 `CSI u` 恢复光标且不影响 Kitty 键盘协议；删除线和隐藏属性在两端生效。隐藏属于显示属性，原文仍可被用户显式选择复制。 | [基础控制与文字效果](../09-24-terminal-grid-compatibility/prd.md) |
| R2 | 块状、竖线、下划线光标及其闪烁状态在两端呈现；自定义光标颜色、重连和全屏应用切换时也正确。 | [光标呈现](../09-24-terminal-cursor-presentation/prd.md) |
| R3 | OSC 8 的 HTTP/HTTPS 链接在实时内容、保留历史和重连后可打开。桌面通过外层终端的链接操作，Android 长按链接后在系统操作菜单中选择“打开链接”。普通点击、鼠标报告及现有选择复制继续遵循原交互。 | [网页链接](../09-24-terminal-web-hyperlinks/prd.md) |
| R4 | OSC 0/2 应用标题传递并显示；它与用户命名的会话名是独立字段。桌面更新外层终端标题，Android 标题栏显示“会话名 · 应用标题”；空标题回到原有标签。 | [应用标题](../09-24-terminal-application-title/prd.md) |
| R5 | OSC 21 未知字段回复符合协议；`CSI 18t` 返回权威终端模型的实际字符行列数。 | [查询回复](../09-24-terminal-query-replies/prd.md) |

## Acceptance criteria

- [x] 变更前盘点区分完整支持、仅解析、部分支持、主动拒绝和未实现，并保存官方依据、测试结果与局限。
- [x] 新分支保留全部已有工作区改动。
- [x] R1：ASCII、宽字符和组合字符的重复经过边界检查；保存/恢复及 Kitty 协议互不冲突；两端显示 strike/conceal，选择复制保持原文本语义。
- [x] R2：六种显式形状/闪烁组合和默认复位可见；隐藏光标不闪现，后台不持续重绘；重连恢复同一声明状态。
- [x] R3：带链接的可见文本、换行、宽字符、历史窗口和重连均保留目标；只有显式用户操作打开 HTTP/HTTPS；清除/覆盖内容后没有残留链接。
- [x] R4：标题单独改变也会发布；重连恢复当前标题；空标题及 RIS 正确复位；会话列表与手工名称不被应用改写。
- [x] R5：回复字节、顺序、未知字段编码以及 resize 前后的行列数有协议回归测试，不伪造像素尺寸。
- [x] 跨子任务：快照、增量、历史、DEC 2026 同步输出、主/备用屏幕和 Android 行缓存保持一致；现有输入、颜色、通知、选择复制回归通过。
- [x] 完成宿主质量门禁与 Android 构建/静态/单元检查，并记录实际设备或模拟器交互证据；无法执行的验证明确列出，不能以构建成功代替运行验收。

## Out of scope

Android 应用主动 OSC 52 写入及剪贴板读取；OSC 7、133/633 shell integration；进度、更多通知与铃声策略；图像协议；远程 `file:` 打开；字体/窗口控制；文本闪烁；其余 DA/DCS 查询；扩大鼠标、焦点或键盘协议范围。现有 Android 系统选择/复制保留。

## Planning artifacts and review

[design.md](design.md) 定义共享数据与两端呈现方案；[implement.md](implement.md) 定义串行顺序及验证。五个子任务独立验收，父任务负责整体整合。范围和具体设计均已获得用户认可（2026-09-24 回复“开始”），按子任务顺序进入实现。

Acceptance evidence and platform limits: [research/implementation-evidence.md](research/implementation-evidence.md). Implementation reviewed; user authorized commit and merge on 2026-09-24.
